//! Integer arithmetic: `-x`, the binary operators and conversions, each with its range, its
//! check decided, and its linear form. A linear form survives `+` and `-` of a constant and a
//! conversion when the result is exact: proven, or checked with checks on (with `--release`, an
//! operation that isn't proven wraps, so its result has no linear form).

use super::bounds::Val;
use super::interval::{self as iv, Interval};
use super::sites::{self, CheckKind};
use super::state::State;
use super::{expr_key, Cx, Learn};
use crate::ir::*;

impl Cx<'_> {
    /// An integer `-x`, binary operation or conversion: its value, and its check decided (unless
    /// `already` proven: a module analysed before).
    pub(super) fn arith(&mut self, st: &State, e: &Expr, sure: bool, already: bool) -> Val {
        match &e.kind {
            ExprKind::Unary(UnOp::Neg, x) => {
                let a = self.eval(st, x, sure);
                if !e.ty.is_int() {
                    return Val::range(Interval::of_type(e.ty));
                }
                let exact = iv::neg(a);
                self.site(e, CheckKind::Overflow, exact.fits(e.ty), already, &[(x, a)]);
                Val::range(iv::settle(exact, e.ty, self.arith.checked()))
            }
            ExprKind::Binary(op, l, r) => self.binary(st, e, *op, (l, r), sure, already),
            ExprKind::Cast(x) => self.cast(st, e, x, sure, already),
            _ => self.eval_v(st, e, sure),
        }
    }

    #[inline(never)]
    fn binary(&mut self, st: &State, e: &Expr, op: BinOp, (l, r): (&Expr, &Expr), sure: bool, already: bool) -> Val {
        let checked = self.arith.checked();
        let (va, vb) = (self.eval_v(st, l, sure), self.eval_v(st, r, sure));
        let (a, b) = (va.r, vb.r);
        let ty = l.ty;
        if !ty.is_int() || op.is_comparison() {
            return Val::range(Interval::of_type(e.ty));
        }
        let operands = [(l, a), (r, b)];
        Val::range(match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::AddWrap | BinOp::SubWrap | BinOp::MulWrap => {
                // `*% +% -%` have no check and wrap in every build: a result outside the type is
                // any value of it, and a linear form holds only when nothing wrapped.
                let wrapping = op.is_wrapping();
                let exact = match op {
                    BinOp::Add | BinOp::AddWrap => iv::add(a, b),
                    BinOp::Sub | BinOp::SubWrap => iv::sub(a, b),
                    _ => iv::mul(a, b),
                };
                if !wrapping {
                    self.site(e, CheckKind::Overflow, exact.fits(ty), already, &operands);
                }
                let r = iv::settle(exact, ty, checked && !wrapping);
                let keep = exact.fits(ty) || (!wrapping && (already || checked));
                let constant = |v: &Val| (v.r.lo == v.r.hi).then_some(v.r.lo);
                return match (op, constant(&vb), constant(&va)) {
                    (BinOp::Add | BinOp::AddWrap, Some(k), _) => va.plus(r, k, keep),
                    (BinOp::Sub | BinOp::SubWrap, Some(k), _) => va.plus(r, -k, keep),
                    (BinOp::Add | BinOp::AddWrap, None, Some(k)) => vb.plus(r, k, keep),
                    _ => Val::range(r),
                };
            }
            BinOp::Div | BinOp::Rem => {
                let safe = iv::division_safe(a, b, ty);
                if sites::divides_checked(ty, r) {
                    self.site(e, CheckKind::Division, safe, already, &operands);
                    if sure && !already {
                        // The zero check passed (in both builds): the divisor isn't 0.
                        self.learn(r, nonzero(b));
                    }
                }
                let exact = if op == BinOp::Div { iv::div(a, b, ty) } else { iv::rem(a, b, ty) };
                iv::settle(exact, ty, checked)
            }
            BinOp::Shl | BinOp::Shr => {
                let safe = iv::shift_safe(b, ty);
                if checked {
                    self.site(e, CheckKind::Shift, safe, already, &operands);
                    if sure && !already {
                        self.learn(r, Some(Interval::new(0, i128::from(iv::width(ty)) - 1)));
                    }
                }
                let n = iv::shift_amount(b, ty, checked);
                if op == BinOp::Shl {
                    iv::shl(a, n, ty)
                } else {
                    iv::shr(a, n)
                }
            }
            BinOp::BitAnd => iv::bit_and(a, b, ty),
            BinOp::BitOr | BinOp::BitXor => iv::bit_or_xor(a, b, ty, op == BinOp::BitXor),
            _ => Interval::of_type(e.ty),
        })
    }

    #[inline(never)]
    fn cast(&mut self, st: &State, e: &Expr, x: &Expr, sure: bool, already: bool) -> Val {
        let checked = self.arith.checked();
        let v = self.eval_v(st, x, sure);
        let a = v.r;
        let (from, to) = (x.ty, e.ty);
        if !to.is_int() || from.is_float() {
            return Val::range(Interval::of_type(to)); // a float saturates: anything of the target type
        }
        let narrowing = sites::is_narrowing(from, to);
        if a.fits(to) {
            // A widening, or a narrowing that can't fail: the same value.
            if narrowing && checked {
                self.site(e, CheckKind::Narrowing, true, already, &[(x, a)]);
            }
            return v.plus(a, 0, true);
        }
        if checked && narrowing {
            self.site(e, CheckKind::Narrowing, false, already, &[(x, a)]);
            if sure && !already {
                self.learn(x, Some(Interval::of_type(to)));
            }
        }
        // Exact when the check passed (checks on); otherwise it wrapped.
        let r = iv::settle(a, to, checked && narrowing);
        v.plus(r, 0, already || (checked && narrowing))
    }

    /// Decides the check of kind `kind` on `e` (`safe`: it can't fail), unless it was proven
    /// `already` or codegen doesn't emit it in this build.
    fn site(&mut self, e: &Expr, kind: CheckKind, safe: bool, already: bool, operands: &[(&Expr, Interval)]) {
        // Only a check codegen emits in this build is a site.
        if !already && sites::checks_of(e, self.arith).contains(&kind) {
            self.decide(expr_key(e), kind, e.line, safe, |cx| super::show::check_text(cx.m, cx.f, kind, e, operands));
        }
    }

    /// After a check on `e` passed, `e`'s value is in `range` (when there is one): a fact about
    /// the local `e` reads, if it reads one directly (or through a widening conversion).
    fn learn(&mut self, e: &Expr, range: Option<Interval>) {
        if let (Some(l), Some(r)) = (self.local_of(e), range) {
            self.learned.push(Learn::Range(l, r));
        }
    }

    /// The tracked local whose value `e` is, if it is one: a load of it, or a widening conversion
    /// of one.
    pub(super) fn local_of(&self, e: &Expr) -> Option<LocalIdx> {
        super::terms::local_value(self.f, e)
    }
}

/// What a passed zero check says about a divisor in `b`: it isn't 0, which narrows an end at 0.
fn nonzero(b: Interval) -> Option<Interval> {
    if b.lo == 0 && b.hi > 0 {
        Some(Interval::new(1, b.hi))
    } else if b.hi == 0 && b.lo < 0 {
        Some(Interval::new(b.lo, -1))
    } else {
        None
    }
}
