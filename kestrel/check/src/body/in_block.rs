//! `in x.mem { }` blocks: allocations inside belong to the arena `x.mem`.

use kestrel_syntax::ast::{Block, Expr, ExprKind};

use super::regions::Ctx;
use super::{BodyChecker, Expect};
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    /// `in x.mem { }`: allocations inside belong to `x`.
    pub(crate) fn check_in(&mut self, arena: &Expr, body: &Block) -> Ty {
        let ty = self.check_expr(arena, None);
        let owner = match &arena.kind {
            ExprKind::Field { base, .. } => self.path_key(base),
            _ => self.path_key(arena),
        };
        let ctx = match (ty, owner) {
            (Ty::Arena, Some(p)) => {
                self.check_changeable_arena(arena);
                self.check_par_write(arena);
                Some(Ctx::Owned(p))
            }
            (Ty::Arena, None) => {
                self.error(arena.span, "`in` needs an arena stored in a variable or field");
                None
            }
            (Ty::Error, _) => None,
            (other, _) => {
                let msg = format!("`in` needs an `Arena`, like `in self.mem {{ }}`; found {}", self.tn(other));
                self.error(arena.span, msg);
                None
            }
        };
        let pushed = ctx.is_some();
        self.ctxs.extend(ctx);
        let ty = self.check_block(body, Expect::Discard);
        if pushed {
            self.ctxs.pop();
        }
        if ty == Ty::Never {
            Ty::Never
        } else {
            Ty::Unit
        }
    }

    /// Allocating into an arena changes it, so its owner must be changeable.
    fn check_changeable_arena(&mut self, arena: &Expr) {
        let Some(root) = self.place_root(arena) else { return };
        let name = self.t.local(root).name.clone();
        let help = match self.immutable(root) {
            None => return self.mark_changed(root),
            Some(super::place::Immutable::Captured) => return self.error(arena.span, Self::captured_message(&name)),
            Some(super::place::Immutable::Let) => format!("declare `{name}` with `var`"),
            Some(_) => format!("take `{name}` as a `var` parameter"),
        };
        self.error_help(arena.span, format!("`in` allocates into this arena, so `{name}` must be changeable"), help);
    }
}
