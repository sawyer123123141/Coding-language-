//! Math and random numbers. Compiled code does `sqrt`, `floor` and the like
//! with Cranelift instructions. `float` trigonometry, exponentials and `%` are
//! inline code (a copy of the wide functions of `math/wide.rs`) whose special
//! inputs call the `kestrel_rt_<op>f` functions here; `pow` is always a call.
//! `f64` math calls one `libm` function here per operation.
//!
//! All of it is Kestrel's own: the wide functions, and the `libm` crate, a
//! pure-Rust port of musl's libm, instead of the C runtime's, which picks
//! different code (and can round differently) on CPUs with FMA. So a program computes the same
//! bits on every x86-64 CPU, which replays and networked games depend on.
//! `libm` is pinned to one version, without its `arch` feature, so nothing in
//! an exe's math looks at the CPU; `math/bits.rs` pins the results for a
//! table of inputs. The JIT's copy (linked into the compiler) does get `arch`,
//! turned on by Cranelift's own dependency on `libm`. The results are still
//! equal: `arch` only changes `sqrt` and `fma`, `fma` is never called here, and
//! `sqrt` is correctly rounded either way (spec decision 2).
//!
//! Which path each type takes: `f64` math calls `libm` (`kestrel_rt_sin` and so on below). `float`
//! (`f32`) math calls the wide functions of `math/wide.rs` (`kestrel_rt_sinf` and so on), which
//! compute in `f64` from `+ - * /` and exact steps and round once, so they are the same on every
//! CPU by construction; their special inputs (NaN, infinities, huge arguments) take today's `libm`
//! path. The compiler inlines their fast paths and calls these functions for the rest.

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod wide;
#[doc(hidden)]
pub mod sweep;

#[no_mangle]
pub extern "C" fn kestrel_rt_sinf(x: f32) -> f32 {
    wide::sinf(x)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_cosf(x: f32) -> f32 {
    wide::cosf(x)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_tanf(x: f32) -> f32 {
    wide::tanf(x)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_asinf(x: f32) -> f32 {
    wide::asinf(x)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_acosf(x: f32) -> f32 {
    wide::acosf(x)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_atanf(x: f32) -> f32 {
    wide::atanf(x)
}

/// The angle of the point (`x`, `y`).
#[no_mangle]
pub extern "C" fn kestrel_rt_atan2f(y: f32, x: f32) -> f32 {
    wide::atan2f(y, x)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_expf(x: f32) -> f32 {
    wide::expf(x)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_logf(x: f32) -> f32 {
    wide::logf(x)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_powf(x: f32, y: f32) -> f32 {
    wide::powf(x, y)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_sin(a: f64) -> f64 {
    libm::sin(a)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_cos(a: f64) -> f64 {
    libm::cos(a)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_tan(a: f64) -> f64 {
    libm::tan(a)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_asin(a: f64) -> f64 {
    libm::asin(a)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_acos(a: f64) -> f64 {
    libm::acos(a)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_atan(a: f64) -> f64 {
    libm::atan(a)
}

/// The angle of the point (`x`, `y`).
#[no_mangle]
pub extern "C" fn kestrel_rt_atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_exp(a: f64) -> f64 {
    libm::exp(a)
}

/// The natural logarithm.
#[no_mangle]
pub extern "C" fn kestrel_rt_log(a: f64) -> f64 {
    libm::log(a)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_pow(a: f64, b: f64) -> f64 {
    libm::pow(a, b)
}

thread_local! {
    /// SplitMix64 state. Each thread starts from the clock, like Godot 4.
    static STATE: Cell<u64> = Cell::new(clock_seed());
}

/// The clock's nanoseconds, mixed with a stack address so that threads
/// starting in the same instant still differ.
fn clock_seed() -> u64 {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let local = 0u8;
    nanos ^ (&local as *const u8 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn next(state: &Cell<u64>) -> u64 {
    let s = state.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
    state.set(s);
    let mut z = s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A float in `[0, 1)`.
#[no_mangle]
pub extern "C" fn kestrel_rt_rand_float() -> f64 {
    STATE.with(|s| (next(s) >> 11) as f64 * (1.0 / (1u64 << 53) as f64))
}

/// An integer between `lo` and `hi`, both included (swapped if `lo > hi`).
#[no_mangle]
pub extern "C" fn kestrel_rt_rand_int(lo: i64, hi: i64) -> i64 {
    let (lo, hi) = if lo > hi { (hi, lo) } else { (lo, hi) };
    let span = (hi.wrapping_sub(lo) as u64).wrapping_add(1);
    STATE.with(|s| {
        let r = next(s);
        // A span of 0 is the whole `i64` range: every bit pattern is fair.
        if span == 0 {
            r as i64
        } else {
            lo.wrapping_add(((r as u128 * span as u128) >> 64) as i64)
        }
    })
}

/// Makes this thread's random numbers repeat: the same seed, the same sequence.
#[no_mangle]
pub extern "C" fn kestrel_rt_rand_seed(seed: i64) {
    STATE.with(|s| s.set(seed as u64));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_ops() {
        assert_eq!(kestrel_rt_sin(0.0), 0.0);
        assert_eq!(kestrel_rt_cos(0.0), 1.0);
        assert_eq!(kestrel_rt_tan(0.0), 0.0);
        assert_eq!(kestrel_rt_asin(0.0), 0.0);
        assert_eq!(kestrel_rt_acos(1.0), 0.0);
        assert_eq!(kestrel_rt_atan(0.0), 0.0);
        assert_eq!(kestrel_rt_atan2(0.0, 1.0), 0.0);
        assert_eq!(kestrel_rt_exp(0.0), 1.0);
        assert_eq!(kestrel_rt_log(1.0), 0.0);
        assert_eq!(kestrel_rt_pow(2.0, 10.0), 1024.0);
    }

    #[test]
    fn seeded_sequences_repeat() {
        let take = || {
            kestrel_rt_rand_seed(7);
            let ints: Vec<i64> = (0..5).map(|_| kestrel_rt_rand_int(1, 6)).collect();
            let floats: Vec<f64> = (0..5).map(|_| kestrel_rt_rand_float()).collect();
            (ints, floats)
        };
        assert_eq!(take(), take());
    }

    #[test]
    fn ints_cover_their_range() {
        kestrel_rt_rand_seed(1);
        let mut seen = [false; 6];
        for _ in 0..1000 {
            let n = kestrel_rt_rand_int(1, 6);
            assert!((1..=6).contains(&n));
            seen[(n - 1) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s), "every value turns up: {seen:?}");
        for _ in 0..100 {
            assert!((1..=6).contains(&kestrel_rt_rand_int(6, 1)));
        }
        // The full range has 2^64 values; no overflow.
        kestrel_rt_rand_int(i64::MIN, i64::MAX);
    }

    #[test]
    fn floats_stay_below_one() {
        for _ in 0..10_000 {
            let f = kestrel_rt_rand_float();
            assert!((0.0..1.0).contains(&f), "{f}");
        }
    }
}

#[cfg(test)]
mod bits;
#[cfg(test)]
mod bits_float;
#[cfg(test)]
mod speed;
#[cfg(test)]
mod wide_speed;
#[cfg(test)]
mod wide_test;
