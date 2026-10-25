//! Variants of generic enums (P20): `Maybe[int].Nothing`, `Result[int, string].Ok(value: 3)`.
//! `structs.rs` asks here before today's `enum_named` test, which would return the declaration
//! for a bare generic enum's name. A `TypeApply` base with its types written gives the enum's
//! instance (its variants are filled), and today's `variant_value` / `variant_construct` then work
//! on it as for any enum. A generic enum named bare has nothing to work its types out from yet
//! (Task 4 infers them) and says what to write.

use kestrel_syntax::ast::{Expr, ExprKind};

use super::BodyChecker;
use crate::generic::GenericOwner;
use crate::tables::EnumId;

impl<'a> BodyChecker<'a> {
    /// The enum a variant path `base.variant` (the field access or method call `e`) starts from,
    /// when `base` names a generic enum or applies types to an enum: `None` when it is not such a
    /// path (today's rules apply), `Some(None)` after an error, else the enum or its instance.
    pub(super) fn generic_variant_base(&mut self, e: &Expr, base: &Expr, variant: &str) -> Option<Option<EnumId>> {
        match &base.kind {
            ExprKind::TypeApply { name, name_span, type_args } => {
                let &eid = self.t.enum_by_name.get(name)?;
                let owner = GenericOwner::Enum(eid);
                let Some(list) = self.written_types(owner, name, *name_span, type_args) else { return Some(None) };
                self.t.generic.type_args.insert(e.id, list);
                Some(Some(self.t.intern_enum(eid, list)))
            }
            ExprKind::Ident(name) => {
                if self.lookup_local(name).is_some() {
                    return None;
                }
                let &eid = self.t.enum_by_name.get(name)?;
                let owner = GenericOwner::Enum(eid);
                // Inside its own declaration the bare name means itself (G2).
                if self.t.enm(eid).params.is_empty() || self.own == Some(owner) {
                    return None;
                }
                let types = self.int_types(owner, name);
                let example = match self.t.enm(eid).variants.iter().find(|v| v.name == variant) {
                    Some(v) if v.fields.is_empty() => format!("{types}.{variant}"),
                    Some(_) => format!("{types}.{variant}(...)"),
                    None => {
                        self.error(base.span, format!("`{name}` has no variant `{variant}`"));
                        return Some(None);
                    }
                };
                self.undecided(base.span, owner, name, &example);
                Some(None)
            }
            _ => None,
        }
    }
}
