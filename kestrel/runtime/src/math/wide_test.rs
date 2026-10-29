//! The wide `float` functions against `libm`'s `f64` results: accuracy on the sweep, today's bits
//! on special inputs, the bit tests against their float definitions, and a hash of every result
//! that must be the same in every build (baseline, `target-cpu=native`, debug).
//! The long ones are ignored; run them by hand in release:
//! `cargo test -p kestrel-runtime --release exhaustive_one_argument_functions -- --ignored --nocapture`.

// `float_definition` writes spec decision 2's tests as negated float compares on purpose: NaN
// makes each one true.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::sweep::{self, edges, reference, same, today, ulps, FUNCTIONS};
use super::wide::*;
use std::sync::OnceLock;

/// Every function's sweep, in `FUNCTIONS` order, made once per test run.
fn sweeps() -> &'static [Vec<(f32, f32)>] {
    static SWEEPS: OnceLock<Vec<Vec<(f32, f32)>>> = OnceLock::new();
    SWEEPS.get_or_init(|| {
        std::thread::scope(|s| {
            let handles: Vec<_> = FUNCTIONS.iter().map(|&name| s.spawn(move || sweep::inputs(name))).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        })
    })
}

/// `name`'s results on its sweep (the wide functions, as compiled code calls them).
fn results(name: &str, xs: &[(f32, f32)]) -> Vec<f32> {
    xs.iter().map(|&(a, b)| sweep::wide(name, a, b)).collect()
}

fn two_arguments(name: &str) -> bool {
    matches!(name, "atan2" | "pow" | "fmod")
}

/// Whether `name`'s special test sends `(a, b)` to the slow path.
fn special(name: &str, a: f32, b: f32) -> bool {
    match name {
        "sin" => sin_special(a),
        "cos" => cos_special(a),
        "tan" => tan_special(a),
        "asin" | "acos" => asin_special(a),
        "atan" => atan_special(a),
        "atan2" => atan2_special(a, b),
        "exp" => exp_special(a),
        "log" => log_special(a),
        "pow" => pow_special(a, b),
        "fmod" => fmod_special(a, b, libm::trunc(a as f64 / b as f64)),
        _ => panic!("no function {name}"),
    }
}

#[test]
fn every_float_function_is_within_0_51_ulp_on_the_sweep() {
    let reports: Vec<(f64, String)> = std::thread::scope(|s| {
        let handles: Vec<_> = FUNCTIONS
            .iter()
            .zip(sweeps())
            .map(|(&name, xs)| {
                s.spawn(move || {
                    let mut worst: Vec<(f64, f32, f32)> = Vec::new();
                    let (mut max, mut over) = (0f64, 0usize);
                    for &(a, b) in xs {
                        let u = ulps(sweep::wide(name, a, b), reference(name, a, b));
                        max = max.max(u);
                        if u > MAX_ULP {
                            over += 1;
                        }
                        if worst.len() < 3 || u > worst[2].0 {
                            worst.push((u, a, b));
                            worst.sort_by(|x, y| y.0.total_cmp(&x.0));
                            worst.truncate(3);
                        }
                    }
                    let worst: Vec<String> = worst.iter().map(|(u, a, b)| format!("{name}({a:e}, {b:e}) {u:.6} ulp")).collect();
                    println!("{name}: max {max:.6} ulp over {} inputs", xs.len());
                    let failure = if over == 0 { String::new() } else { format!("{name}: {over} inputs over {MAX_ULP} ulp; worst {}", worst.join(", ")) };
                    (max, failure)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let failures: Vec<&str> = reports.iter().map(|(_, f)| f.as_str()).filter(|f| !f.is_empty()).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn special_inputs_keep_todays_bits() {
    let e = edges();
    let mut wrong = Vec::new();
    for (&name, xs) in FUNCTIONS.iter().zip(sweeps()) {
        // Every edge (every pair of edges for two arguments), and the sweep's random specials too.
        let pairs: Vec<(f32, f32)> = if two_arguments(name) { e.iter().flat_map(|&a| e.iter().map(move |&b| (a, b))).collect() } else { e.iter().map(|&a| (a, 0.0)).collect() };
        for (a, b) in pairs.into_iter().chain(xs.iter().copied()) {
            if special(name, a, b) {
                let (w, t) = (sweep::wide(name, a, b), today(name, a, b));
                if !same(w, t) {
                    wrong.push(format!("{name}({a:e}, {b:e}) = {w:e}, today {t:e}"));
                }
            }
        }
    }
    let count = wrong.len();
    wrong.truncate(20);
    assert!(wrong.is_empty(), "{count} special inputs changed, among them:\n{}", wrong.join("\n"));
}

/// Spec decision 2's table written with float compares, under the pinned mode.
fn float_definition(name: &str, a: f32, b: f32) -> bool {
    let inf = f32::INFINITY;
    match name {
        "sin" => !(a.abs() < SIN_FAST),
        "cos" => !(a.abs() < COS_FAST),
        "tan" => !(a.abs() < TAN_FAST),
        "asin" | "acos" => !(a.abs() <= 1.0),
        "atan" => !(a.abs() < inf),
        "atan2" => !(a.abs() < inf && b.abs() < inf) || (a == 0.0 && b == 0.0),
        "exp" => a.is_nan(),
        "log" => !(a > 0.0 && a < inf),
        "pow" => !(a > 0.0 && a < inf) || !(b.abs() < inf),
        "fmod" => {
            let q = libm::trunc(a as f64 / b as f64);
            !(a.abs() < inf) || b == 0.0 || !(b.abs() < inf) || !(q.abs() < FMOD_Q_LIMIT)
        }
        _ => panic!("no function {name}"),
    }
}

#[test]
fn the_special_tests_match_the_float_definitions() {
    let mut xs: Vec<(f32, f32)> = edges().iter().flat_map(|&a| edges().into_iter().map(move |b| (a, b))).collect();
    let mut state = 0x5EED_B175u64;
    for _ in 0..1_000_000 {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let z = (state ^ (state >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        xs.push((f32::from_bits(z as u32), f32::from_bits((z >> 32) as u32)));
    }
    let mut wrong = Vec::new();
    for &(a, b) in &xs {
        for name in ["sin", "cos", "tan", "asin", "atan", "atan2", "exp", "log", "pow", "fmod"] {
            if special(name, a, b) != float_definition(name, a, b) {
                wrong.push(format!("{name}({a:e}, {b:e}): bit test {}", special(name, a, b)));
            }
        }
        if tiny(a) != (a.abs() < TINY) {
            wrong.push(format!("tiny({a:e}): bit test {}", tiny(a)));
        }
    }
    wrong.truncate(20);
    assert!(wrong.is_empty(), "bit tests that disagree with the float definitions:\n{}", wrong.join("\n"));
}

#[test]
fn float_remainder_is_identical_to_libm() {
    let xs = &sweeps()[FUNCTIONS.iter().position(|&f| f == "fmod").unwrap()];
    let wrong: Vec<String> = xs
        .iter()
        .filter(|&&(a, b)| !same(fmodf(a, b), libm::fmodf(a, b)))
        .take(20)
        .map(|&(a, b)| format!("{a:e} % {b:e} = {:e}, libm {:e}", fmodf(a, b), libm::fmodf(a, b)))
        .collect();
    assert!(wrong.is_empty(), "remainders that differ from libm::fmodf:\n{}", wrong.join("\n"));
}

#[test]
fn the_36_det_math_float_rows_keep_their_bits() {
    let mut wrong = Vec::new();
    for x in [0.5f32, 1.0, 2.5, -3.0, 100.0, 1234.5] {
        for (name, a, b) in [("sin", x, 0.0), ("cos", x, 0.0), ("atan2", x, 0.75), ("exp", x, 0.0), ("log", x, 0.0), ("pow", 1.25, x)] {
            let (w, t) = (sweep::wide(name, a, b), today(name, a, b));
            if !same(w, t) {
                wrong.push(format!("{name}({a}, {b}) = {w:?}, today {t:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "det_math rows that changed:\n{}", wrong.join("\n"));
}

/// FNV-1a 64 of every function's results on its sweep, in `FUNCTIONS` order, NaN as `0x7FC0_0000`.
/// Every build of the runtime must give this value.
const WIDE_HASH: u64 = 0x2271_60C4_5A77_805E;

#[test]
fn wide_results_hash_is_pinned() {
    let mut h = 0xCBF2_9CE4_8422_2325u64;
    for (&name, xs) in FUNCTIONS.iter().zip(sweeps()) {
        for r in results(name, xs) {
            let bits = if r.is_nan() { 0x7FC0_0000 } else { r.to_bits() };
            for byte in bits.to_le_bytes() {
                h = (h ^ byte as u64).wrapping_mul(0x100_0000_01B3);
            }
        }
    }
    assert_eq!(h, WIDE_HASH, "the wide results hash to {h:#018X}");
}

/// Per thread: the largest ulps and its input, and how many results differ from today's (which is
/// `reference as f32`, `libm`'s `f64` result rounded once).
fn exhaustive_part(wide: fn(f32) -> f32, refer: fn(f64) -> f64, lo: u64, hi: u64) -> (f64, u32, u64) {
    let (mut max, mut at, mut differ) = (0f64, 0u32, 0u64);
    for bits in lo..hi {
        let x = f32::from_bits(bits as u32);
        let (w, r) = (wide(x), refer(x as f64));
        let u = ulps(w, r);
        if u > max {
            (max, at) = (u, bits as u32);
        }
        if !same(w, r as f32) {
            differ += 1;
        }
    }
    (max, at, differ)
}

/// A one-argument function: its name, the wide function and `libm`'s `f64` reference.
type OneArgument = (&'static str, fn(f32) -> f32, fn(f64) -> f64);

#[test]
#[ignore = "every f32 input: about 2 minutes in release on 8 threads"]
fn exhaustive_one_argument_functions() {
    if cfg!(debug_assertions) {
        panic!("run this test with --release");
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8) as u64;
    let functions: [OneArgument; 8] = [
        ("sin", sinf, libm::sin),
        ("cos", cosf, libm::cos),
        ("tan", tanf, libm::tan),
        ("asin", asinf, libm::asin),
        ("acos", acosf, libm::acos),
        ("atan", atanf, libm::atan),
        ("exp", expf, libm::exp),
        ("log", logf, libm::log),
    ];
    let mut over = Vec::new();
    for (name, wide, refer) in functions {
        let step = (1u64 << 32) / threads;
        let parts: Vec<(f64, u32, u64)> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads).map(|t| s.spawn(move || exhaustive_part(wide, refer, t * step, if t + 1 == threads { 1 << 32 } else { (t + 1) * step }))).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let (max, at) = parts.iter().fold((0f64, 0u32), |m, p| if p.0 > m.0 { (p.0, p.1) } else { m });
        let differ: u64 = parts.iter().map(|p| p.2).sum();
        println!("{name}: max {max:.6} ulp at {:e} ({at:#010X}); {differ} results differ from today's (libm's f64 result rounded once)", f32::from_bits(at));
        if max > MAX_ULP {
            over.push(format!("{name}: {max:.6} ulp at {:e} ({at:#010X})", f32::from_bits(at)));
        }
    }
    assert!(over.is_empty(), "over {MAX_ULP} ulp:\n{}", over.join("\n"));
}

#[test]
#[ignore = "prints the accuracy table for results.md: run it by hand with --nocapture"]
fn print_wide_report() {
    println!("| function | inputs | max ulp | mean ulp | same as today | equal to the correctly rounded value |");
    println!("|---|---:|---:|---:|---:|---:|");
    for (&name, xs) in FUNCTIONS.iter().zip(sweeps()) {
        let (mut max, mut sum, mut n, mut today_same, mut rounded) = (0f64, 0f64, 0usize, 0usize, 0usize);
        for &(a, b) in xs {
            let (w, r) = (sweep::wide(name, a, b), reference(name, a, b));
            let u = ulps(w, r);
            if u.is_finite() {
                max = max.max(u);
                sum += u;
                n += 1;
            }
            today_same += same(w, today(name, a, b)) as usize;
            rounded += same(w, r as f32) as usize;
        }
        let pct = |c: usize| c as f64 * 100.0 / xs.len() as f64;
        println!("| `{name}` | {} | {max:.4} | {:.4} | {:.4}% | {:.4}% |", xs.len(), sum / n as f64, pct(today_same), pct(rounded));
    }
}
