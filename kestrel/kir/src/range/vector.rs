//! Vectors in range analysis: the forms that make and take apart a SIMD vector (`VecLit`, `Splat`,
//! `Shuffle`, `LaneOf`), the `ivec4` operations, lane loads and stores, and how the report writes
//! them (spec R16-R22, R25-R26).
//!
//! An `ivec4` has **one range for all four lanes**, the widest: a make is the hull of its lanes, a
//! fill its number's range, a swizzle or a lane read its vector's range, and writing one lane joins
//! the old range with the new lane's (one lane changes, three stay). The operators use the scalar
//! rules for `int` on those ranges, which hold lane by lane: every interval helper is called with
//! `LANE` (`int`), never with the vector type (whose width and signedness the helpers don't know).
//! A vector check is one site for all four lanes, removed only when the range proves every lane.
//!
//! What a vector never is: a fact term or the owner of a linear form (M13; `Val::range` only), so
//! no `a - b <= c` names one; and a condition on one of its lanes teaches nothing about it (a lane
//! load has no linear form and `terms::local_value` doesn't follow a `Lane` step), since `v.x < 10`
//! says nothing about `v.y`. A float vector has no checks and isn't analysed.

use super::bounds::Val;
use super::interval::{self as iv, Interval};
use super::show::{expr_text, range_text, write_expr, write_operand};
use super::sites::CheckKind;
use super::state::State;
use super::Cx;
use crate::ir::*;

/// The lane names, in order.
const LANES: [&str; 4] = ["x", "y", "z", "w"];

/// The type every interval helper is asked about for an `ivec4` (R26).
const LANE: KTy = KTy::I32;

/// A vector type's name as Kestrel writes it.
pub fn vector_name(ty: KTy) -> &'static str {
    match ty {
        KTy::I32X4 => "ivec4",
        _ => "vec4",
    }
}

/// The name of lane `i`: `x`, `y`, `z` or `w`.
pub fn lane_name(i: u8) -> &'static str {
    LANES[i as usize]
}

impl Cx<'_> {
    /// A vector form, deciding the checks in its parts: a make is the hull of its lanes, a fill
    /// its number's range, a swizzle or a lane of a value its vector's range.
    pub(super) fn vector(&mut self, st: &State, e: &Expr, sure: bool) -> Val {
        let r = match &e.kind {
            ExprKind::VecLit(xs) => {
                let mut hull = Some(None);
                for x in xs {
                    let r = self.lanes(st, x, sure);
                    hull = match (hull, r) {
                        (Some(h), Some(r)) => Some(Some(h.map_or(r, |h: Interval| h.join(r)))),
                        _ => None,
                    };
                }
                hull.flatten()
            }
            ExprKind::Splat(x) | ExprKind::Shuffle(x, _) | ExprKind::LaneOf(x, _) => self.lanes(st, x, sure),
            _ => unreachable!("not a vector form"),
        };
        Val::range(int_lanes(e.ty, r))
    }

    /// The range of `x`'s lanes when they are `int`s (`x` an `int` or an `ivec4`), deciding the
    /// checks in it; `None` for any other value (a float, a `u8`).
    fn lanes(&mut self, st: &State, x: &Expr, sure: bool) -> Option<Interval> {
        let r = self.eval(st, x, sure);
        (x.ty.lane() == LANE).then_some(r)
    }

    /// A load of a lane of a tracked `ivec4` local (`v.x`): the vector's range, and no linear
    /// form. `None` for any other load.
    pub(super) fn lane_load(&self, st: &State, p: &Place) -> Option<Val> {
        if !is_lane_place(p) || self.f.locals[p.root.0 as usize].ty != KTy::I32X4 {
            return None;
        }
        st.get(&self.t, p.root).map(Val::range)
    }

    /// Writing `value` to one lane of the tracked `ivec4` local `l`: its range joins the old one
    /// (one lane changes, three stay; R16). Without a value, it can be anything.
    pub(super) fn lane_store(&self, st: &mut State, l: LocalIdx, value: Option<Val>) {
        match (st.get(&self.t, l), value) {
            (Some(old), Some(v)) => st.set(&self.t, l, old.join(v.r)),
            _ => st.forget(&self.t, l),
        }
    }

    /// An `ivec4` operation (`is_int_vector_op`): its operands evaluated, its check (if this build
    /// has one) decided by the operands' ranges unless `already` proven, and its range by the
    /// scalar rules for `int`. Never a linear form; a comparison's value is any `bool`, and a
    /// conversion between vectors (`ivec4(v)` saturates a float) any value of its lane type.
    pub(super) fn vector_arith(&mut self, st: &State, e: &Expr, sure: bool, already: bool) -> Val {
        let checked = self.arith.checked();
        let r = match &e.kind {
            ExprKind::Unary(UnOp::Neg, x) => {
                let a = self.eval(st, x, sure);
                let exact = iv::neg(a);
                self.site(e, CheckKind::Overflow, exact.fits(LANE), already, &[(x, a)]);
                iv::settle(exact, LANE, checked)
            }
            ExprKind::Unary(UnOp::BitNot, x) => iv::bit_not(self.eval(st, x, sure), LANE),
            ExprKind::Binary(op, l, r) => {
                let (a, b) = (self.eval(st, l, sure), self.eval(st, r, sure));
                if op.is_comparison() {
                    return Val::range(Interval::of_type(e.ty));
                }
                self.lane_binary(e, *op, (l, a), (r, b), sure, already)
            }
            ExprKind::Cast(x) => {
                self.eval(st, x, sure);
                return Val::range(Interval::of_type(e.ty.lane()));
            }
            _ => unreachable!("not an `ivec4` operation"),
        };
        Val::range(r)
    }

    /// `l op r` on `ivec4` lanes in `a` and `b` (a shift's `r` is its one `int` amount).
    fn lane_binary(&mut self, e: &Expr, op: BinOp, (l, a): (&Expr, Interval), (r, b): (&Expr, Interval), sure: bool, already: bool) -> Interval {
        let checked = self.arith.checked();
        let operands = [(l, a), (r, b)];
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::AddWrap | BinOp::SubWrap | BinOp::MulWrap => {
                // `*% +% -%` have no check and wrap in every build: a result outside `int` is any.
                let wrapping = op.is_wrapping();
                let exact = match op {
                    BinOp::Add | BinOp::AddWrap => iv::add(a, b),
                    BinOp::Sub | BinOp::SubWrap => iv::sub(a, b),
                    _ => iv::mul(a, b),
                };
                if !wrapping {
                    self.site(e, CheckKind::Overflow, exact.fits(LANE), already, &operands);
                }
                iv::settle(exact, LANE, checked && !wrapping)
            }
            BinOp::Shl | BinOp::Shr => {
                // One `int` amount for every lane, checked as an `int` shift's: 0 to 31.
                if checked {
                    self.site(e, CheckKind::Shift, iv::shift_safe(b, LANE), already, &operands);
                    if sure && !already {
                        self.learn(r, Some(Interval::new(0, i128::from(iv::width(LANE)) - 1)));
                    }
                }
                let n = iv::shift_amount(b, LANE, checked);
                if op == BinOp::Shl {
                    iv::shl(a, n, LANE)
                } else {
                    // Arithmetic, as `sshr` is (R8).
                    iv::shr(a, n)
                }
            }
            BinOp::BitAnd => iv::bit_and(a, b, LANE),
            BinOp::BitOr | BinOp::BitXor => iv::bit_or_xor(a, b, LANE, op == BinOp::BitXor),
            // `ivec4` has no `/` or `%`.
            _ => Interval::of_type(LANE),
        }
    }

    /// `min` or `max` of `ivec4`s: lane by lane, so the ends of the two ranges.
    pub(super) fn vector_math(&mut self, st: &State, e: &Expr, sure: bool) -> Val {
        let ExprKind::Math(op, xs) = &e.kind else { unreachable!("not a math call") };
        let rs: Vec<Option<Interval>> = xs.iter().map(|x| self.lanes(st, x, sure)).collect();
        let r = match (op, rs.as_slice()) {
            (MathOp::Min | MathOp::Max, [Some(a), Some(b)]) => Some(min_max(*op == MathOp::Max, *a, *b)),
            _ => None,
        };
        Val::range(int_lanes(e.ty, r))
    }
}

/// The range of a value of type `ty` (a vector, or a lane of one) whose `int` lanes are in `r`:
/// `r` when the lanes are `int`s and known, else any value of the lane type.
fn int_lanes(ty: KTy, r: Option<Interval>) -> Interval {
    match r {
        Some(r) if ty.lane() == LANE => r.clip(LANE),
        _ => Interval::of_type(ty.lane()),
    }
}

/// `min(a, b)` (or `max`) of values in `a` and `b`.
fn min_max(max: bool, a: Interval, b: Interval) -> Interval {
    if max {
        Interval::new(a.lo.max(b.lo), a.hi.max(b.hi))
    } else {
        Interval::new(a.lo.min(b.lo), a.hi.min(b.hi))
    }
}

/// Whether a place is a lane of its root (`v.x`): a `Lane` is only ever a place's last step, and
/// a vector local's lane is its only one.
pub(super) fn is_lane_place(p: &Place) -> bool {
    matches!(p.path[..], [Proj::Lane(_)])
}

/// Whether `e` is an `ivec4` operation that arithmetic analysis sends here: `-v`, `~v`, any
/// binary operation on `ivec4`s (a shift's amount is a plain `int`), or a conversion to or from a
/// vector.
pub(super) fn is_int_vector_op(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Unary(UnOp::Neg | UnOp::BitNot, _) => e.ty == KTy::I32X4,
        ExprKind::Binary(_, l, _) => l.ty == KTy::I32X4,
        ExprKind::Cast(x) => e.ty.is_vector() || x.ty.is_vector(),
        _ => false,
    }
}

/// The evidence for an operand `x` of a vector check whose range is `r`: "the lanes of `a` are 0 to
/// 99" for a vector (one range for its four lanes), the scalar phrase for a shift's amount.
pub(super) fn lanes_evidence(m: &Module, f: &Func, x: &Expr, r: Interval) -> String {
    let text = expr_text(m, f, x);
    if !x.ty.is_vector() {
        return format!("`{text}` {}", range_text(r, x.ty));
    }
    let phrase = range_text(r, x.ty.lane());
    let phrase = match phrase.strip_prefix("is ") {
        Some(rest) => format!("are {rest}"),
        None => phrase,
    };
    format!("the lanes of `{text}` {phrase}")
}

/// Whether `x` is a literal as an operand of a check: a number, or a number filling every lane.
pub(super) fn is_literal(x: &Expr) -> bool {
    match &x.kind {
        ExprKind::Int(_) => true,
        ExprKind::Splat(n) => matches!(n.kind, ExprKind::Int(_)),
        _ => false,
    }
}

/// A vector form in Kestrel syntax: `vec4(a, b, c, d)`, `vec4(n)` for a `Splat`, `v.wzyx`, `e.x`.
pub(super) fn write_vector(m: &Module, f: &Func, e: &Expr, s: &mut String) {
    match &e.kind {
        ExprKind::VecLit(xs) => {
            s.push_str(vector_name(e.ty));
            s.push('(');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                write_expr(m, f, x, s);
            }
            s.push(')');
        }
        ExprKind::Splat(x) => {
            s.push_str(vector_name(e.ty));
            s.push('(');
            write_expr(m, f, x, s);
            s.push(')');
        }
        ExprKind::Shuffle(x, lanes) => {
            write_operand(m, f, x, s);
            s.push('.');
            lanes.iter().for_each(|&i| s.push_str(lane_name(i)));
        }
        ExprKind::LaneOf(x, i) => {
            write_operand(m, f, x, s);
            s.push('.');
            s.push_str(lane_name(*i));
        }
        _ => unreachable!("not a vector form"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_and_max_take_the_ends() {
        let (a, b) = (Interval::new(-5, 10), Interval::new(0, 3));
        assert_eq!(min_max(false, a, b), Interval::new(-5, 3));
        assert_eq!(min_max(true, a, b), Interval::new(0, 10));
    }

    #[test]
    fn a_lane_range_is_an_ints() {
        assert_eq!(int_lanes(KTy::I32X4, Some(Interval::new(0, 7))), Interval::new(0, 7));
        assert_eq!(int_lanes(KTy::I32, Some(Interval::new(0, 7))), Interval::new(0, 7));
        assert_eq!(int_lanes(KTy::I32X4, None), Interval::of_type(KTy::I32));
        assert_eq!(int_lanes(KTy::F32X4, Some(Interval::new(0, 7))), Interval::of_type(KTy::F32));
    }
}
