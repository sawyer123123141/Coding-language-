//! The wrapping operators `*% +% -%` and `*%= +%= -%=`: integer arithmetic that wraps around on
//! overflow and never checks, in every build. The types that have them are listed in `wraps`.

use kestrel_syntax::ast::{AssignOp, BinaryOp, Expr};
use kestrel_syntax::Span;

use super::BodyChecker;
use crate::ty::Ty;

/// Whether the wrapping operators work on `ty`: `int`, `i64` and `u8`.
pub(super) fn wraps(ty: Ty) -> bool {
    ty.is_int()
}

impl<'a> BodyChecker<'a> {
    /// `a *% b`, `a +% b`, `a -% b`: integers meet at one type as `*` does. Floats and float
    /// vectors have no wrapping form.
    pub(super) fn wrap_binary(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, lt: Ty, rhs: &Expr, rt: Ty) -> Ty {
        if lt == Ty::Error || rt == Ty::Error {
            return Ty::Error;
        }
        if [lt, rt].iter().any(|&t| self.no_wrap(t)) {
            let plain = op.plain().expect("a wrapping operator");
            self.wrap_float_error(e.span, op.symbol(), plain.symbol());
            return Ty::Error;
        }
        for (side, t) in [(lhs, lt), (rhs, rt)] {
            if !wraps(t) {
                let msg = format!("`{}` needs integers, found {}", op.symbol(), self.tn(t));
                self.error(side.span, msg);
                return Ty::Error;
            }
        }
        self.operands(e, op, lhs, lt, rhs, rt)
    }

    /// `x *%= v` and the other compound forms, once `target` is checked. Returns whether it
    /// reported an error about the target's type (the value is then checked on its own).
    pub(super) fn wrap_compound(&mut self, target: &Expr, target_ty: Ty, op: AssignOp, value: &Expr) -> bool {
        if !op.binary().is_some_and(BinaryOp::is_wrapping) || !self.no_wrap(target_ty) {
            return false;
        }
        let plain = op.binary().and_then(BinaryOp::plain).expect("a wrapping operator");
        let plain = format!("{}=", plain.symbol());
        let span = target.span.to(value.span);
        self.wrap_float_error(span, op.symbol(), &plain);
        self.check_expr(value, None);
        true
    }

    /// Types with no wrapping form: floats and every vector.
    fn no_wrap(&self, ty: Ty) -> bool {
        ty.is_float() || self.t.vec_dims(ty).is_some()
    }

    /// W1: the message that sends floats to the plain operator.
    fn wrap_float_error(&mut self, span: Span, op: &str, plain: &str) {
        self.error(span, format!("`{op}` wraps integers; for floats use `{plain}`"));
    }
}
