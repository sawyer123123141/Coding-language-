//! `let x = opt else { ... }`: unwrapping an optional. When `opt` is a place that can't change,
//! `x` reads its payload in place (a view, like a loop's item) instead of moving or copying it.

use kestrel_syntax::ast::{Binding, Block, Expr, ExprKind, TypeExpr};

use super::{BodyChecker, Expect};
use crate::collect_ty::resolve_type;
use crate::tables::{LocalId, Res};
use crate::ty::Ty;

/// A `let ... else` view in scope: `view` reads from inside `root` until the end of the block that
/// declares it, which is open while there are at least `depth` scopes.
#[derive(Clone)]
pub(crate) struct LetGuard {
    pub(crate) view: LocalId,
    pub(crate) root: LocalId,
    pub(crate) depth: usize,
}

impl<'a> BodyChecker<'a> {
    /// `let x = opt else { ... }`: `x` is the value inside the optional;
    /// the `else` block runs when it's `none`, and must leave.
    pub(crate) fn check_let_else(&mut self, is_var: bool, bind: &Binding, ty: Option<&TypeExpr>, value: &Expr, otherwise: &Block) {
        let ann = ty.map(|t| resolve_type(t, self.t, self.diags));
        let want = ann.map(|t| if t == Ty::Error { t } else { Ty::Optional(self.t.intern_elem(t)) });
        let found = self.check_value(value, want);
        let inner = match found {
            Ty::Optional(e) => self.t.elem(e),
            Ty::Error => Ty::Error,
            other => {
                let msg = format!("`let ... else` unwraps an optional (`T?`), but this is {}", self.tn(other));
                self.error(value.span, msg);
                Ty::Error
            }
        };
        // The `else` block runs when `value` is `none`: nothing has been unwrapped or moved yet.
        // It must leave, so what it moves doesn't flow past it.
        let before = self.moved.clone();
        if self.check_block(otherwise, Expect::Discard) != Ty::Never {
            self.error_help(otherwise.span, "the `else` block has to leave", "end it with `return`, `break` or `continue`");
        }
        self.moved = before;
        let root = if inner == Ty::Error || is_var { None } else { self.view_root(value, inner) };
        if root.is_none() {
            self.consume(value);
        }
        let l = self.declare(bind, inner, is_var, false, false);
        if let Some(root) = root {
            self.t.locals[l.0 as usize].alias = Some(root);
            self.views.insert(l);
            self.note_view(l, value);
            self.t.let_else_views.insert(l);
            self.let_guards.push(LetGuard { view: l, root, depth: self.scopes.len() });
        }
    }

    /// The variable `x` reads from when `let x = value else` can be a view: `value` is a place
    /// rooted at a variable that can't change, it isn't a `T` made into a `T?`, and its payload
    /// (`inner`) holds memory worth not copying. A whole owning variable moves instead, as with `let`.
    fn view_root(&self, value: &Expr, inner: Ty) -> Option<LocalId> {
        let worth = self.t.has_heap(inner) || self.t.is_owning(inner);
        if !worth || self.t.coerce.contains_key(&value.id) || !self.is_place(value) {
            return None;
        }
        let root = self.place_root(value)?;
        self.immutable(root)?;
        if let Some(&Res::Local(l)) = self.t.res.get(&value.id) {
            if self.t.is_owning(inner) && !self.t.local(l).is_param && !self.views.contains(&l) {
                return None;
            }
        }
        Some(root)
    }

    /// Whether `e` is a variable, or fields and array or list elements of one: memory a view can read.
    fn is_place(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Ident(_) => matches!(self.t.res.get(&e.id), Some(Res::Local(_))),
            ExprKind::Field { base, .. } => self.t.field_index.contains_key(&e.id) && self.is_place(base),
            ExprKind::Index { base, .. } => matches!(self.t.ty(base.id), Ty::Array(_) | Ty::List(_)) && self.is_place(base),
            _ => false,
        }
    }
}
