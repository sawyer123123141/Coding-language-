use kestrel_syntax::ast::{AssignOp, Expr, ExprKind};

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
            // `xs.len` is computed, not stored.
            ExprKind::Field { base, .. } if matches!(self.t.ty(base.id), Ty::Array(_) | Ty::List(_) | Ty::Map(..)) => None,
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => self.place_root(base),
            _ => None,
        }
    }

    pub(crate) fn immutable(&self, root: LocalId) -> Option<Immutable> {
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
            let f = self.fn_id.expect("inside a function");
            self.t.fns[f.0 as usize].mutates_params = true;
        } else {
            self.t.locals[root.0 as usize].addressed = true;
        }
    }

    pub(crate) fn check_assign(&mut self, target: &Expr, op: AssignOp, value: &Expr) {
        if !matches!(target.kind, ExprKind::Ident(_) | ExprKind::Field { .. } | ExprKind::Index { .. }) {
            self.error(target.span, "only variables, fields and elements can be assigned to");
            self.check_expr(value, None);
            return;
        }
        let target_ty = self.check_expr(target, None);
        let Some(root) = self.place_root(target) else {
            if let (ExprKind::Ident(name), Some(Res::Const(_))) = (&target.kind, self.t.res.get(&target.id)) {
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
            // Plain assignment keeps a local in a register; only a `var` parameter matters.
            None if self.t.local(root).is_var_param => {
                let f = self.fn_id.expect("inside a function");
                self.t.fns[f.0 as usize].mutates_params = true;
            }
            None => {}
        }
        if let ExprKind::Index { base, .. } = &target.kind {
            if let Some((_, v)) = self.map_of(self.t.ty(base.id)) {
                return self.check_map_set(target, op, value, v);
            }
        }
        if target_ty == Ty::Arena {
            self.error_help(
                target.span,
                "an `Arena` can't be replaced: values that point into it would be left dangling",
                "replace the whole value that owns it",
            );
        }
        if op != AssignOp::Set && self.t.vec_dims(target_ty).is_some() {
            return self.check_vec_compound(target_ty, op, value);
        }
        if op.binary().is_some_and(|b| b.is_bitwise()) && !target_ty.is_int() && target_ty != Ty::Error {
            let msg = format!("`{}` needs an integer, found {}", op.symbol(), self.tn(target_ty));
            self.error(target.span, msg);
        }
        if op != AssignOp::Set && !target_ty.is_numeric() && target_ty != Ty::Error {
            let msg = format!("`{}` needs a number, found {}", op.symbol(), self.tn(target_ty));
            self.error(target.span, msg);
        }
        self.check_value(value, Some(target_ty));
        self.consume(value);
        self.check_store(target, value);
    }

    /// `v += w`, `v *= 2.0`: the value is a vector of the same size, or a number.
    fn check_vec_compound(&mut self, target_ty: Ty, op: AssignOp, value: &Expr) {
        let value_ty = self.check_expr(value, None);
        if op == AssignOp::Rem {
            self.error(value.span, "`%=` doesn't work on vectors");
        } else if value_ty.is_numeric() {
            self.coerce(value, value_ty, Ty::Float);
        } else if value_ty != target_ty && value_ty != Ty::Error {
            let msg = format!("`{}` needs a {} or a number, found {}", op.symbol(), self.tn(target_ty), self.tn(value_ty));
            self.error(value.span, msg);
        }
    }
}
