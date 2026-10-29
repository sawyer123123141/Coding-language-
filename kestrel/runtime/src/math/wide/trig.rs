//! The wide trigonometric functions: `sin`, `cos` and `tan` from one range reduction and one pair
//! of polynomials, and `atan2`, `atan`, `asin` and `acos` from one arctangent core.

use super::special::*;
use super::*;

/// `x = k π/2 + r` with `|r|` at most about `π/4` (musl's `__rem_pio2f`, medium size): `(r, k)`.
/// `k * PIO2_1` is exact for `|x|` below `2^28 π/2`.
#[inline(always)]
pub fn reduce(x: f32) -> (f64, i64) {
    let xd = x as f64;
    let (k, ki) = round_k(xd * INV_PIO2);
    ((xd - k * PIO2_1) - k * PIO2_1T, ki)
}

/// `(sin r, cos r)` on `|r| <= π/4` (musl's `__sindf` and `__cosdf`).
#[inline(always)]
pub fn sin_cos_poly(r: f64) -> (f64, f64) {
    let z = r * r;
    let w = z * z;
    let s = z * r;
    let sin = (r + s * (S1 + z * S2)) + (s * w) * (S3 + z * S4);
    let cos = ((1.0 + z * C0) + w * C1) + (w * z) * (C2 + z * C3);
    (sin, cos)
}

/// `v` with its sign flipped when `k & 2` is set (the quadrants where sin and cos are negated).
#[inline(always)]
pub fn flip(v: f64, k: i64) -> f64 {
    f64::from_bits(v.to_bits() ^ (((k & 2) as u64) << 62))
}

pub fn sinf(x: f32) -> f32 {
    if sin_special(x) {
        return libm::sin(x as f64) as f32;
    }
    if tiny(x) {
        return x;
    }
    let (r, k) = reduce(x);
    let (s, c) = sin_cos_poly(r);
    flip(if k & 1 == 0 { s } else { c }, k) as f32
}

pub fn cosf(x: f32) -> f32 {
    if cos_special(x) {
        return libm::cos(x as f64) as f32;
    }
    let (r, k) = reduce(x);
    let (s, c) = sin_cos_poly(r);
    let k = k + 1;
    flip(if k & 1 == 0 { s } else { c }, k) as f32
}

pub fn tanf(x: f32) -> f32 {
    if !TAN_INLINE || tan_special(x) {
        return libm::tan(x as f64) as f32;
    }
    if tiny(x) {
        return x;
    }
    let (r, k) = reduce(x);
    let (s, c) = sin_cos_poly(r);
    let (n, d) = if k & 1 == 0 { (s, c) } else { (-c, s) };
    (n / d) as f32
}

/// `atan2(ay, ax)` for `ay, ax >= 0` (not both zero, both finite), or `π` minus it when
/// `x_negative`: swap so that the ratio is at most 1, reduce ratios above `tan(π/8)` by `π/4`, then
/// `t p(t^2)` with one division.
#[inline(always)]
pub fn atan_core(ay: f64, ax: f64, x_negative: bool) -> f64 {
    let swap = ay > ax;
    let num = if swap { ax } else { ay };
    let den = if swap { ay } else { ax };
    let big = num > TAN_PI_8 * den;
    let tn = if big { num - den } else { num };
    let td = if big { num + den } else { den };
    let t = tn / td;
    let a = t * estrin(&A7, t * t);
    let a = if big { PI_4 + a } else { a };
    let a = if swap { PI_2 - a } else { a };
    if x_negative {
        PI - a
    } else {
        a
    }
}

pub fn atan2f(y: f32, x: f32) -> f32 {
    if atan2_special(y, x) {
        return libm::atan2(y as f64, x as f64) as f32;
    }
    (atan_core(y.abs() as f64, x.abs() as f64, x.is_sign_negative()) as f32).copysign(y)
}

pub fn atanf(x: f32) -> f32 {
    if atan_special(x) {
        return libm::atan(x as f64) as f32;
    }
    if tiny(x) {
        return x;
    }
    (atan_core(x.abs() as f64, 1.0, false) as f32).copysign(x)
}

/// `asin x = atan2(x, sqrt((1-x)(1+x)))`.
pub fn asinf(x: f32) -> f32 {
    if asin_special(x) {
        return libm::asin(x as f64) as f32;
    }
    if tiny(x) {
        return x;
    }
    let a = x.abs() as f64;
    let c = ((1.0 - a) * (1.0 + a)).sqrt();
    (atan_core(a, c, false) as f32).copysign(x)
}

/// `acos x = atan2(sqrt((1-x)(1+x)), x)`, in `[0, π]`.
pub fn acosf(x: f32) -> f32 {
    if asin_special(x) {
        return libm::acos(x as f64) as f32;
    }
    let a = x.abs() as f64;
    let c = ((1.0 - a) * (1.0 + a)).sqrt();
    atan_core(c, a, x.is_sign_negative()) as f32
}
