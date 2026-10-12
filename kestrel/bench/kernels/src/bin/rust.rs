//! The kernels in Rust, for comparison: the particle step of `step.kes` (`step`), the same with
//! `pow` per particle as `step_pow.kes` (`pow`), and the copy of `copy.kes` (`copy`). One thread,
//! release build, timed like `kernels` (median ns per item).
//!
//!   rust <step|pow|copy> [--n N] [--iters K] [--reps R]

use std::hint::black_box;
use std::time::Instant;

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().cloned().unwrap_or_else(|| "step".into());
    let number = |name: &str, default: usize| args.iter().position(|a| a == name).map_or(default, |i| args[i + 1].parse().expect("a number"));
    let (n, iters, reps) = (number("--n", 200_000), number("--iters", 30), number("--reps", 9));
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
