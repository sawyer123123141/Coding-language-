//! The result type of a closure literal that says none: what its `return`s and its tail agree on.

use kestrel_syntax::ast::{Block, Expr, ExprKind, Stmt};
use kestrel_syntax::Span;

use super::BodyChecker;
use crate::ty::Ty;

/// What the `return`s and the tail of a closure say about its result.
pub(super) struct Infer {
    /// The type of the first value returned or given by the tail.
    pub(super) found: Option<Ty>,
    /// A `return` with no value.
    pub(super) bare_return: Option<Span>,
}

impl<'a> BodyChecker<'a> {
    /// The result type of a closure that said none: what its `return`s and its tail agree on.
    pub(super) fn inferred_result(&mut self, tail: Option<&Expr>, body: Ty) -> Ty {
        // A tail that gives a value is the first or a later one.
        if let Some(t) = tail {
            if !matches!(body, Ty::Unit | Ty::Never) {
                let found = self.frames.last().and_then(|f| f.infer.as_ref()).and_then(|i| i.found);
                match found {
                    Some(w) => self.coerce(t, body, w),
                    None => self.set_found(body),
                }
            }
        }
        let infer = self.frames.last_mut().and_then(|f| f.infer.take()).expect("inferring a result");
        let found = infer.found.unwrap_or(Ty::Unit);
        if let (Some(span), false) = (infer.bare_return, matches!(found, Ty::Unit | Ty::Error)) {
            let msg = format!("this closure must return {}", self.tn(found));
            self.error(span, msg);
        }
        found
    }

    fn set_found(&mut self, ty: Ty) {
        if let Some(infer) = self.frames.last_mut().and_then(|f| f.infer.as_mut()) {
            infer.found.get_or_insert(ty);
        }
    }

    /// Whether the closure being checked is still working out its result type.
    pub(crate) fn infers_result(&self) -> bool {
        self.frames.last().is_some_and(|f| f.infer.is_some())
    }

    /// `return` in a closure whose result type is being worked out: the first value fixes it,
    /// and the rest must agree.
    pub(crate) fn infer_return(&mut self, value: Option<&Expr>, span: Span) {
        let Some(v) = value else {
            if let Some(infer) = self.frames.last_mut().and_then(|f| f.infer.as_mut()) {
                infer.bare_return.get_or_insert(span);
            }
            return;
        };
        let found = self.frames.last().and_then(|f| f.infer.as_ref()).and_then(|i| i.found);
        let ty = self.check_value(v, found);
        if ty == Ty::Unit {
            self.error(v.span, "this expression has no value to return");
            self.set_found(Ty::Error);
            return;
        }
        self.set_found(ty);
        self.consume(v);
        self.check_return_region(v);
    }
}

/// Whether the last statement of a closure body whose result isn't known gives no value: it is
/// not an expression, or it is an `if` or `match` whose first branch is such, or an `if` without
/// an `else`. (The result of a trailing call is whatever the call gives.)
pub(super) fn tail_is_effect(body: &Block) -> bool {
    match body.stmts.last() {
        None => true,
        Some(Stmt::Expr(e)) => match &e.kind {
            ExprKind::If { then_block, else_block, .. } => else_block.is_none() || tail_is_effect_branch(then_block),
            ExprKind::Match { arms, .. } => arms.first().is_none_or(|a| tail_is_effect_branch(&a.body)),
            _ => false,
        },
        Some(_) => true,
    }
}

/// `tail_is_effect` for a branch, where a `print` call is an effect too.
fn tail_is_effect_branch(branch: &Block) -> bool {
    match branch.stmts.last() {
        Some(Stmt::Expr(e)) => matches!(&e.kind, ExprKind::Call { name, .. } if name == "print") || tail_is_effect(branch),
        _ => true,
    }
}
