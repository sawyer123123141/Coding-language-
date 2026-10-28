//! Kestrel's own math (`libm`) against the platform's (Rust's `f64` methods, which call the C
//! runtime), in nanoseconds per call. Run it only on a quiet machine, in release:
//! `cargo test -p kestrel-runtime --release own_math_against_the_platform -- --ignored --nocapture`.

use std::hint::black_box;
use std::time::Instant;

const CALLS: usize = 10_000_000;

/// Nanoseconds per call of `f` over `CALLS` inputs spread evenly from `lo` to `hi` (the second
/// argument runs from `lo2` to `hi2` at the same time).
fn per_call(lo: f64, hi: f64, lo2: f64, hi2: f64, f: impl Fn(f64, f64) -> f64) -> f64 {
    let step = (hi - lo) / CALLS as f64;
    let step2 = (hi2 - lo2) / CALLS as f64;
    let start = Instant::now();
    let mut sum = 0.0;
    for i in 0..CALLS {
        let x = black_box(lo + i as f64 * step);
        let y = black_box(lo2 + i as f64 * step2);
        sum += f(x, y);
    }
    black_box(sum);
    start.elapsed().as_nanos() as f64 / CALLS as f64
}

type Pair = (fn(f64, f64) -> f64, fn(f64, f64) -> f64);
/// Name, range of the first argument, range of the second, (libm, platform).
type Row = (&'static str, (f64, f64), (f64, f64), Pair);

#[test]
#[ignore = "a measurement: run it by hand on a quiet machine, in release"]
fn own_math_against_the_platform() {
    let rows: [Row; 11] = [
        ("sin", (-10.0, 10.0), (0.0, 0.0), (|x, _| libm::sin(x), |x, _| x.sin())),
        ("cos", (-10.0, 10.0), (0.0, 0.0), (|x, _| libm::cos(x), |x, _| x.cos())),
        ("tan", (-1.5, 1.5), (0.0, 0.0), (|x, _| libm::tan(x), |x, _| x.tan())),
        ("asin", (-1.0, 1.0), (0.0, 0.0), (|x, _| libm::asin(x), |x, _| x.asin())),
        ("acos", (-1.0, 1.0), (0.0, 0.0), (|x, _| libm::acos(x), |x, _| x.acos())),
        ("atan", (-100.0, 100.0), (0.0, 0.0), (|x, _| libm::atan(x), |x, _| x.atan())),
        ("atan2", (-100.0, 100.0), (100.0, -100.0), (|y, x| libm::atan2(y, x), |y, x| y.atan2(x))),
        ("exp", (-50.0, 50.0), (0.0, 0.0), (|x, _| libm::exp(x), |x, _| x.exp())),
        ("log", (1e-3, 1000.0), (0.0, 0.0), (|x, _| libm::log(x), |x, _| x.ln())),
        ("pow", (0.01, 10.0), (-10.0, 10.0), (|a, b| libm::pow(a, b), |a, b| a.powf(b))),
        ("fmod", (-1000.0, 1000.0), (0.5, 10.0), (|a, b| libm::fmod(a, b), |a, b| a % b)),
    ];
    for (name, (lo, hi), (lo2, hi2), (own, platform)) in rows {
        let own = per_call(lo, hi, lo2, hi2, own);
        let platform = per_call(lo, hi, lo2, hi2, platform);
        println!("{name:>5}: libm {own:.2} ns, platform {platform:.2} ns, ratio {:.2}", own / platform);
    }
}

const PINS: usize = 100_000_000;

/// Nanoseconds per call of `f`, over `PINS` calls.
fn per_pin(f: impl Fn()) -> f64 {
    let start = Instant::now();
    for _ in 0..PINS {
        f();
    }
    start.elapsed().as_nanos() as f64 / PINS as f64
}

/// What pinning the float mode costs: `fpu::pin` (one `ldmxcsr`) inlined into a loop, and the
/// call that compiled code makes after a native of the window library returns
/// (`kestrel_rt_fpu_pin`, through a pointer the optimiser can't see through). Run it like
/// `own_math_against_the_platform`: `cargo test -p kestrel-runtime --release pinning_the_float_mode
/// -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run it by hand on a quiet machine, in release"]
fn pinning_the_float_mode() {
    let empty = per_pin(|| black_box(()));
    let inline = per_pin(|| {
        crate::fpu::pin();
        black_box(());
    });
    let entry: extern "C" fn() = black_box(crate::fpu::kestrel_rt_fpu_pin);
    let call = per_pin(|| black_box(entry)());
    // The low six bits are the sticky exception flags, which the timing's own arithmetic sets.
    assert_eq!(crate::fpu::read() & !0x3F, crate::fpu::PINNED);
    println!("empty loop {empty:.3} ns, pin inlined {inline:.3} ns, kestrel_rt_fpu_pin called {call:.3} ns per iteration ({PINS} each)");
}
