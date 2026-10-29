//! The inputs and measures that test the wide `float` functions (`wide.rs`): a sweep of random
//! inputs plus edge values per function, `libm`'s `f64` reference, today's results (the `f64`
//! `libm` path narrowed once) and the error in ulps. The runtime's own tests use it, and so do the
//! codegen and CLI tests, which compare compiled code with these functions on the same inputs.
//! Everything here is deterministic: the same inputs in every build.

// The edges are `f32` roundings of constants like π/4 and sqrt(2), on purpose.
#![allow(clippy::approx_constant)]

use crate::panic::kestrel_rt_fmod_f32;

/// Every wide function, in the order the tests and the pinned hash walk them.
pub const FUNCTIONS: [&str; 11] = ["sin", "cos", "tan", "asin", "acos", "atan", "atan2", "exp", "log", "pow", "fmod"];

/// How many random inputs each function's sweep has, before the edges.
pub const RANDOM: usize = 1_000_000;

/// SplitMix64, seeded per function.
struct Rng(u64);

impl Rng {
    fn u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// In `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
    /// Uniform in `[lo, hi)`, rounded to `f32`.
    fn range(&mut self, lo: f64, hi: f64) -> f32 {
        (lo + (hi - lo) * self.unit()) as f32
    }
    /// Any bit pattern: NaNs, infinities and subnormals included.
    fn bits(&mut self) -> f32 {
        f32::from_bits(self.u64() as u32)
    }
    /// A magnitude log-uniform in `[2^lo, 2^hi]`, with a random sign.
    fn logu(&mut self, lo: f64, hi: f64) -> f32 {
        let m = libm::exp2(lo + (hi - lo) * self.unit()) as f32;
        if self.u64() & 1 == 0 {
            m
        } else {
            -m
        }
    }
}

/// The edge values: signed zeros, subnormals, the extremes, infinities, NaN, each domain's and
/// each reduction's edges, every input of `bits.rs` as an `f32`, and every input of
/// `bits_float.rs`. No two have the same bits.
#[rustfmt::skip]
pub fn edges() -> Vec<f32> {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    let all: &[f32] = &[
        // The spike's edges.
        0.0, -0.0, f32::MIN_POSITIVE, -f32::MIN_POSITIVE, 1e-45, -1e-45, 1e-40, -1e-40, 1e-20, f32::MAX, -f32::MAX,
        f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 1.0, -1.0, 0.5, 2.0, 3.0, 10.0, PI, FRAC_PI_2, FRAC_PI_4, -PI,
        100.0, 1e5, 1e10, 1e30, 88.72, 88.73, 89.0, -87.0, -103.97, -104.0, -110.0, 2.4414062e-4, 2.44e-4, 8191.99,
        8192.0, 4.0e8, 4.3e8, 1.0000001, 0.99999994, 0.70710677, 1.4142135, 1e7, -1e7,
        // `bits.rs`'s inputs as `f32`s (1e-300 and 1e300 become 0 and infinity).
        -3.0, 1e6, 1e22, -0.5, 0.3, 0.99999, 1.5, 709.78, 710.0, -745.1, -746.0, 0.33333334, 1024.0, -1075.0,
        -1074.0, 7.5, -7.5, -8.0,
        // `bits_float.rs`'s inputs.
        -2.4414062e-4, 2.5, 0.7853982, 1.5707964, 1234.5, 399999968.0, 0.41421357, -1e30, 0.75, 128.0, -150.0,
        -149.0, 536870912.0, 3e-41, 1.25, -2.0, -5.0, 5.0,
        // The review's range edges: the fast bound's neighbours, 2^28, 2^28 π/2, odd multiples of
        // π/4, ±1 neighbours, subnormal `exp` results and a quotient just below 2^29.
        400000032.0, 268435456.0, 421657440.0, 2.3561945, 3.926991, -0.7853982, 5.4977875, -0.99999994,
        -1.0000001, -100.0, -102.0, 536870880.0, 0.0625, 1e-30, -1e-10, 6.0,
        // The negative fast bound and its neighbour, and the high side of `log`'s sqrt(1/2) split:
        // the floats just above 0.70710677 and 1.4142135 (0.7071068 prints as 0.70710683).
        -399999968.0, -4.0e8, 0.7071068, 1.4142137,
    ];
    let mut v: Vec<f32> = Vec::with_capacity(all.len());
    for &x in all {
        if !v.iter().any(|&e| e.to_bits() == x.to_bits()) {
            v.push(x);
        }
    }
    v
}

/// The seed of `name`'s sweep: FNV-1a of the name.
fn seed(name: &str) -> u64 {
    name.bytes().fold(0xCBF2_9CE4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01B3))
}

/// One random input of `name`, from the spike's mixes (typical ranges, log-uniform magnitudes and
/// random bit patterns).
fn random(name: &str, r: &mut Rng) -> (f32, f32) {
    let p = r.unit();
    match name {
        "sin" | "cos" | "tan" => {
            let x = if p < 0.4 {
                r.range(-10.0, 10.0)
            } else if p < 0.8 {
                r.range(-1000.0, 1000.0)
            } else if p < 0.9 {
                r.logu(-30.0, 30.0)
            } else {
                r.bits()
            };
            (x, 0.0)
        }
        "asin" | "acos" => {
            let x = if p < 0.5 {
                r.range(-1.0, 1.0)
            } else if p < 0.7 {
                // [0.5, 1] by bit pattern, with a random sign: dense near ±1.
                let m = 0x3F00_0000 + (r.u64() % 0x0080_0001) as u32;
                f32::from_bits(m | ((r.u64() as u32) & 0x8000_0000))
            } else if p < 0.85 {
                r.logu(-30.0, 0.0)
            } else {
                r.bits()
            };
            (x, 0.0)
        }
        "atan" => (if p < 0.4 { r.range(-10.0, 10.0) } else if p < 0.8 { r.logu(-40.0, 40.0) } else { r.bits() }, 0.0),
        "exp" => (if p < 0.7 { r.range(-110.0, 95.0) } else { r.bits() }, 0.0),
        "log" => {
            let x = if p < 0.6 {
                f32::from_bits(r.u64() as u32 & 0x7FFF_FFFF)
            } else if p < 0.8 {
                r.range(0.5, 2.0)
            } else {
                r.bits()
            };
            (x, 0.0)
        }
        "pow" if p < 0.1 => {
            // Integers and dyadic rationals to small integer and half-integer powers: the exponents
            // `powf` computes exactly (1, -1, 2 and 0.5) and their neighbours.
            let x = if r.u64() & 1 == 0 { (r.u64() % 5000 + 1) as f32 } else { (r.u64() % 4096 + 1) as f32 / 64.0 };
            const Y: [f32; 10] = [1.0, -1.0, 2.0, 0.5, -2.0, 3.0, -0.5, 0.25, 4.0, -3.0];
            (x, Y[(r.u64() % Y.len() as u64) as usize])
        }
        "pow" => {
            let x = if p < 0.5 {
                r.range(0.0, 10.0)
            } else if p < 0.75 {
                r.logu(-60.0, 60.0).abs()
            } else {
                r.bits()
            };
            let q = r.unit();
            let y = if q < 0.5 {
                r.range(-20.0, 20.0)
            } else if q < 0.75 {
                (r.u64() % 21) as f32 - 10.0
            } else if q < 0.9 {
                r.range(-200.0, 200.0)
            } else {
                r.bits()
            };
            (x, y)
        }
        "atan2" => {
            if p < 0.7 {
                (r.range(-100.0, 100.0), r.range(-100.0, 100.0))
            } else if p < 0.85 {
                (r.logu(-40.0, 40.0), r.logu(-40.0, 40.0))
            } else {
                (r.bits(), r.bits())
            }
        }
        "fmod" => {
            if p < 0.6 {
                (r.range(-1e4, 1e4), r.range(-100.0, 100.0))
            } else if p < 0.8 {
                (r.logu(-30.0, 40.0), r.logu(-20.0, 20.0))
            } else {
                (r.bits(), r.bits())
            }
        }
        _ => panic!("no function {name}"),
    }
}

fn two_arguments(name: &str) -> bool {
    matches!(name, "atan2" | "pow" | "fmod")
}

/// `name`'s sweep: `RANDOM` random inputs, then every edge (every pair of edges for `atan2`, `pow`
/// and `fmod`). The second value is 0 for one-argument functions.
pub fn inputs(name: &str) -> Vec<(f32, f32)> {
    let mut r = Rng(seed(name));
    let e = edges();
    let mut v = Vec::with_capacity(RANDOM + e.len() * e.len());
    for _ in 0..RANDOM {
        v.push(random(name, &mut r));
    }
    for &a in &e {
        if two_arguments(name) {
            v.extend(e.iter().map(|&b| (a, b)));
        } else {
            v.push((a, 0.0));
        }
    }
    v
}

/// The reference: `libm`'s `f64` function on the widened inputs, within one `f64` ulp of the true
/// value (exact for `fmod`).
pub fn reference(name: &str, a: f32, b: f32) -> f64 {
    let (a, b) = (a as f64, b as f64);
    match name {
        "sin" => libm::sin(a),
        "cos" => libm::cos(a),
        "tan" => libm::tan(a),
        "asin" => libm::asin(a),
        "acos" => libm::acos(a),
        "atan" => libm::atan(a),
        "atan2" => libm::atan2(a, b),
        "exp" => libm::exp(a),
        "log" => libm::log(a),
        "pow" => libm::pow(a, b),
        "fmod" => libm::fmod(a, b),
        _ => panic!("no function {name}"),
    }
}

/// The wide function `name`, as compiled code calls it (`kestrel_rt_<name>f`, or
/// `kestrel_rt_fmod_f32` for `fmod`).
pub fn wide(name: &str, a: f32, b: f32) -> f32 {
    use super::*;
    match name {
        "sin" => kestrel_rt_sinf(a),
        "cos" => kestrel_rt_cosf(a),
        "tan" => kestrel_rt_tanf(a),
        "asin" => kestrel_rt_asinf(a),
        "acos" => kestrel_rt_acosf(a),
        "atan" => kestrel_rt_atanf(a),
        "atan2" => kestrel_rt_atan2f(a, b),
        "exp" => kestrel_rt_expf(a),
        "log" => kestrel_rt_logf(a),
        "pow" => kestrel_rt_powf(a, b),
        "fmod" => kestrel_rt_fmod_f32(a, b),
        _ => panic!("no function {name}"),
    }
}

/// What Kestrel computed before the wide functions: `libm`'s `f64` function on the widened inputs,
/// narrowed once, or `libm::fmodf` for `fmod`.
pub fn today(name: &str, a: f32, b: f32) -> f32 {
    if name == "fmod" {
        libm::fmodf(a, b)
    } else {
        reference(name, a, b) as f32
    }
}

/// The error of `got` in `f32` ulps of the true value `r`, taking the ulp from `r`'s exponent
/// (subnormal results use the smallest ulp). Infinite for a wrong special value: a NaN, an infinity,
/// or a zero whose sign differs from `r`'s where `r` rounds to a zero in `f32`.
pub fn ulps(got: f32, r: f64) -> f64 {
    if r.is_nan() || got.is_nan() {
        return if r.is_nan() && got.is_nan() { 0.0 } else { f64::INFINITY };
    }
    let rf = r as f32;
    if rf.is_infinite() || got.is_infinite() {
        // An infinite result is right when the true value rounds to it.
        return if rf == got { 0.0 } else { f64::INFINITY };
    }
    if got == 0.0 && rf == 0.0 && r.is_sign_negative() != got.is_sign_negative() {
        return f64::INFINITY;
    }
    let exponent = (((r.to_bits() >> 52) & 0x7FF) as i64 - 1023).max(-126) - 23;
    let ulp = f64::from_bits(((exponent + 1023) as u64) << 52);
    (got as f64 - r).abs() / ulp
}

/// Equal bits, or both NaN.
pub fn same(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}
