//! `pop` on a list of owning items: the item may not come from inside an arena's owner.

use kestrel_syntax::ast::{Expr, ExprKind};

use super::lists::written;
use super::BodyChecker;
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    /// A kept `pop` of owning items moves the item out without copying it (a non-owning item is
    /// copied into the current arena, like an indexed read). So it can't take the item from
    /// inside a value that holds an `Arena`, or from a list the caller passed in: the item's
    /// memory may be in an owner's arena, which goes when the owner is replaced or cleared. Only
    /// a list rooted in a local of this function hands one out. A discarded `pop` drops the item
    /// at once.
    pub(super) fn check_pop_owner(&mut self, e: &Expr, receiver: &Expr, elem: Ty) {
        if !self.t.is_owning(elem) || self.discarded == Some(e.id) {
            return;
        }
        let list = written(receiver);
        if let Some(owner) = self.arena_owner(receiver) {
            return self.error_help(
                e.span,
                format!("can't take an item out of `{list}` with `pop`: it may point into `{owner}`'s arena"),
                format!("read what you need with `{list}[i]`, then use `remove_at`"),
            );
        }
        // A list the caller passed in (a `var` parameter or `var self`, or anything reached
        // through one) may be inside an owner of the caller's, which it can replace while the
        // item lives.
        let from_caller = self.list_place(receiver).is_some_and(|(root, _)| self.t.local(root).is_param);
        if from_caller {
            self.error_help(
                e.span,
                format!("can't take an item out of `{list}` with `pop`: it may point into the caller's arena"),
                "return the index and let the caller use `pop` or `remove_at`",
            );
        }
    }

    /// The outermost value holding an `Arena` that the place `e` is inside of (not `e` itself),
    /// as written. A loop's item or a view stands for the place it reads.
    fn arena_owner(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Field { .. } if self.t.swizzles.contains_key(&e.id) => None,
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => {
                self.arena_owner(base).or_else(|| self.has_arena_field(self.t.ty(base.id)).then(|| written(base)))
            }
            ExprKind::Ident(_) => {
                let (root, path) = self.list_place(e)?;
                let mut ty = self.t.local(root).ty;
                let mut name = self.t.local(root).name.clone();
                for step in &path {
                    if self.has_arena_field(ty) {
                        return Some(name);
                    }
                    ty = match step {
                        Some(f) => {
                            name.push_str(&format!(".{f}"));
                            self.field_ty(ty, f)?
                        }
                        None => {
                            name.push_str("[_]");
                            self.elem_of(ty)?
                        }
                    };
                }
                None
            }
            _ => None,
        }
    }

    /// Whether a value of `ty` has an `Arena` field of its own.
    fn has_arena_field(&self, ty: Ty) -> bool {
        match ty {
            Ty::Struct(s) => self.t.strukt(s).fields.iter().any(|f| f.ty == Ty::Arena),
            Ty::Variant(en, v) => self.t.enm(en).variants[v as usize].fields.iter().any(|f| f.ty == Ty::Arena),
            Ty::Enum(en) => self.t.enm(en).variants.iter().flat_map(|v| &v.fields).any(|f| f.ty == Ty::Arena),
            _ => false,
        }
    }

    /// The type of field `name` of a struct or variant.
    fn field_ty(&self, ty: Ty, name: &str) -> Option<Ty> {
        let fields = match ty {
            Ty::Struct(s) => &self.t.strukt(s).fields,
            Ty::Variant(en, v) => &self.t.enm(en).variants[v as usize].fields,
            _ => return None,
        };
        fields.iter().find(|f| f.name == name).map(|f| f.ty)
    }
}
