//! Ranges of integer values, and what each operator does to them (the spec's "Ranges of
//! expressions" table). Every computation is in `i128`: an operand's range is always inside its
//! type (at most 64 bits), so sums and products of two of them can't overflow, and a shift's
//! amount is brought into `0..width` before it is used.

use crate::ir::KTy;

/// The values from `lo` to `hi`, both included (`lo <= hi`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub lo: i128,
    pub hi: i128,
}

/// The number of bits of an integer type (a `bool` counts as 8, as codegen stores it).
pub fn width(ty: KTy) -> u32 {
    match ty {
        KTy::Bool | KTy::U8 => 8,
        KTy::I32 => 32,
        _ => 64,
    }
}

impl Interval {
    pub fn new(lo: i128, hi: i128) -> Self {
        debug_assert!(lo <= hi, "empty range {lo}..{hi}");
        Interval { lo, hi }
    }

    pub fn exact(v: i128) -> Self {
        Interval { lo: v, hi: v }
    }

    /// Every value of `ty`: `u8` is 0 to 255, `int` 32 bits, `i64` 64 bits, `bool` 0 to 1. Any
    /// other type (only asked about by mistake) gets the `i64` range.
    pub fn of_type(ty: KTy) -> Self {
        match ty {
            KTy::Bool => Interval::new(0, 1),
            KTy::U8 => Interval::new(0, 255),
            KTy::I32 => Interval::new(i32::MIN.into(), i32::MAX.into()),
            _ => Interval::new(i64::MIN.into(), i64::MAX.into()),
        }
    }

    pub fn contains(self, v: i128) -> bool {
        self.lo <= v && v <= self.hi
    }

    /// Whether every value fits `ty`.
    pub fn fits(self, ty: KTy) -> bool {
        let t = Interval::of_type(ty);
        t.lo <= self.lo && self.hi <= t.hi
    }

    /// The values in both, or `None` when there are none.
    pub fn meet(self, other: Interval) -> Option<Interval> {
        let (lo, hi) = (self.lo.max(other.lo), self.hi.min(other.hi));
        (lo <= hi).then_some(Interval { lo, hi })
    }

    /// The part of the range inside `ty`. When none of it is (the value never fits, so whatever
    /// needed it to stopped the program), the type's whole range: any answer is sound there.
    pub fn clip(self, ty: KTy) -> Interval {
        self.meet(Interval::of_type(ty)).unwrap_or(Interval::of_type(ty))
    }

    /// The smallest range holding both.
    pub fn join(self, other: Interval) -> Interval {
        Interval { lo: self.lo.min(other.lo), hi: self.hi.max(other.hi) }
    }

    /// `self` (the range at a loop's top so far) widened by `new` (the next round's): an end that
    /// is still growing jumps to the type's limit, so a loop's repeats end after a few rounds. The
    /// result holds both ranges.
    pub fn widen(self, new: Interval, ty: KTy) -> Interval {
        let t = Interval::of_type(ty);
        Interval {
            lo: if new.lo < self.lo { t.lo.min(new.lo) } else { self.lo },
            hi: if new.hi > self.hi { t.hi.max(new.hi) } else { self.hi },
        }
    }
}

/// The result of an operation whose exact result is `exact`, after its check (if it has one):
/// with checks on, a result outside the type stopped the program, so it is the exact range clipped
/// to the type; otherwise (`--release`, or an operation with no overflow check) a result that
/// doesn't fit wrapped, so it is any value of the type.
pub fn settle(exact: Interval, ty: KTy, checked: bool) -> Interval {
    if exact.fits(ty) {
        exact
    } else if checked {
        exact.clip(ty)
    } else {
        Interval::of_type(ty)
    }
}

pub fn add(a: Interval, b: Interval) -> Interval {
    Interval::new(a.lo + b.lo, a.hi + b.hi)
}

pub fn sub(a: Interval, b: Interval) -> Interval {
    Interval::new(a.lo - b.hi, a.hi - b.lo)
}

pub fn neg(a: Interval) -> Interval {
    Interval::new(-a.hi, -a.lo)
}

/// The smallest range holding the four corners of `f` over `a` and `b`: right for an `f` that is
/// monotonic in each operand separately.
fn corners(a: Interval, b: Interval, f: impl Fn(i128, i128) -> i128) -> Interval {
    let vs = [f(a.lo, b.lo), f(a.lo, b.hi), f(a.hi, b.lo), f(a.hi, b.hi)];
    Interval::new(*vs.iter().min().unwrap(), *vs.iter().max().unwrap())
}

pub fn mul(a: Interval, b: Interval) -> Interval {
    corners(a, b, |x, y| x * y)
}

/// The divisor's values other than 0, as up to two ranges of one sign each (none when it is
/// always 0: the zero check always fails).
fn nonzero_parts(b: Interval) -> Vec<Interval> {
    let mut parts = Vec::new();
    if b.lo < 0 {
        parts.push(Interval::new(b.lo, b.hi.min(-1)));
    }
    if b.hi > 0 {
        parts.push(Interval::new(b.lo.max(1), b.hi));
    }
    parts
}

/// `a / b` (truncating, as `sdiv` and `udiv` do), over every divisor but 0 (the zero check stops
/// the program before a division by 0). The minimum divided by -1 gives its exact value (one past
/// the maximum), which `settle` clips (checked) or turns into the type's range (wrapping, where
/// codegen divides by 1 instead and gives the minimum). `ty` gives the range when the divisor is
/// always 0.
pub fn div(a: Interval, b: Interval, ty: KTy) -> Interval {
    // For a divisor of one sign, `a / b` is monotonic in each operand, so the corners bound it.
    let parts = nonzero_parts(b);
    let mut it = parts.into_iter().map(|p| corners(a, p, |x, y| x / y));
    match it.next() {
        Some(first) => it.fold(first, Interval::join),
        None => Interval::of_type(ty),
    }
}

/// `a % b`: smaller in size than the divisor's largest size, no larger than the dividend, with the
/// dividend's sign (as `srem` and `urem` give it).
pub fn rem(a: Interval, b: Interval, ty: KTy) -> Interval {
    let largest = nonzero_parts(b).iter().map(|p| p.lo.abs().max(p.hi.abs())).max();
    let Some(largest) = largest else { return Interval::of_type(ty) };
    let lo = if a.lo < 0 { -(-a.lo).min(largest - 1) } else { 0 };
    let hi = if a.hi > 0 { a.hi.min(largest - 1) } else { 0 };
    Interval::new(lo, hi)
}

/// The amounts a shift of a `ty` value can really shift by: `n` when it is inside `0..width`;
/// otherwise, with the check on, the part of it inside (the check stopped the rest), and without
/// it any amount in `0..width` (the machine takes the amount modulo the width).
pub fn shift_amount(n: Interval, ty: KTy, checked: bool) -> Interval {
    let ok = Interval::new(0, i128::from(width(ty)) - 1);
    if n.fits_in(ok) {
        n
    } else if checked {
        n.meet(ok).unwrap_or(ok)
    } else {
        ok
    }
}

impl Interval {
    fn fits_in(self, outer: Interval) -> bool {
        outer.lo <= self.lo && self.hi <= outer.hi
    }
}

/// `a << n` for an amount inside `0..width`: exact when every result fits the type, otherwise the
/// type's range (a left shift wraps in both builds: it has no overflow check).
pub fn shl(a: Interval, n: Interval, ty: KTy) -> Interval {
    let r = corners(a, n, |x, k| x << k);
    if r.fits(ty) {
        r
    } else {
        Interval::of_type(ty)
    }
}

/// `a >> n` for an amount inside `0..width` (arithmetic for signed types; an unsigned value is
/// never negative, so the same formula holds).
pub fn shr(a: Interval, n: Interval) -> Interval {
    corners(a, n, |x, k| x >> k)
}

/// `a & b`: 0 to the smaller high end of the sides that are 0 or more, when one side is.
pub fn bit_and(a: Interval, b: Interval, ty: KTy) -> Interval {
    match (a.lo >= 0, b.lo >= 0) {
        _ if a.lo == a.hi && b.lo == b.hi => Interval::exact(a.lo & b.lo),
        (true, true) => Interval::new(0, a.hi.min(b.hi)),
        (true, false) => Interval::new(0, a.hi),
        (false, true) => Interval::new(0, b.hi),
        (false, false) => Interval::of_type(ty),
    }
}

/// `a | b` and `a ^ b`: 0 to the next power of two above both high ends, minus 1, when both sides
/// are 0 or more.
pub fn bit_or_xor(a: Interval, b: Interval, ty: KTy, xor: bool) -> Interval {
    if a.lo == a.hi && b.lo == b.hi {
        return Interval::exact(if xor { a.lo ^ b.lo } else { a.lo | b.lo });
    }
    if a.lo < 0 || b.lo < 0 {
        return Interval::of_type(ty);
    }
    let high = a.hi.max(b.hi) as u128;
    let above = (high + 1).next_power_of_two();
    Interval::new(0, (above - 1) as i128)
}

/// `~a`: every bit flipped, which for a signed value is `-a - 1` and for a `u8` is `255 - a`.
pub fn bit_not(a: Interval, ty: KTy) -> Interval {
    if ty.is_signed() {
        Interval::new(-a.hi - 1, -a.lo - 1)
    } else {
        let top = Interval::of_type(ty).hi;
        Interval::new(top - a.hi, top - a.lo)
    }
}

/// `rand_int(a, b)`: from the lower low end to the higher high end (the runtime swaps reversed
/// bounds); the type's range when that doesn't fit the result, which codegen narrows.
pub fn rand_int(a: Interval, b: Interval, ty: KTy) -> Interval {
    let r = a.join(b);
    if r.fits(ty) {
        r
    } else {
        Interval::of_type(ty)
    }
}

/// Whether a division of `a` by `b` (both of type `ty`) can't fail: the divisor is never 0, and
/// (signed) the dividend is never the minimum when the divisor can be -1.
pub fn division_safe(a: Interval, b: Interval, ty: KTy) -> bool {
    let min = Interval::of_type(ty).lo;
    !b.contains(0) && !(ty.is_signed() && a.contains(min) && b.contains(-1))
}

/// Whether a shift of a `ty` value by `n` can't fail its check: `n` is 0 to the width minus 1.
pub fn shift_safe(n: Interval, ty: KTy) -> bool {
    n.lo >= 0 && n.hi < i128::from(width(ty))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iv(lo: i128, hi: i128) -> Interval {
        Interval::new(lo, hi)
    }

    #[test]
    fn division_bounds_hold_at_every_corner() {
        assert_eq!(div(iv(-7, 7), iv(2, 3), KTy::I32), iv(-3, 3));
        assert_eq!(div(iv(-7, 7), iv(-2, 3), KTy::I32), iv(-7, 7));
        assert_eq!(div(iv(5, 5), iv(0, 0), KTy::I32), Interval::of_type(KTy::I32));
        let min = i128::from(i32::MIN);
        // The minimum by -1 is one past the maximum; settling turns that into the type's range.
        assert_eq!(div(iv(min, min), iv(-1, -1), KTy::I32).hi, -min);
        assert_eq!(settle(div(iv(min, min), iv(-1, -1), KTy::I32), KTy::I32, false), Interval::of_type(KTy::I32));
    }

    #[test]
    fn remainders_take_the_dividends_sign() {
        assert_eq!(rem(iv(0, 100), iv(1, 10), KTy::I32), iv(0, 9));
        assert_eq!(rem(iv(-100, 5), iv(-10, 10), KTy::I32), iv(-9, 5));
        assert_eq!(rem(iv(-3, -1), iv(7, 7), KTy::I32), iv(-3, 0));
    }

    #[test]
    fn shifts_bring_the_amount_into_the_width_first() {
        assert_eq!(shift_amount(iv(-1, 3), KTy::I32, true), iv(0, 3));
        assert_eq!(shift_amount(iv(-1, 3), KTy::I32, false), iv(0, 31));
        assert_eq!(shl(iv(1, 1), iv(0, 31), KTy::I32), Interval::of_type(KTy::I32));
        assert_eq!(shl(iv(1, 1), iv(5, 5), KTy::I32), iv(32, 32));
        assert_eq!(shr(iv(-8, 8), iv(1, 2)), iv(-4, 4));
    }

    #[test]
    fn bit_operations_follow_the_table() {
        assert_eq!(bit_and(iv(i64::MIN.into(), i64::MAX.into()), iv(0x7FFF_FFFF, 0x7FFF_FFFF), KTy::I64), iv(0, 0x7FFF_FFFF));
        assert_eq!(bit_or_xor(iv(0, 5), iv(0, 8), KTy::I32, true), iv(0, 15));
        assert_eq!(bit_or_xor(iv(0, 7), iv(0, 0), KTy::I32, false), iv(0, 7));
        assert_eq!(bit_not(iv(0, 10), KTy::I32), iv(-11, -1));
        assert_eq!(bit_not(iv(0, 10), KTy::U8), iv(245, 255));
    }

    #[test]
    fn widening_jumps_a_growing_end_to_the_limit() {
        let t = Interval::of_type(KTy::I32);
        assert_eq!(iv(0, 0).widen(iv(0, 1), KTy::I32), iv(0, t.hi));
        assert_eq!(iv(0, 5).widen(iv(1, 3), KTy::I32), iv(0, 5));
        assert_eq!(iv(0, 5).widen(iv(-1, 3), KTy::I32), iv(t.lo, 5));
    }
}
