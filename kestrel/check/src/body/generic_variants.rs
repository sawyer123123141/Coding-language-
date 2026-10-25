//! Variants of generic enums (P20): `Maybe[int].Nothing`, `Result[int, string].Ok(value: 3)`,
//! and with no types written, `Maybe.Nothing` and `Result.Ok(value: 3)` (inferred, P8).
//! `structs.rs` asks here before today's `enum_named` test, which would return the declaration
//! for a bare generic enum's name. A `TypeApply` base with its types written gives the enum's
//! instance (its variants are filled), and today's `variant_value` / `variant_construct` then work
//! on it as for any enum. A generic enum named bare has its types worked out (`inferred.rs`).

use kestrel_syntax::ast::{Arg, Expr, ExprKind};
use kestrel_syntax::Span;

use super::BodyChecker;
use crate::generic::GenericOwner;
use crate::tables::EnumId;
use crate::ty::Ty;

/// What the base of a variant path `base.variant` names.
enum Base<'e> {
    /// Not a generic enum: today's rules apply.
    Plain,
    /// Its written types were refused (reported).
    Refused,
    /// An instance with its types written.
    Written(EnumId),
    /// A generic enum named bare, as written and where.
    Bare(EnumId, &'e str, Span),
}

impl<'a> BodyChecker<'a> {
    /// `base.variant` as the field access `e` (a variant without fields): `None` when `base` isn't
    /// a generic enum.
    pub(super) fn generic_variant_value(&mut self, e: &Expr, base: &Expr, variant: &str, variant_span: Span, want: Option<Ty>) -> Option<Ty> {
        Some(match self.variant_base(e, base) {
            Base::Plain => return None,
            Base::Refused => Ty::Error,
            Base::Written(id) => self.variant_value(e, id, variant, variant_span),
            Base::Bare(def, name, at) => self.check_inferred_unit(e, def, (name, at), (variant, variant_span), want),
        })
    }

    /// `base.variant(args)` as the method call `e` (a variant with fields): `None` when `base`
    /// isn't a generic enum.
    pub(super) fn generic_variant_call(&mut self, e: &Expr, base: &Expr, variant: &str, variant_span: Span, args: &[Arg], want: Option<Ty>) -> Option<Ty> {
        Some(match self.variant_base(e, base) {
            Base::Plain => return None,
            Base::Refused => {
                self.check_loose_args(args);
                Ty::Error
            }
            Base::Written(id) => self.variant_construct(e, id, variant, variant_span, args),
            Base::Bare(def, name, at) => self.check_inferred_variant(e, def, (name, at), (variant, variant_span), args, want),
        })
    }

    /// What `base` names, for the variant path `e`.
    fn variant_base<'e>(&mut self, e: &Expr, base: &'e Expr) -> Base<'e> {
        match &base.kind {
            ExprKind::TypeApply { name, name_span, type_args } => {
                let Some(&eid) = self.t.enum_by_name.get(name) else { return Base::Plain };
                let owner = GenericOwner::Enum(eid);
                let Some(list) = self.written_types(owner, name, *name_span, type_args) else { return Base::Refused };
                self.t.generic.type_args.insert(e.id, list);
                Base::Written(self.t.intern_enum(eid, list))
            }
            ExprKind::Ident(name) => {
                if self.lookup_local(name).is_some() {
                    return Base::Plain;
                }
                let Some(&eid) = self.t.enum_by_name.get(name) else { return Base::Plain };
                // Inside its own declaration the bare name means itself (G2).
                if self.t.enm(eid).params.is_empty() || self.own == Some(GenericOwner::Enum(eid)) {
                    return Base::Plain;
                }
                Base::Bare(eid, name, base.span)
            }
            _ => Base::Plain,
        }
    }
}
