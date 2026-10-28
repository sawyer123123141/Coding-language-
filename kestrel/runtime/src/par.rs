//! `par for`: a loop's iterations run on a pool of threads, in chunks
//! handed out through an atomic counter. Each participating thread
//! allocates into an arena of its own; when the loop is done, those
//! arenas' blocks move into the caller's current arena, so what the body
//! made lives exactly as long as it would after a plain `for`.
//!
//! A collection growing in an arena the threads share (the program arena,
//! or one from outside the loop) takes the job's lock. An iteration that
//! fails on a pool thread hands its message to the caller and parks that
//! thread for good (unwinding through compiled code isn't safe).
//!
//! The same failure is reported at any thread count: each failure is kept with the start of the
//! chunk it happened in, and the lowest start wins. After a failure no new chunk is taken, a chunk
//! already taken runs to its end, and a failing caller thread waits for the others before it
//! reports the winner. Chunks are handed out in increasing order, so every chunk below a failing
//! one was already taken.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use crate::arena::{self, Arena};

/// A compiled loop body: runs iterations `start..end` with the captured
/// variables' addresses at `env`.
pub type Body = extern "C" fn(env: *const u8, start: i64, end: i64);

/// The largest stack a pool thread gets, like the main thread's.
const STACK_SIZE: usize = 256 * 1024 * 1024;

/// One running `par for`.
struct Job {
    body: Body,
    /// The captured variables' addresses (a pointer kept as an integer, so
    /// the job can cross threads; the caller keeps it alive until the end).
    env: usize,
    count: i64,
    chunk: i64,
    next: AtomicI64,
    failed: AtomicBool,
    /// The failure with the smallest chunk start so far: (chunk start, message).
    failure: Mutex<Option<(i64, String)>>,
    /// The caller's program arena, for collections with no arena of their own.
    program: usize,
    /// Held while a collection grows in an arena the threads share.
    growth: Mutex<()>,
    /// Participants still working, and the arenas of those done.
    state: Mutex<(usize, Vec<usize>)>,
    done: Condvar,
}

impl Job {
    /// Records a failure in the chunk starting at `start`; the lowest start wins.
    fn fail(&self, start: i64, msg: &str) {
        let mut f = self.failure.lock().unwrap_or_else(|e| e.into_inner());
        if f.as_ref().is_none_or(|(best, _)| start < *best) {
            *f = Some((start, msg.to_string()));
        }
        self.failed.store(true, Ordering::Release);
    }

    /// The message of the failure with the lowest chunk start.
    fn winner(&self) -> Option<String> {
        self.failure.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(_, m)| m.clone())
    }

    /// A participant is done, handing over the arena it allocated into.
    fn finish(&self, arena: Option<*mut Arena>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.0 -= 1;
        s.1.extend(arena.map(|a| a as usize));
        // The caller waits for 0 when the job is over, or for 1 (itself) when it failed.
        if s.0 <= 1 {
            self.done.notify_all();
        }
    }
}

/// What a thread working on a job knows: the job, whether it is a pool thread, and the start of the
/// chunk it is running.
type Participation = (Arc<Job>, bool, Cell<i64>);

thread_local! {
    /// The job this thread is working on, if any.
    static CURRENT: RefCell<Option<Participation>> = const { RefCell::new(None) };
}

struct Pool {
    tickets: Mutex<Sender<Arc<Job>>>,
    receiver: Arc<Mutex<Receiver<Arc<Job>>>>,
    /// Pool threads that can still take work (failed ones park for good).
    live: AtomicUsize,
    spawning: Mutex<()>,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let (tx, rx) = channel();
        Pool { tickets: Mutex::new(tx), receiver: Arc::new(Mutex::new(rx)), live: AtomicUsize::new(0), spawning: Mutex::new(()) }
    })
}

impl Pool {
    /// Starts pool threads until `wanted` are alive, if the system allows.
    /// Returns how many helpers a job can count on.
    fn reserve(&self, wanted: usize) -> usize {
        let _g = self.spawning.lock().unwrap_or_else(|e| e.into_inner());
        while self.live.load(Ordering::Acquire) < wanted {
            let rx = Arc::clone(&self.receiver);
            let started = std::thread::Builder::new().name("kestrel-par".into()).stack_size(STACK_SIZE).spawn(move || serve(rx));
            if started.is_err() {
                break;
            }
            self.live.fetch_add(1, Ordering::AcqRel);
        }
        wanted.min(self.live.load(Ordering::Acquire))
    }

    /// Asks `helpers` pool threads to join `job`.
    fn send(&self, job: &Arc<Job>, helpers: usize) {
        let tickets = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        for _ in 0..helpers {
            let _ = tickets.send(Arc::clone(job));
        }
    }
}

/// Reports a participant as done even if it stops by panicking (a bug in
/// the runtime), so the caller never waits forever.
struct Done<'a> {
    job: &'a Job,
    arena: Option<*mut Arena>,
}

impl Drop for Done<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            // A bug in the runtime, not in the program: any real failure ranks before it.
            self.job.fail(i64::MAX, "a `par for` thread stopped unexpectedly");
        }
        self.job.finish(self.arena);
    }
}

fn serve(rx: Arc<Mutex<Receiver<Arc<Job>>>>) {
    loop {
        let job = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
        match job {
            Ok(job) => participate(&job, true),
            Err(_) => return,
        }
    }
}

/// Works on `job` until its iterations run out (or one failed).
fn participate(job: &Arc<Job>, pool_thread: bool) {
    // A pool thread's float mode is whatever its last job left. The caller's is pinned only in a
    // program started by `kestrel_rt_start`; a host (the Godot add-on) keeps its own.
    if pool_thread || crate::fpu::started() {
        crate::fpu::pin();
    }
    CURRENT.with_borrow_mut(|c| *c = Some((Arc::clone(job), pool_thread, Cell::new(0))));
    let a = arena::new_uncounted();
    let mut done = Done { job, arena: None };
    arena::kestrel_rt_arena_push(a);
    while !job.failed.load(Ordering::Acquire) {
        let start = job.next.fetch_add(job.chunk, Ordering::Relaxed);
        if start >= job.count {
            break;
        }
        CURRENT.with_borrow(|c| c.iter().for_each(|(_, _, chunk)| chunk.set(start)));
        (job.body)(job.env as *const u8, start, (start + job.chunk).min(job.count));
    }
    arena::kestrel_rt_arena_pop();
    CURRENT.with_borrow_mut(|c| *c = None);
    done.arena = Some(a);
}

/// How many threads a `par for` uses: `KESTREL_THREADS`, else one per
/// logical core.
fn threads() -> usize {
    static THREADS: OnceLock<usize> = OnceLock::new();
    *THREADS.get_or_init(|| {
        let set = std::env::var("KESTREL_THREADS").ok().and_then(|v| v.trim().parse::<usize>().ok());
        set.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get())).max(1)
    })
}

/// Runs `body` over `0..count` on `threads` threads (this one included).
/// Inside another job, or with nothing to split, it runs in order here.
pub(crate) fn run_job(count: i64, body: Body, env: *const u8, threads: usize) -> Result<(), String> {
    let inside = CURRENT.with_borrow(|c| c.is_some());
    if count <= 1 || threads <= 1 || inside {
        if count > 0 {
            body(env, 0, count);
        }
        return Ok(());
    }
    // Fewer threads than asked for when the system can't start more.
    let helpers = pool().reserve(threads.min(count as usize) - 1);
    let threads = helpers + 1;
    let job = Arc::new(Job {
        body,
        env: env as usize,
        count,
        chunk: (count / (threads as i64 * 8)).max(1),
        next: AtomicI64::new(0),
        failed: AtomicBool::new(false),
        failure: Mutex::new(None),
        program: arena::program_arena() as usize,
        growth: Mutex::new(()),
        state: Mutex::new((threads, Vec::new())),
        done: Condvar::new(),
    });
    pool().send(&job, helpers);
    participate(&job, false);
    let mut s = job.state.lock().unwrap_or_else(|e| e.into_inner());
    while s.0 > 0 {
        s = job.done.wait(s).unwrap_or_else(|e| e.into_inner());
    }
    let arenas = std::mem::take(&mut s.1);
    drop(s);
    if job.failed.load(Ordering::Acquire) {
        return Err(job.winner().unwrap_or_else(|| "a `par for` iteration failed".to_string()));
    }
    let current = arena::kestrel_rt_current_arena();
    for a in arenas {
        // SAFETY: each came from `new_uncounted`, is no longer pushed
        // anywhere, and is handed over exactly once; `current` is live.
        let child = unsafe { Box::from_raw(a as *mut Arena) };
        // SAFETY: `current` is the live current arena.
        unsafe { (*current).adopt(child) };
    }
    Ok(())
}

/// Runs a `par for`: `body` over iterations `0..count` across the pool.
/// A failed iteration fails the program here, on the caller's thread.
#[no_mangle]
pub extern "C" fn kestrel_rt_par_for(count: i64, body: Body, env: *const u8) {
    if let Err(msg) = run_job(count, body, env, threads()) {
        crate::panic::fail(&msg);
    }
}

/// Runs `f`, which grows a collection in `arena`, holding the job's lock
/// when other threads may be allocating there too.
pub(crate) fn shared_growth<R>(arena: *mut Arena, f: impl FnOnce() -> R) -> R {
    let job = CURRENT.with_borrow(|c| c.as_ref().map(|(j, _, _)| Arc::clone(j)));
    match job {
        Some(job) if arena != arena::kestrel_rt_current_arena() => {
            let _g = job.growth.lock().unwrap_or_else(|e| e.into_inner());
            f()
        }
        _ => f(),
    }
}

/// Inside a job, the caller's program arena (this thread's own would be
/// the wrong one).
pub(crate) fn job_program_arena() -> Option<*mut Arena> {
    CURRENT.with_borrow(|c| c.as_ref().map(|(j, _, _)| j.program as *mut Arena))
}

/// A runtime failure inside a job. Outside a job it returns `None`. On a pool thread it records the
/// failure for the caller and parks the thread for good, never returning. On the caller it records
/// the failure, waits until every other participant is done (chunks already taken run to their end,
/// no new one is taken), and returns the message of the failure with the lowest chunk start; the
/// caller then fails with that message.
pub(crate) fn fail_in_job(msg: &str) -> Option<String> {
    let (job, pool_thread, start) = CURRENT.with_borrow_mut(|c| c.take())?;
    job.fail(start.get(), msg);
    if pool_thread {
        pool().live.fetch_sub(1, Ordering::AcqRel);
        job.finish(None);
        drop(job);
        loop {
            std::thread::park();
        }
    }
    // The caller counts itself in `state.0`; the others are done when it is down to 1.
    let mut s = job.state.lock().unwrap_or_else(|e| e.into_inner());
    while s.0 > 1 {
        s = job.done.wait(s).unwrap_or_else(|e| e.into_inner());
    }
    drop(s);
    job.winner()
}

#[cfg(test)]
mod tests;
