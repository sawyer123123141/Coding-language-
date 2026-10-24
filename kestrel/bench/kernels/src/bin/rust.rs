//! The kernels in Rust, for comparison: the particle step of `step.kes` (`step`), the same with
//! `pow` per particle as `step_pow.kes` (`pow`), and the copy of `copy.kes` (`copy`); and the list
//! kernels `filter.kes` (`filter`: `iter().filter().collect()`), `map.kes` (`map`:
//! `iter().map().collect()`) and `sort_by.kes` (`sort_by`: a copy sorted with
//! `sort_by(|a, b| a.x.total_cmp(&b.x))`). One thread, release build, timed like `kernels` (median
//! ns per item).
//!
//! The interface kernels (`iface_hp.kes`, `iface_step.kes` and their `_direct` versions) are the
//! modes `dyn_hp`, `dyn_step` (a `Vec<Box<dyn Trait>>` of two types, in the generator's order) and
//! `direct_hp`, `direct_step` (a `Vec<Goblin>`).
//!
//! The value-noise kernels (`noise_scalar.kes`, `noise_corners.kes`, `noise_wide.kes`) are the
//! modes `noise_scalar` (one sample at a time) and `noise_wide` (four at once, SSE4.1), in
//! `src/noise4.rs`.
//!
//!   rust <step|pow|copy|filter|map|sort_by|dyn_hp|dyn_step|direct_hp|direct_step|noise_scalar|noise_wide> [--n N] [--iters K] [--reps R]

use std::hint::black_box;
use std::time::Instant;

#[path = "../noise4.rs"]
mod noise4;

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 720.0;
const PULL: f32 = 900.0;
const KEEP: f64 = 0.5;
const BOUNCE: f32 = 0.7;

#[derive(Clone, Copy)]
struct V2 {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
struct Particle {
    pos: V2,
    vel: V2,
}

#[inline(always)]
fn step(p: &mut Particle, dt: f32, target: V2, keep: f32) {
    let (dx, dy) = (target.x - p.pos.x, target.y - p.pos.y);
    let len = (dx * dx + dy * dy).sqrt();
    let (nx, ny) = if len > 0.0 { (dx / len, dy / len) } else { (0.0, 0.0) };
    p.vel.x = (p.vel.x + nx * PULL * dt) * keep;
    p.vel.y = (p.vel.y + ny * PULL * dt) * keep;
    p.pos.x += p.vel.x * dt;
    p.pos.y += p.vel.y * dt;
    if p.pos.x < 0.0 {
        p.pos.x = 0.0;
        p.vel.x = p.vel.x.abs() * BOUNCE;
    } else if p.pos.x > WIDTH {
        p.pos.x = WIDTH;
        p.vel.x = -p.vel.x.abs() * BOUNCE;
    }
    if p.pos.y < 0.0 {
        p.pos.y = 0.0;
        p.vel.y = p.vel.y.abs() * BOUNCE;
    } else if p.pos.y > HEIGHT {
        p.pos.y = HEIGHT;
        p.vel.y = -p.vel.y.abs() * BOUNCE;
    }
}

#[inline(never)]
fn run_step(parts: &mut [Particle], dt: f32, target: V2, keep: f32) {
    for p in parts.iter_mut() {
        step(p, dt, target, keep);
    }
}

#[inline(never)]
fn run_pow(parts: &mut [Particle], dt: f32, target: V2, _keep: f32) {
    for p in parts.iter_mut() {
        // `black_box` stops LLVM from noticing every particle gets the same value.
        let keep = black_box(KEEP).powf(dt as f64) as f32;
        step(p, dt, target, keep);
    }
}

#[inline(never)]
fn run_copy(dst: &mut [V2], parts: &[Particle]) {
    for (d, p) in dst.iter_mut().zip(parts) {
        *d = p.pos;
    }
}

/// `count` particles from a fixed seed (splitmix64).
fn spawn(count: usize) -> Vec<Particle> {
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut range = |lo: f32, hi: f32| {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        lo + (hi - lo) * (((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32)
    };
    (0..count)
        .map(|_| Particle {
            pos: V2 { x: range(0.0, WIDTH), y: range(0.0, HEIGHT) },
            vel: V2 { x: range(-200.0, 200.0), y: range(-200.0, 200.0) },
        })
        .collect()
}

/// The ZX81 generator the list kernels use for their data, as the `.kes` files write it.
fn zx81(count: usize) -> Vec<i32> {
    let mut x = 1;
    (0..count).map(|_| { x = (x * 75 + 74) % 65537; x }).collect()
}

#[derive(Clone, Copy)]
struct Item {
    x: f32,
    id: i32,
}

#[inline(never)]
fn run_filter(xs: &[i32]) -> f32 {
    let ys: Vec<i32> = xs.iter().filter(|e| **e % 2 == 0).copied().collect();
    ys.len() as f32 + ys[ys.len() - 1] as f32
}

#[inline(never)]
fn run_map(vs: &[V2]) -> f32 {
    let ls: Vec<f32> = vs.iter().map(|v| (v.x * v.x + v.y * v.y).sqrt()).collect();
    ls[0] + ls[ls.len() - 1] + ls.len() as f32
}

#[inline(never)]
fn run_sort_by(src: &[Item]) -> f32 {
    let mut work = src.to_vec();
    work.sort_by(|a, b| a.x.total_cmp(&b.x));
    let mut s = 0;
    for it in &work {
        s = (s * 31 + it.id) % 1_000_003;
    }
    s as f32
}

trait Damageable {
    fn hp(&self) -> i32;
    fn step(&mut self);
}

struct Goblin {
    health: i32,
}

struct Slime {
    size: i32,
}

impl Damageable for Goblin {
    #[inline]
    fn hp(&self) -> i32 { self.health }
    #[inline]
    fn step(&mut self) { self.health += 1; }
}

impl Damageable for Slime {
    #[inline]
    fn hp(&self) -> i32 { self.size + 1 }
    #[inline]
    fn step(&mut self) { self.size += 3; }
}

fn dyn_foes(n: usize) -> Vec<Box<dyn Damageable>> {
    zx81(n)
        .into_iter()
        .map(|x| -> Box<dyn Damageable> { if x % 2 == 0 { Box::new(Goblin { health: x % 100 }) } else { Box::new(Slime { size: x % 100 }) } })
        .collect()
}

fn direct_foes(n: usize) -> Vec<Goblin> {
    zx81(n).into_iter().map(|x| Goblin { health: x % 100 }).collect()
}

#[inline(never)]
fn run_dyn_hp(foes: &[Box<dyn Damageable>]) -> f32 {
    let mut sum = 0;
    for d in foes {
        sum += d.hp();
    }
    sum as f32
}

#[inline(never)]
fn run_dyn_step(foes: &mut [Box<dyn Damageable>]) {
    for d in foes.iter_mut() {
        d.step();
    }
}

#[inline(never)]
fn run_direct_hp(foes: &[Goblin]) -> f32 {
    let mut sum = 0;
    for d in foes {
        sum += d.hp();
    }
    sum as f32
}

#[inline(never)]
fn run_direct_step(foes: &mut [Goblin]) {
    for d in foes.iter_mut() {
        d.step();
    }
}

/// The interface kernels: the objects are made once per repetition, then `iters` passes are timed
/// (for the `step` modes the sum of `hp()` after the passes is the checksum).
fn iface_kernel(mode: &str, n: usize, iters: usize, reps: usize) {
    let (mut per, mut sum) = (Vec::new(), 0.0f32);
    for rep in 0..=reps {
        sum = 0.0;
        let mut dyns = dyn_foes(n);
        let mut directs = direct_foes(n);
        let start = Instant::now();
        for _ in 0..iters {
            match mode {
                "dyn_hp" => sum += run_dyn_hp(black_box(&dyns)),
                "direct_hp" => sum += run_direct_hp(black_box(&directs)),
                "dyn_step" => run_dyn_step(black_box(&mut dyns)),
                _ => run_direct_step(black_box(&mut directs)),
            }
        }
        let t = start.elapsed().as_secs_f64();
        match mode {
            "dyn_step" => sum = run_dyn_hp(&dyns),
            "direct_step" => sum = run_direct_hp(&directs),
            _ => {}
        }
        // The first pass warms up.
        if rep > 0 {
            per.push(t / (n * iters) as f64 * 1e9);
        }
    }
    per.sort_by(f64::total_cmp);
    println!("rust {mode}: {:.2} ns per item (median; {:.2}-{:.2}), n {n}, iters {iters}, reps {reps}, checksum {sum}", per[per.len() / 2], per[0], per[per.len() - 1]);
}

/// The list kernels: the data is made once per repetition, then `iters` passes are timed.
fn list_kernel(mode: &str, n: usize, iters: usize, reps: usize) {
    let xs = zx81(n);
    let vs: Vec<V2> = xs.iter().map(|&x| { let a = x as f32 / 65537.0; V2 { x: a, y: 1.0 - a * 0.5 } }).collect();
    let items: Vec<Item> = xs.iter().enumerate().map(|(i, &r)| Item { x: r as f32 / 65537.0, id: i as i32 }).collect();
    let (mut per, mut sum) = (Vec::new(), 0.0f32);
    for rep in 0..=reps {
        sum = 0.0;
        let start = Instant::now();
        for _ in 0..iters {
            sum += match mode {
                "filter" => run_filter(black_box(&xs)),
                "map" => run_map(black_box(&vs)),
                _ => run_sort_by(black_box(&items)),
            };
        }
        let t = start.elapsed().as_secs_f64();
        // The first pass warms up.
        if rep > 0 {
            per.push(t / (n * iters) as f64 * 1e9);
        }
    }
    per.sort_by(f64::total_cmp);
    println!("rust {mode}: {:.2} ns per item (median; {:.2}-{:.2}), n {n}, iters {iters}, reps {reps}, checksum {sum}", per[per.len() / 2], per[0], per[per.len() - 1]);
}

/// The noise kernels: `n * iters` samples per timed call, like the Kestrel `bench(n, iters)`.
fn noise_kernel(mode: &str, n: usize, iters: usize, reps: usize) {
    let (mut per, mut sum) = (Vec::new(), 0.0f64);
    for rep in 0..=reps {
        let start = Instant::now();
        sum = match mode {
            "noise_scalar" => noise4::bench_scalar(black_box(n as i32), black_box(iters as i32)),
            _ => noise4::bench_wide(black_box(n as i32), black_box(iters as i32)),
        };
        let t = start.elapsed().as_secs_f64();
        // The first pass warms up.
        if rep > 0 {
            per.push(t / (n * iters) as f64 * 1e9);
        }
    }
    per.sort_by(f64::total_cmp);
    println!("rust {mode}: {:.2} ns per item (median; {:.2}-{:.2}), n {n}, iters {iters}, reps {reps}, checksum {} (f64 {sum})", per[per.len() / 2], per[0], per[per.len() - 1], sum as f32);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().cloned().unwrap_or_else(|| "step".into());
    let number = |name: &str, default: usize| args.iter().position(|a| a == name).map_or(default, |i| args[i + 1].parse().expect("a number"));
    let (n, iters, reps) = (number("--n", 200_000), number("--iters", 30), number("--reps", 9));
    if matches!(mode.as_str(), "filter" | "map" | "sort_by") {
        return list_kernel(&mode, n, iters, reps);
    }
    if matches!(mode.as_str(), "dyn_hp" | "dyn_step" | "direct_hp" | "direct_step") {
        return iface_kernel(&mode, n, iters, reps);
    }
    if matches!(mode.as_str(), "noise_scalar" | "noise_wide") {
        return noise_kernel(&mode, n, iters, reps);
    }
    let dt = black_box(1.0f32 / 60.0);
    let target = black_box(V2 { x: 640.0, y: 360.0 });
    let keep = black_box(KEEP).powf(dt as f64) as f32;
    let mut per = Vec::new();
    for rep in 0..=reps {
        let mut parts = spawn(n);
        let mut dst = vec![V2 { x: 0.0, y: 0.0 }; n];
        let start = Instant::now();
        for _ in 0..iters {
            match mode.as_str() {
                "step" => run_step(black_box(&mut parts), dt, target, keep),
                "pow" => run_pow(black_box(&mut parts), dt, target, keep),
                "copy" => run_copy(black_box(&mut dst), black_box(&parts)),
                other => panic!("no kernel named {other}"),
            }
        }
        let t = start.elapsed().as_secs_f64();
        black_box((&parts, &dst));
        // The first pass warms up.
        if rep > 0 {
            per.push(t / (n * iters) as f64 * 1e9);
        }
    }
    per.sort_by(f64::total_cmp);
    println!("rust {mode}: {:.2} ns per item (median; {:.2}-{:.2}), n {n}, iters {iters}, reps {reps}", per[per.len() / 2], per[0], per[per.len() - 1]);
}
