//! The Rust functions alone, in nanoseconds per call: for each `float` function, `today` (what
//! 49ae9f0 runs: `libm`'s `f64` function on the widened input, narrowed once, or `libm::fmodf` for
//! the remainder) against the wide function compiled code calls (`kestrel_rt_<op>f`, or
//! `kestrel_rt_fmod_f32`), both through an `extern "C"` function pointer. This times the slow-path
//! target and `pow`, which stays a call; the inline code is timed by `bench/kernels/math_<op>.kes`.
//! The identity rows are the harness's own cost.
//!
//! The spike's method: 65,536 inputs times 153 passes (about 10M calls), every row run once per
//! round, 9 interleaved rounds, then the median and range. Inputs are random (branches on the input
//! mispredict, as with random angles); `ORDER=seq` spreads them evenly in order instead (the
//! second argument runs the other way, high to low). Run it only on a quiet machine, in release:
//! `cargo test -p kestrel-runtime --release wide_math_speed -- --ignored --nocapture`, then again
//! with `ORDER=seq`. `ROUNDS` and `PASSES` change the 9 and the 153, for a quick check that it runs.

use std::hint::black_box;
use std::time::Instant;

use super::*;
use crate::panic::kestrel_rt_fmod_f32;

const LEN: usize = 65_536;

type F1 = extern "C" fn(f32) -> f32;
type F2 = extern "C" fn(f32, f32) -> f32;

#[derive(Clone, Copy)]
enum Call {
    One(F1),
    Two(F2),
}

/// SplitMix64, as `sweep.rs` and the spike draw their inputs.
struct Rng(u64);

impl Rng {
    fn u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `[lo, hi)`, rounded to `f32`.
    fn range(&mut self, lo: f64, hi: f64) -> f32 {
        let unit = (self.u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64);
        (lo + (hi - lo) * unit) as f32
    }
}

/// `LEN` inputs for each argument: random, or with `ORDER=seq` spread evenly in order (the second
/// from `hi2` down to `lo2`, so `atan2`'s pairs don't all lie on the diagonal).
fn inputs(seq: bool, (lo, hi): (f64, f64), (lo2, hi2): (f64, f64)) -> (Vec<f32>, Vec<f32>) {
    let mut r = Rng(42);
    let mut a = Vec::with_capacity(LEN);
    let mut b = Vec::with_capacity(LEN);
    for i in 0..LEN {
        if seq {
            let t = i as f64 / LEN as f64;
            a.push((lo + (hi - lo) * t) as f32);
            b.push((hi2 - (hi2 - lo2) * t) as f32);
        } else {
            a.push(r.range(lo, hi));
            b.push(r.range(lo2, hi2));
        }
    }
    (a, b)
}

/// Nanoseconds per call of `call` over `passes` passes of the inputs, summing the results.
fn time(a: &[f32], b: &[f32], passes: usize, call: Call) -> f64 {
    let start = Instant::now();
    let mut sum = 0.0f32;
    match call {
        Call::One(f) => {
            let f = black_box(f);
            for _ in 0..passes {
                for &x in a {
                    sum += f(x);
                }
                black_box(a);
            }
        }
        Call::Two(f) => {
            let f = black_box(f);
            for _ in 0..passes {
                for (&x, &y) in a.iter().zip(b) {
                    sum += f(x, y);
                }
                black_box(a);
            }
        }
    }
    black_box(sum);
    start.elapsed().as_nanos() as f64 / (a.len() * passes) as f64
}

fn median(v: &mut [f64]) -> String {
    v.sort_by(f64::total_cmp);
    format!("{:.2} ({:.2}-{:.2})", v[v.len() / 2], v[0], v[v.len() - 1])
}

extern "C" fn identity1(x: f32) -> f32 {
    x
}
extern "C" fn identity2(x: f32, _: f32) -> f32 {
    x
}

macro_rules! today1 {
    ($($name:ident = $f:path;)*) => {$(
        extern "C" fn $name(x: f32) -> f32 {
            $f(x as f64) as f32
        }
    )*};
}
today1! {
    today_sin = libm::sin;
    today_cos = libm::cos;
    today_tan = libm::tan;
    today_asin = libm::asin;
    today_acos = libm::acos;
    today_atan = libm::atan;
    today_exp = libm::exp;
    today_log = libm::log;
}
extern "C" fn today_atan2(y: f32, x: f32) -> f32 {
    libm::atan2(y as f64, x as f64) as f32
}
extern "C" fn today_pow(x: f32, y: f32) -> f32 {
    libm::pow(x as f64, y as f64) as f32
}
extern "C" fn today_rem(a: f32, b: f32) -> f32 {
    libm::fmodf(a, b)
}

/// One function's rows: its name, its inputs' ranges (the second unused for one argument), and
/// `today` and the wide function.
struct Function {
    name: &'static str,
    range: (f64, f64),
    range2: (f64, f64),
    today: Call,
    wide: Call,
}

/// The kernels' ranges (`bench/kernels/math_<op>.kes`), which are the spike's.
fn functions() -> Vec<Function> {
    use Call::{One, Two};
    let f = |name, range, range2, today, wide| Function { name, range, range2, today, wide };
    let none = (0.0, 0.0);
    vec![
        f("sin", (-10.0, 10.0), none, One(today_sin), One(kestrel_rt_sinf)),
        f("cos", (-10.0, 10.0), none, One(today_cos), One(kestrel_rt_cosf)),
        f("tan", (-10.0, 10.0), none, One(today_tan), One(kestrel_rt_tanf)),
        f("asin", (-1.0, 1.0), none, One(today_asin), One(kestrel_rt_asinf)),
        f("acos", (-1.0, 1.0), none, One(today_acos), One(kestrel_rt_acosf)),
        f("atan", (-100.0, 100.0), none, One(today_atan), One(kestrel_rt_atanf)),
        f("atan2", (-100.0, 100.0), (-100.0, 100.0), Two(today_atan2), Two(kestrel_rt_atan2f)),
        f("exp", (-50.0, 50.0), none, One(today_exp), One(kestrel_rt_expf)),
        f("log", (1e-3, 1000.0), none, One(today_log), One(kestrel_rt_logf)),
        f("pow", (0.01, 10.0), (-10.0, 10.0), Two(today_pow), Two(kestrel_rt_powf)),
        f("rem", (-1000.0, 1000.0), (0.5, 10.0), Two(today_rem), Two(kestrel_rt_fmod_f32)),
    ]
}

fn env_number(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[test]
#[ignore = "a measurement: run it by hand on a quiet machine, in release"]
fn wide_math_speed() {
    let rounds = env_number("ROUNDS", 9).max(1);
    let passes = env_number("PASSES", 153).max(1);
    let seq = std::env::var("ORDER").is_ok_and(|o| o == "seq");
    // (function, version, inputs, call): every row, in the order a round runs them.
    let mut rows: Vec<(&str, &str, usize, Call)> = Vec::new();
    let mut data: Vec<(Vec<f32>, Vec<f32>)> = Vec::new();
    data.push(inputs(seq, (-10.0, 10.0), (-10.0, 10.0)));
    rows.push(("identity", "one argument (the harness's own cost)", 0, Call::One(identity1)));
    rows.push(("identity", "two arguments (the harness's own cost)", 0, Call::Two(identity2)));
    for f in functions() {
        data.push(inputs(seq, f.range, f.range2));
        let at = data.len() - 1;
        rows.push((f.name, "today (what 49ae9f0 runs)", at, f.today));
        rows.push((f.name, "wide (the kestrel_rt function)", at, f.wide));
    }
    let mut times = vec![Vec::new(); rows.len()];
    for _ in 0..rounds {
        for (i, &(_, _, at, call)) in rows.iter().enumerate() {
            let (a, b) = &data[at];
            times[i].push(time(a, b, passes, call));
        }
    }
    let order = if seq { "in order" } else { "random" };
    println!("{LEN} {order} inputs times {passes} passes, {rounds} interleaved rounds");
    println!("| function | version | ns per call, median of {rounds} (range) |");
    println!("|---|---|---:|");
    for (i, (name, version, _, _)) in rows.iter().enumerate() {
        println!("| {name} | {version} | {} |", median(&mut times[i]));
    }
    for &(name, version, at, call) in &rows {
        let (a, b) = &data[at];
        let sum = sum_once(a, b, call);
        assert!(sum.is_finite(), "{name} {version} summed to {sum}");
    }
}

/// One pass's sum in `f64`, to check every row computes finite results.
fn sum_once(a: &[f32], b: &[f32], call: Call) -> f64 {
    match call {
        Call::One(f) => a.iter().map(|&x| f(x) as f64).sum(),
        Call::Two(f) => a.iter().zip(b).map(|(&x, &y)| f(x, y) as f64).sum(),
    }
}
