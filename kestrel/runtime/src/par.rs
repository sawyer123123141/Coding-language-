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

use std::cell::RefCell;
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
    failure: Mutex<Option<String>>,
    /// The caller's program arena, for collections with no arena of their own.
    program: usize,
    /// Held while a collection grows in an arena the threads share.
    growth: Mutex<()>,
    /// Participants still working, and the arenas of those done.
    state: Mutex<(usize, Vec<usize>)>,
    done: Condvar,
}

impl Job {
    fn fail(&self, msg: &str) {
        let mut f = self.failure.lock().unwrap_or_else(|e| e.into_inner());
        f.get_or_insert_with(|| msg.to_string());
        self.failed.store(true, Ordering::Release);
    }

    /// A participant is done, handing over the arena it allocated into.
    fn finish(&self, arena: Option<*mut Arena>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.0 -= 1;
        s.1.extend(arena.map(|a| a as usize));
        if s.0 == 0 {
            self.done.notify_all();
        }
    }
}

thread_local! {
    /// The job this thread is working on, and whether this is a pool thread.
    static CURRENT: RefCell<Option<(Arc<Job>, bool)>> = const { RefCell::new(None) };
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
    /// Asks `helpers` pool threads to join `job`, starting threads as needed.
    fn run(&self, job: &Arc<Job>, helpers: usize) {
        let _g = self.spawning.lock().unwrap_or_else(|e| e.into_inner());
        while self.live.load(Ordering::Acquire) < helpers {
            let rx = Arc::clone(&self.receiver);
            let started = std::thread::Builder::new().name("kestrel-par".into()).stack_size(STACK_SIZE).spawn(move || serve(rx));
            if started.is_err() {
                break;
            }
            self.live.fetch_add(1, Ordering::AcqRel);
        }
        let tickets = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        for _ in 0..helpers {
            let _ = tickets.send(Arc::clone(job));
        }
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
    CURRENT.with_borrow_mut(|c| *c = Some((Arc::clone(job), pool_thread)));
    let a = arena::kestrel_rt_arena_new();
    arena::kestrel_rt_arena_push(a);
    while !job.failed.load(Ordering::Acquire) {
        let start = job.next.fetch_add(job.chunk, Ordering::Relaxed);
        if start >= job.count {
            break;
        }
        (job.body)(job.env as *const u8, start, (start + job.chunk).min(job.count));
    }
    arena::kestrel_rt_arena_pop();
    CURRENT.with_borrow_mut(|c| *c = None);
    job.finish(Some(a));
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
    let threads = threads.min(count as usize);
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
    pool().run(&job, threads - 1);
    participate(&job, false);
    let mut s = job.state.lock().unwrap_or_else(|e| e.into_inner());
    while s.0 > 0 {
        s = job.done.wait(s).unwrap_or_else(|e| e.into_inner());
    }
    let arenas = std::mem::take(&mut s.1);
    drop(s);
    if job.failed.load(Ordering::Acquire) {
        let msg = job.failure.lock().unwrap_or_else(|e| e.into_inner()).take();
        return Err(msg.unwrap_or_else(|| "a `par for` iteration failed".to_string()));
    }
    let current = arena::kestrel_rt_current_arena();
    for a in arenas {
        // SAFETY: each came from `kestrel_rt_arena_new`, is no longer pushed
        // anywhere, and is handed over exactly once; `current` is live.
        let mut other = unsafe { Box::from_raw(a as *mut Arena) };
        // SAFETY: `current` is the live current arena.
        unsafe { (*current).absorb(&mut other) };
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
    let job = CURRENT.with_borrow(|c| c.as_ref().map(|(j, _)| Arc::clone(j)));
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
    CURRENT.with_borrow(|c| c.as_ref().map(|(j, _)| j.program as *mut Arena))
}

/// A runtime failure inside a job. On a pool thread it hands the message
/// to the caller and parks the thread for good, never returning. On the
/// caller it stops the other threads and returns true; the caller then
/// fails as usual. Outside a job it returns false.
pub(crate) fn fail_in_job(msg: &str) -> bool {
    let Some((job, pool_thread)) = CURRENT.with_borrow_mut(|c| c.take()) else { return false };
    job.fail(msg);
    if !pool_thread {
        return true;
    }
    pool().live.fetch_sub(1, Ordering::AcqRel);
    job.finish(None);
    drop(job);
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU8;

    use super::*;

    fn run(count: i64, body: Body, env: *const u8, threads: usize) -> Result<(), String> {
        run_job(count, body, env, threads)
    }

    extern "C" fn mark(env: *const u8, start: i64, end: i64) {
        let seen = env as *const AtomicU8;
        for i in start..end {
            // SAFETY: the test's env holds `count` counters.
            unsafe { (*seen.add(i as usize)).fetch_add(1, Ordering::Relaxed) };
        }
    }

    #[test]
    fn every_index_runs_once() {
        for threads in [1, 2, 4, 16] {
            for count in [0i64, 1, 7, 1000, 100_003] {
                let seen: Vec<AtomicU8> = (0..count).map(|_| AtomicU8::new(0)).collect();
                run(count, mark, seen.as_ptr() as *const u8, threads).unwrap();
                assert!(seen.iter().all(|s| s.load(Ordering::Relaxed) == 1), "threads {threads}, count {count}");
            }
        }
    }

    extern "C" fn nested(env: *const u8, start: i64, end: i64) {
        for _ in start..end {
            // An inner job runs in order on this thread.
            run_job(10, mark, env, 8).unwrap();
        }
    }

    #[test]
    fn nested_jobs_run_inline() {
        let seen: Vec<AtomicU8> = (0..10).map(|_| AtomicU8::new(0)).collect();
        run(25, nested, seen.as_ptr() as *const u8, 4).unwrap();
        assert!(seen.iter().all(|s| s.load(Ordering::Relaxed) == 25));
    }

    extern "C" fn allocate(env: *const u8, start: i64, end: i64) {
        let slots = env as *mut usize;
        for i in start..end {
            let p = arena::kestrel_rt_alloc(8, 8, 8, 0) as *mut i64;
            // SAFETY: `p` holds 8 i64s; the env holds a slot per index.
            unsafe {
                *p = i * 3;
                *slots.add(i as usize) = p as usize;
            }
        }
    }

    #[test]
    fn allocations_survive_the_loop() {
        let mut slots = vec![0usize; 5000];
        run(5000, allocate, slots.as_mut_ptr() as *const u8, 8).unwrap();
        // The threads' arenas are freed by now; their blocks belong to this thread's.
        let filler: Vec<*mut u8> = (0..5000).map(|_| arena::kestrel_rt_alloc(8, 8, 8, 0)).collect();
        assert!(!filler.is_empty());
        for (i, &p) in slots.iter().enumerate() {
            // SAFETY: each slot points at an i64 that should still be alive.
            assert_eq!(unsafe { *(p as *const i64) }, i as i64 * 3);
        }
    }

    extern "C" fn fail_on_pool(_env: *const u8, _start: i64, _end: i64) {
        let pool_thread = CURRENT.with_borrow(|c| c.as_ref().is_some_and(|(_, p)| *p));
        if pool_thread {
            fail_in_job("iteration failed");
        }
        // The caller waits for a pool thread to fail.
        while !CURRENT.with_borrow(|c| c.as_ref().is_some_and(|(j, _)| j.failed.load(Ordering::Acquire))) {
            std::thread::yield_now();
        }
    }

    #[test]
    fn pool_failures_come_back_to_the_caller() {
        assert_eq!(run(100, fail_on_pool, std::ptr::null(), 4), Err("iteration failed".to_string()));
        // The pool still works afterwards.
        let seen: Vec<AtomicU8> = (0..100).map(|_| AtomicU8::new(0)).collect();
        run(100, mark, seen.as_ptr() as *const u8, 4).unwrap();
        assert!(seen.iter().all(|s| s.load(Ordering::Relaxed) == 1));
    }
}
