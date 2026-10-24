//! Operators on vectors (the spec's "Operators" table; one rule for every vector type, pick 4):
//! component by component with a vector of the same type, or with a plain number that fills
//! every component. What each kind has is in `VecKind` (`vectors.rs`).

use kestrel_syntax::ast::{AssignOp, BinaryOp, Expr};
use kestrel_syntax::Span;

use super::BodyChecker;
use crate::ty::Ty;
use crate::vectors::VecKind;

impl<'a> BodyChecker<'a> {
    /// The error for `op` (written `sym`, like `/` or `/=`) on a vector of type `vt`, if that
    /// type has no such operator.
    fn vec_op_error(&self, op: BinaryOp, sym: &str, vt: Ty, kind: VecKind) -> Option<String> {
        use BinaryOp::*;
        let vn = self.tn(vt);
        match op {
            Add | Sub | Mul | Eq | Ne => None,
            Div if kind.has_div() => None,
            Div => Some(format!(
                "`{vn}` has no `/`: the CPU can't divide integer lanes at once; for a power of two use `>>`, or divide each lane, like `v.x / n`"
            )),
            Rem if kind.has_rem() => None,
            Rem if !kind.int => Some(format!("`{sym}` doesn't work on vectors")),
            Rem => Some(format!(
                "`{vn}` has no `%`: the CPU can't divide integer lanes at once; for a power of two use `&`, or take each lane, like `v.x % n`"
            )),
            Lt | Gt | Le | Ge => Some(format!("`{sym}` doesn't compare vectors; compare components, like `a.x < b.x`")),
            _ if kind.has_bits() => None,
            AddWrap | SubWrap | MulWrap => {
                let plain = op.plain().expect("a wrapping operator").symbol();
                Some(format!("`{sym}` wraps integers; for floats use `{plain}`"))
            }
            _ => Some(format!("`{sym}` needs integers, found {vn}")),
        }
    }

    /// `a op b` where at least one side is a vector: the vector's type (`bool` for `==` and
    /// `!=`), or `Error`.
    pub(super) fn vec_binary(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, lt: Ty, rhs: &Expr, rt: Ty) -> Ty {
        if lt == Ty::Error || rt == Ty::Error {
            return Ty::Error;
        }
        let sym = op.symbol();
        let (vt, kind) = match (self.t.vector(lt), self.t.vector(rt)) {
            (Some(k), _) => (lt, k),
            (None, Some(k)) => (rt, k),
            (None, None) => unreachable!("one side is a vector"),
        };
        if let Some(msg) = self.vec_op_error(op, sym, vt, kind) {
            self.error(e.span, msg);
            return Ty::Error;
        }
        let result = if matches!(op, BinaryOp::Eq | BinaryOp::Ne) { Ty::Bool } else { vt };
        if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
            return if self.vec_shift_amount(sym, rhs, rt) { result } else { Ty::Error };
        }
        let (scalar, scalar_ty) = match (self.t.vector(lt), self.t.vector(rt)) {
            (Some(_), Some(_)) if lt == rt => return result,
            (Some(_), Some(_)) => {
                let msg = format!("`{sym}` can't mix {} and {}", self.tn(lt), self.tn(rt));
                self.error(e.span, msg);
                return Ty::Error;
            }
            (Some(_), None) => (rhs, rt),
            _ => (lhs, lt),
        };
        if !scalar_ty.is_numeric() {
            let msg = format!("`{sym}` needs a vector and a number, found {} and {}", self.tn(lt), self.tn(rt));
            self.error(e.span, msg);
            return Ty::Error;
        }
        if self.fill_lanes(e.span, sym, vt, kind, scalar, scalar_ty) {
            result
        } else {
            Ty::Error
        }
    }

    /// The amount of a vector shift: one integer for every component (V9). Returns whether it is.
    fn vec_shift_amount(&mut self, sym: &str, amount: &Expr, ty: Ty) -> bool {
        if self.t.vector(ty).is_some() {
            let msg = format!("`{sym}` shifts every component by one `int`; shift each component on its own for different amounts");
            self.error(amount.span, msg);
            return false;
        }
        if !ty.is_int() && ty != Ty::Error {
            let msg = format!("`{sym}` needs integers, found {}", self.tn(ty));
            self.error(amount.span, msg);
            return false;
        }
        ty != Ty::Error
    }

    /// A plain number next to a vector of type `vt` fills every component: it becomes the lane
    /// type (a hex literal is read at it). An int vector takes no float (V7). Returns whether
    /// it fits.
    pub(super) fn fill_lanes(&mut self, span: Span, sym: &str, vt: Ty, kind: VecKind, scalar: &Expr, scalar_ty: Ty) -> bool {
        if kind.int && scalar_ty.is_float() {
            let msg = format!("`{sym}` can't mix {} and {}; convert with `{}(...)` first", self.tn(vt), self.tn(scalar_ty), self.tn(kind.as_float()));
            self.error(span, msg);
            return false;
        }
        self.coerce(scalar, scalar_ty, kind.lane());
        true
    }

    /// `v += w`, `c /= 2`, `h >>= 1`, `h *%= K` on a vector `target` of type `target_ty`: the
    /// value is a vector of the same type or a number, as for the operator.
    pub(super) fn check_vec_compound(&mut self, target: &Expr, target_ty: Ty, op: AssignOp, value: &Expr) {
        let kind = self.t.vector(target_ty).expect("a vector target");
        let bop = op.binary().expect("a compound assignment");
        let sym = op.symbol();
        let value_ty = self.check_expr(value, None);
        if value_ty == Ty::Error {
            return;
        }
        if let Some(msg) = self.vec_op_error(bop, sym, target_ty, kind) {
            self.error(value.span, msg);
            return;
        }
        if matches!(bop, BinaryOp::Shl | BinaryOp::Shr) {
            self.vec_shift_amount(sym, value, value_ty);
        } else if value_ty.is_numeric() {
            self.fill_lanes(target.span.to(value.span), sym, target_ty, kind, value, value_ty);
        } else if value_ty != target_ty {
            let msg = format!("`{sym}` needs a {} or a number, found {}", self.tn(target_ty), self.tn(value_ty));
            self.error(value.span, msg);
        }
    }
}
