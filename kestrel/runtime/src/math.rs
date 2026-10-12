//! Math and random numbers. Compiled code does `sqrt`, `floor` and the like
//! with Cranelift instructions; the rest (trigonometry, exponentials) calls
//! one function here per operation, in `f64`.

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

#[no_mangle]
pub extern "C" fn kestrel_rt_sin(a: f64) -> f64 {
    a.sin()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_cos(a: f64) -> f64 {
    a.cos()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_tan(a: f64) -> f64 {
    a.tan()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_asin(a: f64) -> f64 {
    a.asin()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_acos(a: f64) -> f64 {
    a.acos()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_atan(a: f64) -> f64 {
    a.atan()
}

/// The angle of the point (`x`, `y`).
#[no_mangle]
pub extern "C" fn kestrel_rt_atan2(y: f64, x: f64) -> f64 {
    y.atan2(x)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_exp(a: f64) -> f64 {
    a.exp()
}

/// The natural logarithm.
#[no_mangle]
pub extern "C" fn kestrel_rt_log(a: f64) -> f64 {
    a.ln()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_pow(a: f64, b: f64) -> f64 {
    a.powf(b)
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
