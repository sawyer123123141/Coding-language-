use kestrel_syntax::ast::{Arg, Expr, ExprKind};
use kestrel_syntax::Span;

use super::BodyChecker;
use crate::tables::{EnumId, Res};
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    /// The enum `e` names, when it is a bare enum name (not a variable).
    pub(crate) fn enum_named(&self, e: &Expr) -> Option<EnumId> {
        let ExprKind::Ident(name) = &e.kind else { return None };
        if self.lookup_local(name).is_some() {
            return None;
        }
        self.t.enum_by_name.get(name).copied()
    }

    /// `Block.Air`: a variant without fields.
    pub(crate) fn variant_value(&mut self, e: &Expr, id: EnumId, name: &str, name_span: Span) -> Ty {
        let info = self.t.enm(id);
        let enum_name = info.name.clone();
        let Some(v) = info.variants.iter().position(|v| v.name == name) else {
            self.error(name_span, format!("`{enum_name}` has no variant `{name}`"));
            return Ty::Error;
        };
        if !info.variants[v].fields.is_empty() {
            self.error(name_span, format!("`{name}` has fields; build it with `{enum_name}.{name}(...)`"));
            return Ty::Error;
        }
        self.t.res.insert(e.id, Res::Variant(id, v as u32));
        Ty::Enum(id)
    }

    /// `Block.Stone(hardness: 3)`: arguments bind to the variant's fields.
    pub(crate) fn variant_construct(&mut self, e: &Expr, id: EnumId, name: &str, name_span: Span, args: &[Arg]) -> Ty {
        let info = self.t.enm(id);
        let enum_name = info.name.clone();
        let Some(v) = info.variants.iter().position(|v| v.name == name) else {
            self.error(name_span, format!("`{enum_name}` has no variant `{name}`"));
            for a in args {
                self.check_expr(&a.value, None);
            }
            return Ty::Error;
        };
        let variant = info.variants[v].clone();
        if variant.fields.is_empty() {
            self.error(e.span, format!("`{name}` has no fields; write `{enum_name}.{name}`"));
            for a in args {
                self.check_expr(&a.value, None);
            }
            return Ty::Error;
        }
        self.t.res.insert(e.id, Res::Variant(id, v as u32));
        let names: Vec<&str> = variant.fields.iter().map(|f| f.name.as_str()).collect();
        let (slots, bound) = self.bind_args(name, &names, args, "field");
        let mut ok = bound;
        for (fi, slot) in slots.iter().enumerate() {
            match *slot {
                Some(ai) => {
                    let a = &args[ai];
                    if a.is_var {
                        self.error(a.span, "constructor arguments can't be `var`");
                    }
                    self.check_value(&a.value, Some(variant.fields[fi].ty));
                }
                None if bound => {
                    self.error(e.span, format!("missing field `{}` for `{name}`", variant.fields[fi].name));
                    ok = false;
                }
                None => {}
            }
        }
        for (ai, a) in args.iter().enumerate() {
            if !slots.contains(&Some(ai)) {
                self.check_expr(&a.value, None);
            }
        }
        if ok {
            self.t.ctor_args.insert(e.id, slots);
        }
        Ty::Enum(id)
    }

    /// The base of a field access. A `match` binding is allowed here (and
    /// only here): it stands for the matched variant's fields.
    pub(crate) fn check_field_base(&mut self, base: &Expr) -> Ty {
        if let ExprKind::Ident(name) = &base.kind {
            if let Some(l) = self.lookup_local(name) {
                let ty = self.t.local(l).ty;
                if let Ty::Variant(..) = ty {
                    self.t.res.insert(base.id, Res::Local(l));
                    self.t.expr_ty[base.id.0 as usize] = ty;
                    return ty;
                }
            }
        }
        self.check_expr(base, None)
    }
}
