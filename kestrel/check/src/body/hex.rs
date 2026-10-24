//! Hex literals meeting a wider type. A hex literal is read at the type it meets: as an operand,
//! as a built-in's argument, wherever a value is widened. A hex literal that was read as a
//! wrapped `int` inside a larger expression (`-0xFFFFFFFF`, `0xFFFFFFFF >> 4`) can't be read
//! again once that expression meets an `i64` or a float, so that is an error that says which
//! value to write: otherwise `(0xFFFFFFFF >> 4) + w` and `w + (0xFFFFFFFF >> 4)` would differ.

use kestrel_syntax::ast::{BinaryOp, Block, Expr, ExprKind, Stmt, UnaryOp};

use super::BodyChecker;
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    /// A hex literal is read at the type it meets, whichever side it is on: next to an `i64` `w`,
    /// `0xFFFFFFFF + w` and `w + 0xFFFFFFFF` both use 4294967295 (not -1), and next to a float it is
    /// the float it spells (4294967295.0 times an `f64` 1.5, whichever side it is on). Returns
    /// whether `e` was one and is now typed `to`. (A variable that holds a wrapped hex literal is an
    /// `int`: `x * 1.5` is -1.5.)
    fn retype_hex(&mut self, e: &Expr, from: Ty, to: Ty) -> bool {
        let ExprKind::Int(n) = e.kind else { return false };
        if !to.is_numeric() || from == to || !self.t.hex_literals.contains(&e.id) {
            return false;
        }
        let ty = self.int_literal(e, n, Some(to));
        self.t.expr_ty[e.id.0 as usize] = ty;
        true
    }

    /// `e`, of type `from`, is widened to `to`. A bare hex literal is read at `to` instead
    /// (returns true: nothing converts). A wrapped hex literal inside `e` is an error.
    pub(super) fn widen_hex(&mut self, e: &Expr, from: Ty, to: Ty) -> bool {
        if self.retype_hex(e, from, to) {
            return true;
        }
        if self.t.wrapped_literals.is_empty() {
            return false;
        }
        let mut found = Vec::new();
        self.wrapped_inside(e, &mut found);
        for lit in found {
            let ExprKind::Int(n) = lit.kind else { continue };
            let lit_ty = self.t.expr_ty[lit.id.0 as usize];
            if lit_ty == to || !lit_ty.coerces_to(to) {
                continue;
            }
            // Reported once: a wider context that widens again doesn't repeat it.
            let Some(v) = self.t.wrapped_literals.remove(&lit.id) else { continue };
            let (from_name, to_name) = (self.tn(lit_ty), self.tn(to));
            let mean = if lit_ty == Ty::Int { "i64" } else { to.name() };
            let msg = format!(
                "`0x{n:X}` is read as the `{from_name}` {v} here, so this changes when it is widened to `{to_name}`; write `{mean}(0x{n:X})` to mean {n}, or `{from_name}(0x{n:X})` to keep {v}"
            );
            self.error(lit.span, msg);
        }
        false
    }

    /// The wrapped hex literals that `e`'s value is made from: through `-`, `~`, the arithmetic
    /// and bit operators and the values of `if` and `match`, not through calls or indexes
    /// (whose values have their own types).
    fn wrapped_inside<'e>(&self, e: &'e Expr, found: &mut Vec<&'e Expr>) {
        match &e.kind {
            ExprKind::Int(_) if self.t.wrapped_literals.contains_key(&e.id) => found.push(e),
            ExprKind::Unary { op: UnaryOp::Neg | UnaryOp::BitNot, expr } => self.wrapped_inside(expr, found),
            ExprKind::Binary { op, lhs, rhs } if !op.is_comparison() && !matches!(op, BinaryOp::And | BinaryOp::Or) => {
                self.wrapped_inside(lhs, found);
                self.wrapped_inside(rhs, found);
            }
            ExprKind::If { then_block, else_block, .. } => {
                for b in std::iter::once(then_block).chain(else_block) {
                    self.wrapped_tail(b, found);
                }
            }
            ExprKind::Match { arms, .. } => arms.iter().for_each(|a| self.wrapped_tail(&a.body, found)),
            _ => {}
        }
    }

    fn wrapped_tail<'e>(&self, b: &'e Block, found: &mut Vec<&'e Expr>) {
        if let Some(Stmt::Expr(tail)) = b.stmts.last() {
            self.wrapped_inside(tail, found);
        }
    }
}
