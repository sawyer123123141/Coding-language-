//! Kestrel's own `float` (`f32`) math: "wide" functions. Each widens its `f32` operands to `f64`
//! once, computes with `+ - * /`, `sqrt`, integer bit operations and exact rounding to an integer
//! (the `ROUND` add), and rounds to `f32` once. Those operations are correctly rounded IEEE
//! operations, so every build and every x86-64 CPU gives the same bits by construction, with no
//! table and no call into the C runtime.
//!
//! Each function has a fast path and a special test (`special`, spec decision 2): an integer test
//! on the bit patterns that sends NaN, infinities, huge arguments and the like to the slow path,
//! which is today's code (`libm` in `f64` on the widened operands, narrowed once, or
//! `libm::fmodf`), so special inputs keep today's bits. The compiler inlines the fast paths as
//! Cranelift IR: its helpers mirror the ones here one for one, in the same operation order, and read
//! these constants, never copies of them.

// The constants are written as their sources print them (musl's to 21 digits), and `INV_PIO2` is
// musl's `invpio2`, which equals `FRAC_2_PI`.
#![allow(clippy::excessive_precision, clippy::approx_constant)]

pub mod trig;
pub use trig::*;
pub use special::*;

/// `1.5 * 2^52`. For `|v| < 2^51`, `t = v + ROUND` rounds `v` to the nearest integer (ties to even)
/// and `k = t - ROUND` is that integer, exactly; `bits(t) - bits(ROUND)` is it as an `i64`.
pub const ROUND: f64 = 6755399441055744.0;

/// 53 bits of `2/π`, the first 25 bits of `π/2` and the rest of `π/2` (musl's `__rem_pio2f`,
/// medium size): for `|k| < 2^28`, `k * PIO2_1` is exact.
pub const INV_PIO2: f64 = 6.36619772367581382433e-01;
pub const PIO2_1: f64 = 1.57079631090164184570e+00;
pub const PIO2_1T: f64 = 1.58932547735281966916e-08;

/// `sin`, `cos` and `tan` take the fast path for `|x|` below these; each may be lowered on its own
/// if the reduction's worst cases above some magnitude miss `MAX_ULP`.
pub const SIN_FAST: f32 = 4.0e8;
pub const COS_FAST: f32 = 4.0e8;
pub const TAN_FAST: f32 = 4.0e8;

/// `2^-12`: below it `sin`, `tan`, `atan` and `asin` of `x` round to `x` itself.
pub const TINY: f32 = 2.4414062e-4;

/// Whether `tan` is `s / c` from the `sin` and `cos` polynomials (it met `MAX_ULP`); when false,
/// `tanf` is `libm::tan` in `f64` rounded once, and the compiler calls it.
pub const TAN_INLINE: bool = true;

/// The accuracy gate, in `f32` ulps of the true value.
pub const MAX_ULP: f64 = 0.51;

/// musl's `__sindf` (sin r on `|r| <= π/4`, relative error below `2^-37.5`).
pub const S1: f64 = -0.166666666416265235595;
pub const S2: f64 = 0.0083333293858894631756;
pub const S3: f64 = -0.000198393348360966317347;
pub const S4: f64 = 0.0000027183114939898219064;
/// musl's `__cosdf` (cos r on `|r| <= π/4`, error below `2^-34.1`).
pub const C0: f64 = -0.499999997251031003120;
pub const C1: f64 = 0.0416666233237390631894;
pub const C2: f64 = -0.00138867637746099294692;
pub const C3: f64 = 0.0000243904487962774090654;

pub const LOG2E: f64 = std::f64::consts::LOG2_E;
pub const LN2: f64 = std::f64::consts::LN_2;
pub const PI: f64 = std::f64::consts::PI;
pub const PI_2: f64 = std::f64::consts::FRAC_PI_2;
pub const PI_4: f64 = std::f64::consts::FRAC_PI_4;

/// `exp` clamps its argument to these: beyond them the `f32` result is 0 or infinity.
pub const EXP_LO: f64 = -104.0;
pub const EXP_HI: f64 = 89.0;

/// `exp(r)` on `|r| <= ln2/2`, near-minimax of degree 8 (relative error `2^-39.8`).
pub const E8: [f64; 9] = [
    1.0,
    0.9999999999797852,
    0.49999999999797934,
    0.16666666891045775,
    0.041666666890957,
    0.008333266097949614,
    0.0013888821677630362,
    0.00019915866926782682,
    2.4876164022625967e-05,
];
/// Degree 9 (relative error `2^-45.6`), for `pow`.
pub const E9: [f64; 10] = [
    1.0000000000000135,
    1.0000000000000013,
    0.4999999999943859,
    0.16666666666615648,
    0.04166666704055191,
    0.008333333367311603,
    0.0013888801749656543,
    0.0001984119064754424,
    2.4884459751751116e-05,
    2.7632640675430236e-06,
];

/// The bits of `sqrt(1/2)` in `f64`: `log_core` reduces its argument to `[sqrt(1/2), sqrt(2))`.
pub const SQRT_HALF_BITS: u64 = 0x3FE6_A09E_667F_3BCD;
/// `atanh(s)/s` as a polynomial in `s^2` for `s` in `[0, (sqrt2-1)/(sqrt2+1)]`, degree 5 (`2^-45.1`).
pub const L5: [f64; 6] = [
    0.9999999999999736,
    0.3333333333978963,
    0.1999999744591619,
    0.14286083072937864,
    0.11087124955038745,
    0.09804047819876527,
];

/// `atan(t)/t` as a polynomial in `t^2` on `|t| <= tan(π/8)`, degree 7 (`2^-40.3`).
pub const A7: [f64; 8] = [
    0.9999999999992447,
    -0.33333333276922494,
    0.19999993053555815,
    -0.14285386553752139,
    0.11103456908947464,
    -0.08992552906150912,
    0.06974197686595265,
    -0.03765510601291402,
];
pub const TAN_PI_8: f64 = 0.41421356237309503;

/// `2^29`: below it `q * b` and `a - q * b` are exact in `f64` for `f32` operands.
pub const FMOD_Q_LIMIT: f64 = 536870912.0;

/// `v` rounded to the nearest integer, as an `f64` and an `i64`, for `|v| < 2^51`. Exact
/// operations only: no float-to-int conversion.
#[inline(always)]
pub fn round_k(v: f64) -> (f64, i64) {
    let t = v + ROUND;
    (t - ROUND, (t.to_bits() as i64).wrapping_sub(ROUND.to_bits() as i64))
}

/// `c[0] + c[1] x + ...` by Estrin's scheme, in this fixed order, for 8, 9 or 10 coefficients.
#[inline(always)]
pub fn estrin<const N: usize>(c: &[f64; N], x: f64) -> f64 {
    let x2 = x * x;
    let x4 = x2 * x2;
    let p01 = c[0] + c[1] * x;
    let p23 = c[2] + c[3] * x;
    let p45 = c[4] + c[5] * x;
    let p67 = c[6] + c[7] * x;
    let q0 = p01 + p23 * x2;
    let q1 = p45 + p67 * x2;
    let s0 = q0 + q1 * x4;
    match N {
        8 => s0,
        9 => s0 + c[8] * (x4 * x4),
        10 => s0 + (c[8] + c[9] * x) * (x4 * x4),
        _ => panic!("estrin takes 8, 9 or 10 coefficients"),
    }
}

/// `e^z` for an `f32` result: clamp `z` to `[EXP_LO, EXP_HI]`, reduce by `k ln2`, evaluate `c` on
/// the rest, and scale by `2^k` (a normal `f64` for every `k` the clamps allow).
#[inline(always)]
pub fn exp_core<const N: usize>(z: f64, c: &[f64; N]) -> f64 {
    let z = if z < EXP_LO { EXP_LO } else { z };
    let z = if z > EXP_HI { EXP_HI } else { z };
    let (k, ki) = round_k(z * LOG2E);
    let r = z - k * LN2;
    estrin(c, r) * f64::from_bits(((ki + 1023) as u64) << 52)
}

/// `ln x` for a positive normal `f64` `x`: `x = 2^e m` with `m` in `[sqrt(1/2), sqrt(2))` from the
/// bits, then `ln m = 2 atanh(s)` with `s = (m-1)/(m+1)`.
#[inline(always)]
pub fn log_core(x: f64) -> f64 {
    let ix = x.to_bits().wrapping_add(0x3FF0_0000_0000_0000 - SQRT_HALF_BITS);
    let e = (ix >> 52) as i64 - 1023;
    let m = f64::from_bits((ix & 0x000F_FFFF_FFFF_FFFF) + SQRT_HALF_BITS);
    let f = m - 1.0;
    let s = f / (2.0 + f);
    let u = s * s;
    let g = L5[0] + u * (L5[1] + u * (L5[2] + u * (L5[3] + u * (L5[4] + u * L5[5]))));
    e as f64 * LN2 + 2.0 * s * g
}

pub fn expf(x: f32) -> f32 {
    if exp_special(x) {
        return libm::exp(x as f64) as f32;
    }
    exp_core(x as f64, &E8) as f32
}

pub fn logf(x: f32) -> f32 {
    if log_special(x) {
        return libm::log(x as f64) as f32;
    }
    log_core(x as f64) as f32
}

pub fn powf(x: f32, y: f32) -> f32 {
    if pow_special(x, y) {
        return libm::pow(x as f64, y as f64) as f32;
    }
    exp_core(y as f64 * log_core(x as f64), &E9) as f32
}

/// The exact remainder `a - trunc(a/b) b`, like `fmodf`, through `f64`: for a quotient below
/// `2^29` the product and the difference are exact, and one step fixes a quotient that the
/// division rounded to the next integer.
pub fn fmodf(a: f32, b: f32) -> f32 {
    let (ad, bd) = (a as f64, b as f64);
    let q = libm::trunc(ad / bd);
    if fmod_special(a, b, q) {
        return libm::fmodf(a, b);
    }
    let r = ad - q * bd;
    let ab = bd.abs().copysign(ad);
    let fix = r != 0.0 && (r < 0.0) != (ad < 0.0);
    let over = !fix && r.abs() >= bd.abs();
    let r = if fix {
        r + ab
    } else if over {
        r - ab
    } else {
        r
    };
    if r == 0.0 {
        0f32.copysign(a)
    } else {
        r as f32
    }
}

/// The special tests of spec decision 2: integer tests on the bit patterns, which the inline code
/// emits as the same integer operations, so both sides classify every input alike under any float
/// mode. Each is true for NaN.
pub mod special {
    use super::{FMOD_Q_LIMIT, SIN_FAST, COS_FAST, TAN_FAST, TINY};

    /// Clears the sign bit of an `f32`'s bits.
    pub const ABS_MASK: u32 = 0x7FFF_FFFF;
    /// Clears the sign bit of an `f64`'s bits.
    pub const ABS_MASK_64: u64 = 0x7FFF_FFFF_FFFF_FFFF;
    /// The bits of `f32` infinity: a magnitude at or above it is infinite or NaN.
    pub const INF_BITS: u32 = 0x7F80_0000;
    /// The bits of `1.0f32`.
    pub const ONE_BITS: u32 = 0x3F80_0000;
    /// `bits(x) - 1` (wrapping) at or above this: `x` is zero, negative, infinite or NaN.
    pub const NOT_POSITIVE_FINITE: u32 = 0x7F7F_FFFF;

    #[inline(always)]
    pub fn magnitude(x: f32) -> u32 {
        x.to_bits() & ABS_MASK
    }

    #[inline(always)]
    pub fn sin_special(x: f32) -> bool {
        magnitude(x) >= SIN_FAST.to_bits()
    }

    #[inline(always)]
    pub fn cos_special(x: f32) -> bool {
        magnitude(x) >= COS_FAST.to_bits()
    }

    #[inline(always)]
    pub fn tan_special(x: f32) -> bool {
        magnitude(x) >= TAN_FAST.to_bits()
    }

    /// NaN only: `exp(±inf)` stays on the fast path, through the clamps.
    #[inline(always)]
    pub fn exp_special(x: f32) -> bool {
        magnitude(x) > INF_BITS
    }

    #[inline(always)]
    pub fn log_special(x: f32) -> bool {
        x.to_bits().wrapping_sub(1) >= NOT_POSITIVE_FINITE
    }

    #[inline(always)]
    pub fn atan_special(x: f32) -> bool {
        magnitude(x) >= INF_BITS
    }

    /// `|x| > 1` or NaN; `acos` uses it too.
    #[inline(always)]
    pub fn asin_special(x: f32) -> bool {
        magnitude(x) > ONE_BITS
    }

    /// Either operand infinite or NaN, or both zero.
    #[inline(always)]
    pub fn atan2_special(y: f32, x: f32) -> bool {
        let (my, mx) = (magnitude(y), magnitude(x));
        my >= INF_BITS || mx >= INF_BITS || (my | mx) == 0
    }

    /// `x` not positive and finite, or `y` infinite or NaN.
    #[inline(always)]
    pub fn pow_special(x: f32, y: f32) -> bool {
        log_special(x) || magnitude(y) >= INF_BITS
    }

    /// `a` infinite or NaN, `b` zero, infinite or NaN, or the quotient `q = trunc(a/b)` (in `f64`)
    /// at or above `FMOD_Q_LIMIT` in magnitude, or NaN.
    #[inline(always)]
    pub fn fmod_special(a: f32, b: f32, q: f64) -> bool {
        let mb = magnitude(b);
        magnitude(a) >= INF_BITS || mb == 0 || mb >= INF_BITS || (q.to_bits() & ABS_MASK_64) >= FMOD_Q_LIMIT.to_bits()
    }

    /// `|x| < TINY`.
    #[inline(always)]
    pub fn tiny(x: f32) -> bool {
        magnitude(x) < TINY.to_bits()
    }
}
