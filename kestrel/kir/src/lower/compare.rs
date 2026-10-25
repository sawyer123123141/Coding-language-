//! `<`, `<=`, `>` and `>=` on two strings or two bools (G5, P5), in plain code and in copies.
//!
//! - **Strings** compare by character, the order `sort()` uses: `StrOp::Cmp(a, b)` is a `u8`, 0 when
//!   `a` goes before `b`, 1 when they are equal, 2 when `a` goes after. So `a < b` is `cmp < 1`,
//!   `a <= b` is `cmp <= 1`, `a > b` is `cmp > 1` and `a >= b` is `cmp >= 1`. Reading a string
//!   makes nothing and fails nothing.
//! - **Bools** compare as their `u8` values (`false` is 0, `true` is 1), so `false < true`.
//!
//! Numbers keep the plain `Binary` in `expr.rs`.

use kestrel_syntax::ast;

use super::{binop, FnLowerer};
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// `lhs op rhs` for an ordering operator `op` on two strings or two bools.
    pub(crate) fn ordered_compare(&mut self, op: ast::BinaryOp, lhs: &ast::Expr, rhs: &ast::Expr, line: u32) -> ExprKind {
        let (l, r) = (self.expr(lhs), self.expr(rhs));
        let at = |kind: ExprKind| Expr { kind, ty: KTy::U8, line };
        let (l, r) = if l.ty == KTy::Str {
            (at(ExprKind::Str(StrOp::Cmp, vec![l, r])), at(ExprKind::Int(1)))
        } else {
            (at(ExprKind::Cast(Box::new(l))), at(ExprKind::Cast(Box::new(r))))
        };
        ExprKind::Binary(binop(op), Box::new(l), Box::new(r))
    }
}
