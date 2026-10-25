//! Uses of generics with no types written, once `infer` has worked them out: a call of a generic
//! function (`pick(1, 2)`), a construction of a generic struct (`Stack(items: [1])`) and a
//! variant of a generic enum (`Result.Ok(value: 3)`, `Maybe.Nothing` from the expected type, P8).
//! Each records its types as `type_args`, then finishes the arguments against the filled-in
//! parameters as a written use would (P3): `check_args`, `check_construct_with`, or the variant
//! built here with the checked arguments (mirroring `variant_construct`, P20).

use kestrel_syntax::ast::{Arg, Expr};
use kestrel_syntax::Span;

use super::args::CallSig;
use super::infer::{deferred, Callee, Form};
use super::BodyChecker;
use crate::generic::{GenericOwner, Subst};
use crate::tables::{EnumId, FnId, ParamSig, Res, StructId};
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    /// `pick(1, 2)`: a call of the generic function `fid`, written `name`.
    pub(crate) fn check_inferred_call(&mut self, e: &Expr, fid: FnId, name: &str, name_span: Span, args: &[Arg], want: Option<Ty>) -> Ty {
        let sig = CallSig::of(self.t, fid);
        let fname = self.t.func(fid).name.clone();
        let names: Vec<&str> = sig.params.iter().map(|p| p.name.as_str()).collect();
        let (slots, ok) = self.bind_args(&fname, &names, args, "parameter");
        let missing: Vec<&str> = slots.iter().zip(&names).filter(|(s, _)| s.is_none()).map(|(_, &n)| n).collect();
        if ok {
            for m in &missing {
                self.error(e.span, format!("missing argument `{m}` for `{fname}`"));
            }
        }
        if !ok || !missing.is_empty() {
            self.check_undeferred_args(args);
            return Ty::Error;
        }
        self.check_callee_purity(name_span, fid);
        let owner = GenericOwner::Fn(fid);
        let callee = Callee { owner, name, form: Form::Call, at: name_span, params: sig.params.clone(), ret: sig.ret };
        let Ok((list, tys)) = self.infer(&callee, args, &slots, want) else { return Ty::Error };
        self.t.res.insert(e.id, Res::Fn(fid));
        self.t.generic.type_args.insert(e.id, list);
        let filled = self.subst_call_sig(sig, Subst { owner, args: list });
        self.check_args(e, filled, args, None, Some(&tys))
    }

    /// `Stack(items: [1])`: a construction of the generic struct `sid`, written `name`.
    pub(crate) fn check_inferred_construct(&mut self, e: &Expr, sid: StructId, name: &str, name_span: Span, args: &[Arg], want: Option<Ty>) -> Ty {
        let info = self.t.strukt(sid).clone();
        let names: Vec<&str> = info.fields.iter().map(|f| f.name.as_str()).collect();
        let (slots, ok) = self.bind_args(&info.name, &names, args, "field");
        let missing: Vec<String> =
            slots.iter().zip(&info.fields).filter(|(s, f)| s.is_none() && !f.has_default).map(|(_, f)| format!("`{}`", f.name)).collect();
        if !ok || !self.missing_fields(e, &info.name, &missing, false) {
            self.check_undeferred_args(args);
            return Ty::Error;
        }
        let params = info.fields.iter().map(|f| ParamSig { name: f.name.clone(), ty: f.ty, is_var: false }).collect();
        let callee = Callee { owner: GenericOwner::Struct(sid), name, form: Form::Construct, at: name_span, params, ret: Ty::Struct(sid) };
        let Ok((list, tys)) = self.infer(&callee, args, &slots, want) else { return Ty::Error };
        let instance = self.t.intern_struct(sid, list);
        self.t.generic.type_args.insert(e.id, list);
        self.check_construct_with(e, instance, args, Some(&tys))
    }

    /// `Maybe.Nothing`: a variant without fields of the generic enum `def` (written `name` at
    /// `at`), whose instance comes from the expected type (P8).
    pub(super) fn check_inferred_unit(&mut self, e: &Expr, def: EnumId, (name, at): (&str, Span), (variant, variant_span): (&str, Span), want: Option<Ty>) -> Ty {
        if !self.t.enm(def).variants.iter().any(|v| v.name == variant && v.fields.is_empty()) {
            // No such variant, or one with fields: today's messages.
            return self.variant_value(e, def, variant, variant_span);
        }
        let callee = Callee { owner: GenericOwner::Enum(def), name, form: Form::Unit(variant), at, params: Vec::new(), ret: Ty::Enum(def) };
        let Ok((list, _)) = self.infer(&callee, &[], &[], want) else { return Ty::Error };
        let instance = self.t.intern_enum(def, list);
        self.t.generic.type_args.insert(e.id, list);
        self.variant_value(e, instance, variant, variant_span)
    }

    /// `Result.Ok(value: 3)`: a variant with fields of the generic enum `def` (written `name` at
    /// `at`), its types worked out from the arguments and the expected type.
    pub(super) fn check_inferred_variant(&mut self, e: &Expr, def: EnumId, (name, at): (&str, Span), (variant, variant_span): (&str, Span), args: &[Arg], want: Option<Ty>) -> Ty {
        let Some(v) = self.t.enm(def).variants.iter().position(|x| x.name == variant && !x.fields.is_empty()) else {
            // No such variant, or one without fields: today's messages.
            return self.variant_construct(e, def, variant, variant_span, args);
        };
        let fields = self.t.enm(def).variants[v].fields.clone();
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let (slots, ok) = self.bind_args(variant, &names, args, "field");
        let missing: Vec<&str> = slots.iter().zip(&names).filter(|(s, _)| s.is_none()).map(|(_, &n)| n).collect();
        if ok {
            for m in &missing {
                self.error(e.span, format!("missing field `{m}` for `{variant}`"));
            }
        }
        if !ok || !missing.is_empty() {
            self.check_undeferred_args(args);
            return Ty::Error;
        }
        let params = fields.iter().map(|f| ParamSig { name: f.name.clone(), ty: f.ty, is_var: false }).collect();
        let callee = Callee { owner: GenericOwner::Enum(def), name, form: Form::Variant(variant), at, params, ret: Ty::Enum(def) };
        let Ok((list, tys)) = self.infer(&callee, args, &slots, want) else { return Ty::Error };
        let instance = self.t.intern_enum(def, list);
        self.t.generic.type_args.insert(e.id, list);
        // As `variant_construct` builds it, with the arguments already checked.
        let fields = self.t.enm(instance).variants[v].fields.clone();
        self.t.res.insert(e.id, Res::Variant(instance, v as u32));
        for (ai, a) in args.iter().enumerate() {
            let Some(fi) = slots.iter().position(|s| *s == Some(ai)) else { continue };
            if a.is_var {
                self.error(a.span, "constructor arguments can't be `var`");
            }
            // (`infer` moved it as it checked it, in source order.)
            self.finish_value(&a.value, tys[ai], Some(fields[fi].ty));
        }
        self.t.ctor_args.insert(e.id, slots);
        Ty::Enum(instance)
    }

    /// The arguments of a use refused before inference (a missing or unknown argument), checked
    /// as written, except the deferred ones: without their parameter's type they would only add
    /// follow-on errors (P10).
    fn check_undeferred_args(&mut self, args: &[Arg]) {
        for a in args.iter().filter(|a| !deferred(&a.value)) {
            self.check_expr(&a.value, None);
        }
    }
}
