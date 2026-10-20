//! Unary and binary operators: numbers, bit operators, comparisons, vectors and string `+`.

use kestrel_syntax::ast::{BinaryOp, Expr, ExprKind, UnaryOp};

use super::BodyChecker;
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    pub(super) fn unary(&mut self, op: UnaryOp, inner: &Expr, want: Option<Ty>) -> Ty {
        match op {
            UnaryOp::Not => {
                self.check_value(inner, Some(Ty::Bool));
                Ty::Bool
            }
            UnaryOp::BitNot => {
                let ty = self.check_expr(inner, want.filter(|t| t.is_int()));
                if !ty.is_int() && ty != Ty::Error {
                    let msg = format!("`~` needs an integer, found {}", self.tn(ty));
                    self.error(inner.span, msg);
                    return Ty::Error;
                }
                ty
            }
            UnaryOp::Neg => {
                let ty = self.check_expr(inner, want.filter(|t| t.is_numeric()));
                if self.t.vec_dims(ty).is_some() {
                    return ty;
                }
                if ty == Ty::U8 {
                    self.error(inner.span, "can't negate a u8");
                    return Ty::Error;
                }
                if !ty.is_numeric() && ty != Ty::Error {
                    let msg = format!("`-` needs a number, found {}", self.tn(ty));
                    self.error(inner.span, msg);
                    return Ty::Error;
                }
                ty
            }
        }
    }

    pub(super) fn binary(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, rhs: &Expr, want: Option<Ty>) -> Ty {
        match op {
            BinaryOp::And | BinaryOp::Or => {
                self.check_value(lhs, Some(Ty::Bool));
                self.check_sometimes(rhs, Some(Ty::Bool));
                Ty::Bool
            }
            // A shift's result has the left side's type; the amount is any integer.
            BinaryOp::Shl | BinaryOp::Shr => {
                let lt = self.check_expr(lhs, want.filter(|t| t.is_int()));
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_int()));
                self.bit_operands(op, lhs, lt, rhs, rt);
                if lt.is_int() { lt } else { Ty::Error }
            }
            _ if op.is_bitwise() => {
                let lt = self.check_expr(lhs, want.filter(|t| t.is_int()));
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_int()));
                if !self.bit_operands(op, lhs, lt, rhs, rt) {
                    return Ty::Error;
                }
                self.operands(e, op, lhs, lt, rhs, rt)
            }
            _ if op.is_comparison() => {
                let lt = self.check_expr(lhs, None);
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_numeric()));
                // (Rule 7b's `holds_fn` stays separate: it doesn't look inside interface values.)
                // Testing against `none` compares no values (it asks only whether there is one), so
                // `d == none` is not a comparison of interface values.
                let against_none = matches!(lhs.kind, ExprKind::None) || matches!(rhs.kind, ExprKind::None);
                let functions = [lt, rt].iter().any(|&t| self.t.holds_fn(t));
                if matches!(op, BinaryOp::Eq | BinaryOp::Ne) && [lt, rt].iter().any(|&t| self.t.holds_uncomparable(t)) && (functions || !against_none) {
                    let msg = if functions { "function values can't be compared" } else { "interface values can't be compared; compare what a method returns" };
                    self.error(e.span, msg);
                    return Ty::Bool;
                }
                if matches!(op, BinaryOp::Eq | BinaryOp::Ne) && [lt, rt].iter().any(|t| matches!(t, Ty::Bool | Ty::Str)) {
                    if lt != rt && lt != Ty::Error && rt != Ty::Error {
                        let msg = format!("`{}` needs two numbers, two bools or two strings, found {} and {}", op.symbol(), self.tn(lt), self.tn(rt));
                        self.error(e.span, msg);
                    }
                    return Ty::Bool;
                }
                self.operands(e, op, lhs, lt, rhs, rt);
                Ty::Bool
            }
            _ => {
                let hint = want.filter(|t| t.is_numeric());
                let lt = self.check_expr(lhs, hint);
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_numeric()).or(hint));
                if op == BinaryOp::Add && (lt == Ty::Str || rt == Ty::Str) {
                    return self.str_concat(e, lt, rt);
                }
                if self.t.vec_dims(lt).is_some() || self.t.vec_dims(rt).is_some() {
                    return self.vec_binary(e, op, lhs, lt, rhs, rt);
                }
                self.operands(e, op, lhs, lt, rhs, rt)
            }
        }
    }

    /// `+ - * /` on vectors: component-wise with a vector of the same size,
    /// or with a number (applied to every component).
    fn vec_binary(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, lt: Ty, rhs: &Expr, rt: Ty) -> Ty {
        if lt == Ty::Error || rt == Ty::Error {
            return Ty::Error;
        }
        let sym = op.symbol();
        if !matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div) {
            self.error(e.span, format!("`{sym}` doesn't work on vectors"));
            return Ty::Error;
        }
        let (vec, scalar, scalar_ty) = match (self.t.vec_dims(lt), self.t.vec_dims(rt)) {
            (Some(_), Some(_)) if lt == rt => return lt,
            (Some(_), Some(_)) => {
                let msg = format!("`{sym}` can't mix {} and {}", self.tn(lt), self.tn(rt));
                self.error(e.span, msg);
                return Ty::Error;
            }
            (Some(_), None) => (lt, rhs, rt),
            _ => (rt, lhs, lt),
        };
        if !scalar_ty.is_numeric() {
            let msg = format!("`{sym}` needs a vector and a number, found {} and {}", self.tn(lt), self.tn(rt));
            self.error(e.span, msg);
            return Ty::Error;
        }
        self.coerce(scalar, scalar_ty, Ty::Float);
        vec
    }

    /// Bit operators take integers. Returns whether both sides are.
    fn bit_operands(&mut self, op: BinaryOp, lhs: &Expr, lt: Ty, rhs: &Expr, rt: Ty) -> bool {
        for (e, t) in [(lhs, lt), (rhs, rt)] {
            if !t.is_int() && t != Ty::Error {
                let msg = format!("`{}` needs integers, found {}", op.symbol(), self.tn(t));
                self.error(e.span, msg);
                return false;
            }
        }
        lt.is_int() && rt.is_int()
    }

    /// Both operands must be numbers that meet at a common type.
    fn operands(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, lt: Ty, rhs: &Expr, rt: Ty) -> Ty {
        if lt == Ty::Error || rt == Ty::Error {
            return Ty::Error;
        }
        let sym = op.symbol();
        if !lt.is_numeric() || !rt.is_numeric() {
            let msg = format!("`{sym}` needs numbers, found {} and {}", self.tn(lt), self.tn(rt));
            self.error(e.span, msg);
            return Ty::Error;
        }
        match Ty::common(lt, rt) {
            Some(c) => {
                self.coerce(lhs, lt, c);
                self.coerce(rhs, rt, c);
                c
            }
            None => {
                let msg = format!("`{sym}` can't mix {} and {}", self.tn(lt), self.tn(rt));
                self.error_help(e.span, msg, "convert one side explicitly, e.g. `int(x)` or `u8(x)`");
                Ty::Error
            }
        }
    }
}
