use kestrel_syntax::ast::{AssignOp, Expr, ExprKind};

use super::narrow::Site;
use super::BodyChecker;
use crate::tables::{LocalId, Res};
use crate::ty::Ty;

/// Why the variable at the root of a place can't be changed.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Immutable {
    Let,
    Param,
    /// `self` in a method that takes `self`, not `var self`.
    SelfParam,
    /// A local from outside the closure literal being checked (see `captures`).
    Captured,
}

impl<'a> BodyChecker<'a> {
    /// The variable a place expression (`x`, `x.f`, `x[i].g`) is rooted at.
    /// Call after the expression has been checked.
    pub(crate) fn place_root(&self, e: &Expr) -> Option<LocalId> {
        match &e.kind {
            // A `match` binding stands for the variable it was matched from.
            ExprKind::Ident(_) => match self.t.res.get(&e.id) {
                Some(Res::Local(l)) => Some(self.t.local(*l).alias.unwrap_or(*l)),
                _ => None,
            },
            // A swizzle (`v.xz`) builds a new vector; it isn't a place.
            ExprKind::Field { .. } if self.t.swizzles.contains_key(&e.id) => None,
            // `xs.len` is computed, not stored; and a string has no places inside it.
            ExprKind::Field { base, .. } if matches!(self.t.ty(base.id), Ty::Array(_) | Ty::List(_) | Ty::Map(..) | Ty::Str) => None,
            ExprKind::Index { base, .. } if self.t.ty(base.id) == Ty::Str => None,
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => self.place_root(base),
            _ => None,
        }
    }

    pub(crate) fn immutable(&self, root: LocalId) -> Option<Immutable> {
        if self.is_capture(root) {
            return Some(Immutable::Captured);
        }
        let info = self.t.local(root);
        match (info.mutable, info.is_param, info.name == "self") {
            (true, _, _) => None,
            (false, true, true) => Some(Immutable::SelfParam),
            (false, true, false) => Some(Immutable::Param),
            (false, false, _) => Some(Immutable::Let),
        }
    }

    /// Records that a call or assignment changes `root`: a `var` parameter
    /// makes the function change its caller's value; a local needs memory.
    pub(crate) fn mark_changed(&mut self, root: LocalId) {
        if self.t.local(root).is_var_param {
            self.note_param_changed(root);
        } else {
            self.t.locals[root.0 as usize].addressed = true;
        }
    }

    /// The function (or closure) that `root`, one of its `var` parameters, belongs to changes it.
    fn note_param_changed(&mut self, root: LocalId) {
        if !self.closure_mutates(root) {
            let f = self.fn_id.expect("a function's own parameter is changed inside the function");
            self.t.fns[f.0 as usize].mutates_params = true;
        }
    }

    pub(crate) fn check_assign(&mut self, target: &Expr, op: AssignOp, value: &Expr) {
        if !matches!(target.kind, ExprKind::Ident(_) | ExprKind::Field { .. } | ExprKind::Index { .. }) {
            self.error(target.span, "only variables, fields and elements can be assigned to");
            self.check_expr(value, None);
            return;
        }
        // `t = value` gives a moved variable its value back; naming it here isn't a use.
        let reinit = match (&target.kind, op) {
            (ExprKind::Ident(name), AssignOp::Set) => self.lookup_local(name),
            _ => None,
        };
        let was = reinit.and_then(|l| self.moved.remove(&l));
        // A removed loop item's slot is gone: assigning doesn't give it back, and naming it is a use.
        let (reinit, was) = match (reinit, was) {
            (Some(l), Some(at)) if at.removed => {
                self.moved.insert(l, at);
                (None, None)
            }
            other => other,
        };
        let target_ty = self.check_expr(target, None);
        if let (Some(l), Some(at)) = (reinit, was) {
            self.moved.insert(l, at);
        }
        // A string is built whole: `s[i] = x` has nothing to change in place.
        if matches!(&target.kind, ExprKind::Index { base, .. } if self.t.ty(base.id) == Ty::Str) {
            self.error(target.span, super::strings::STR_IN_PLACE);
            self.check_expr(value, None);
            return;
        }
        let Some(root) = self.place_root(target) else {
            if let (ExprKind::Ident(name), Some(Res::Const(_) | Res::Builtin(_))) = (&target.kind, self.t.res.get(&target.id)) {
                let msg = format!("can't change constant `{name}`");
                self.error(target.span, msg);
            } else if target_ty != Ty::Error {
                self.error(target.span, "only variables, fields and elements can be assigned to");
            }
            self.check_expr(value, None);
            return;
        };
        self.check_not_matched(target);
        let name = self.t.local(root).name.clone();
        match self.immutable(root) {
            // A loop's own variable says why it can't change (see `loop_vars`).
            Some(_) if self.loop_var_assign_error(target) => {}
            Some(Immutable::Let) => {
                self.error_help(target.span, format!("can't change `{name}`: it was declared with `let`"), "declare it with `var`")
            }
            Some(Immutable::Param) => self.error_help(
                target.span,
                format!("can't change parameter `{name}`"),
                format!("declare it as `var {name}` to change the caller's value"),
            ),
            Some(Immutable::SelfParam) => self.error_help(
                target.span,
                "can't change `self`: the method takes `self`, not `var self`",
                "declare the method with `var self`",
            ),
            Some(Immutable::Captured) => self.error(target.span, Self::captured_message(&name)),
            // Plain assignment keeps a local in a register; only a `var` parameter matters.
            None if self.t.local(root).is_var_param => self.note_param_changed(root),
            None => {}
        }
        self.check_par_write(target);
        if let ExprKind::Index { base, .. } = &target.kind {
            if let Some((_, v)) = self.map_of(self.t.ty(base.id)) {
                return self.going_into(value, Site::MapValue, |c| c.check_map_set(target, op, value, v));
            }
        }
        if target_ty == Ty::Arena {
            self.error_help(
                target.span,
                "an `Arena` can't be replaced: values that point into it would be left dangling",
                "replace the whole value that owns it",
            );
        }
        if self.wrap_compound(target, target_ty, op, value) {
            return;
        }
        if op != AssignOp::Set && self.t.vector(target_ty).is_some() {
            return self.check_vec_compound(target, target_ty, op, value);
        }
        // `a += b` on a `T`: no operator is known to work on it, as for `a + b`.
        if let (true, Ty::Param(p)) = (op != AssignOp::Set, target_ty) {
            self.param_operator_error(target, op.symbol(), p);
            self.check_value(value, Some(target_ty));
            return;
        }
        if op.binary().is_some_and(|b| b.is_bitwise() || b.is_wrapping()) && !target_ty.is_int() && target_ty != Ty::Error {
            let msg = format!("`{}` needs an integer, found {}", op.symbol(), self.tn(target_ty));
            self.error(target.span, msg);
        }
        // `s += t` makes a new string from `s` and `t`.
        let appends = op == AssignOp::Add && target_ty == Ty::Str;
        if op != AssignOp::Set && !appends && !target_ty.is_numeric() && target_ty != Ty::Error {
            let msg = format!("`{}` needs a number, found {}", op.symbol(), self.tn(target_ty));
            self.error(target.span, msg);
        }
        let site = match target.kind {
            ExprKind::Field { .. } => Site::Field,
            ExprKind::Index { .. } => Site::Item,
            _ => Site::Variable,
        };
        self.going_into(value, site, |c| c.check_value(value, Some(target_ty)));
        // (Nothing is stored then, so the memory rules have nothing to add.)
        if op == AssignOp::Set && self.check_iface_replace(target, value, target_ty) {
            return;
        }
        self.consume(value);
        if appends {
            self.check_store_new(target, value);
        } else {
            self.check_store(target, value);
        }
        if let Some(l) = reinit {
            self.moved.remove(&l);
        }
    }
}
