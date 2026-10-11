//! The simulation: the particle, its step (the same arithmetic as `Particle.step` in
//! `particles.kes`), and the ways to run the step over every particle.

use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Condvar, Mutex};

use rayon::prelude::*;

pub const WIDTH: f32 = 1280.0;
pub const HEIGHT: f32 = 720.0;
/// How hard the target pulls (pixels per second, every second), how much speed a particle keeps
/// each second, and how much speed a bounce keeps.
const PULL: f32 = 900.0;
const KEEP: f64 = 0.5;
const BOUNCE: f64 = 0.7;

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct V2 {
    pub x: f32,
    pub y: f32,
}

/// Position and velocity, 16 bytes, like the demo's `Particle` (two `vec2`s).
#[derive(Clone, Copy)]
pub struct Particle {
    pub pos: V2,
    pub vel: V2,
}

/// What one frame's step needs. `keep` is `pow(KEEP, dt)` when the caller worked it out once;
/// `None` means every particle computes it, as the demo does (see `Particle::step`).
#[derive(Clone, Copy)]
pub struct Frame {
    pub dt: f32,
    pub target: V2,
    pub keep: Option<f32>,
}

impl Particle {
    /// Pulls toward `f.target`, slows a little, and bounces off the window edges.
    ///
    /// The demo calls `pow(KEEP, dt)` for every particle (a call into the runtime that gives the
    /// same value for all of them). `black_box` keeps rustc from noticing that and doing the
    /// call once, so this does the same work; `Frame::keep` is the version that hoists it.
    #[inline]
    pub fn step(&mut self, f: &Frame) {
        let (dx, dy) = (f.target.x - self.pos.x, f.target.y - self.pos.y);
        let len = (dx * dx + dy * dy).sqrt();
        // A zero vector stays zero, as `normalized()` does.
        let (nx, ny) = if len > 0.0 { (dx / len, dy / len) } else { (0.0, 0.0) };
        self.vel.x += nx * PULL * f.dt;
        self.vel.y += ny * PULL * f.dt;
        let keep = match f.keep {
            Some(keep) => keep,
            None => black_box(KEEP).powf(f.dt as f64) as f32,
        };
        self.vel.x *= keep;
        self.vel.y *= keep;
        self.pos.x += self.vel.x * f.dt;
        self.pos.y += self.vel.y * f.dt;
        if self.pos.x < 0.0 {
            self.pos.x = 0.0;
            self.vel.x = (self.vel.x.abs() as f64 * BOUNCE) as f32;
        } else if self.pos.x > WIDTH {
            self.pos.x = WIDTH;
            self.vel.x = (-self.vel.x.abs() as f64 * BOUNCE) as f32;
        }
        if self.pos.y < 0.0 {
            self.pos.y = 0.0;
            self.vel.y = (self.vel.y.abs() as f64 * BOUNCE) as f32;
        } else if self.pos.y > HEIGHT {
            self.pos.y = HEIGHT;
            self.vel.y = (-self.vel.y.abs() as f64 * BOUNCE) as f32;
        }
    }
}

/// `count` particles at random spots with random velocities, from a fixed seed (splitmix64), so
/// every run starts the same. The demo's `randf_range` calls come in the same order.
pub fn spawn(count: usize) -> Vec<Particle> {
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut range = |lo: f32, hi: f32| {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        let unit = ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32;
        lo + (hi - lo) * unit
    };
    (0..count)
        .map(|_| {
            let pos = V2 {
                x: range(0.0, WIDTH),
                y: range(0.0, HEIGHT),
            };
            let vel = V2 {
                x: range(-200.0, 200.0),
                y: range(-200.0, 200.0),
            };
            Particle { pos, vel }
        })
        .collect()
}

/// How the step is run over the particles each frame.
pub enum Update {
    /// Not at all (to time the draw with nothing running before it).
    None,
    /// A plain loop on the main thread.
    Single,
    /// rayon's `par_iter_mut` on its global pool (one thread per logical core).
    Rayon,
    /// The same on a pool of this many threads.
    RayonN(rayon::ThreadPool),
    /// `std::thread::scope` over this many chunks, starting the threads every frame.
    Scope(usize),
    /// A pool built like Kestrel's `par for` (`runtime/src/par.rs`): workers block on a channel,
    /// chunks come from an atomic counter, the caller works too, then waits on a condvar.
    Pool(Pool),
}

impl Update {
    /// `none`, `single`, `rayon`, `rayon:N`, `scope:N` or `pool:N`.
    pub fn parse(text: &str) -> Result<Update, String> {
        let (name, n) = match text.split_once(':') {
            Some((name, n)) => (
                name,
                Some(n.parse::<usize>().map_err(|_| format!("bad thread count in {text}"))?.max(1)),
            ),
            None => (text, None),
        };
        Ok(match (name, n) {
            ("none", None) => Update::None,
            ("single", None) => Update::Single,
            ("rayon", None) => Update::Rayon,
            ("rayon", Some(n)) => Update::RayonN(rayon::ThreadPoolBuilder::new().num_threads(n).build().map_err(|e| e.to_string())?),
            ("scope", Some(n)) => Update::Scope(n),
            ("pool", Some(n)) => Update::Pool(Pool::new(n)),
            _ => {
                return Err(format!(
                    "--update takes none, single, rayon, rayon:N, scope:N or pool:N, not {text}"
                ))
            }
        })
    }

    pub fn run(&self, parts: &mut [Particle], f: Frame) {
        match self {
            Update::None => {}
            Update::Single => parts.iter_mut().for_each(|p| p.step(&f)),
            Update::Rayon => parts.par_iter_mut().for_each(|p| p.step(&f)),
            Update::RayonN(pool) => pool.install(|| parts.par_iter_mut().for_each(|p| p.step(&f))),
            Update::Scope(n) => std::thread::scope(|s| {
                for chunk in parts.chunks_mut(parts.len().div_ceil(*n).max(1)) {
                    s.spawn(move || chunk.iter_mut().for_each(|p| p.step(&f)));
                }
            }),
            Update::Pool(pool) => pool.run(parts, f),
        }
    }
}

/// One job of the `Pool`: `count` particles at `parts`, handed out `chunk` at a time.
struct Job {
    /// The particles' address, as an integer so the job can cross threads (the caller keeps the
    /// slice borrowed until every participant is done).
    parts: usize,
    count: usize,
    chunk: usize,
    next: AtomicUsize,
    frame: Frame,
    /// Participants still working.
    left: Mutex<usize>,
    done: Condvar,
}

pub struct Pool {
    tickets: Sender<Arc<Job>>,
    threads: usize,
}

impl Pool {
    /// A pool of `threads` threads counting the caller: `threads - 1` workers.
    fn new(threads: usize) -> Pool {
        let (tickets, receiver) = channel::<Arc<Job>>();
        let receiver = Arc::new(Mutex::new(receiver));
        for _ in 1..threads {
            let receiver = Arc::clone(&receiver);
            std::thread::spawn(move || loop {
                // Blocks in `recv`, holding the lock; the other workers wait for the lock.
                let job = receiver.lock().unwrap().recv();
                match job {
                    Ok(job) => work(&job),
                    Err(_) => return,
                }
            });
        }
        Pool { tickets, threads }
    }

    fn run(&self, parts: &mut [Particle], frame: Frame) {
        let count = parts.len();
        let job = Arc::new(Job {
            parts: parts.as_mut_ptr() as usize,
            count,
            chunk: (count / (self.threads * 8)).max(1),
            next: AtomicUsize::new(0),
            frame,
            left: Mutex::new(self.threads),
            done: Condvar::new(),
        });
        for _ in 1..self.threads {
            self.tickets.send(Arc::clone(&job)).unwrap();
        }
        work(&job);
        let mut left = job.left.lock().unwrap();
        while *left > 0 {
            left = job.done.wait(left).unwrap();
        }
    }
}

/// Takes chunks of `job` until none are left, then reports as done.
fn work(job: &Job) {
    loop {
        let start = job.next.fetch_add(job.chunk, Ordering::Relaxed);
        if start >= job.count {
            break;
        }
        let end = (start + job.chunk).min(job.count);
        // SAFETY: the counter hands every index to one participant, and the caller keeps the
        // slice borrowed until all of them have reported done (after their last use of it).
        let chunk = unsafe { std::slice::from_raw_parts_mut((job.parts as *mut Particle).add(start), end - start) };
        chunk.iter_mut().for_each(|p| p.step(&job.frame));
    }
    let mut left = job.left.lock().unwrap();
    *left -= 1;
    if *left == 0 {
        job.done.notify_all();
    }
}
