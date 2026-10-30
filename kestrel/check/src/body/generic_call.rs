//! Generics used with their types written (G8): `best[Enemy](foes)`, `Stack[int](items: [1])`,
//! `let f = best[Enemy]`. A user `TypeCall` is checked like the matching `Call`: a generic
//! struct gives a construction of its instance, a generic function a call against its signature
//! with the written types put in. Every use records its types as `type_args` for lowering.
//!
//! A generic called or built without types has them worked out (`infer.rs`, `inferred.rs`).

use kestrel_syntax::ast::{Expr, ExprKind, TypeExpr};
use kestrel_syntax::Span;

use super::args::CallSig;
use super::BodyChecker;
use crate::collect_ty::TypeScope;
use crate::generic::{GenericOwner, Subst, TyListId};
use crate::tables::{EnumId, FnId, Res};
use crate::ty::Ty;

/// Whether `e` is a `TypeCall` of a user name (`best[int](xs)`, `Stack[int]()`), not `array`,
/// `list` or `map`. It is handled like the `Call` it stands for (P28).
pub(crate) fn user_type_call(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::TypeCall { name, .. } if !matches!(name.as_str(), "array" | "list" | "map"))
}

impl<'a> BodyChecker<'a> {
    /// `Name[types](args)` for a user name. A plain struct or function is "`Enemy` takes no types".
    pub(crate) fn check_user_type_call(&mut self, e: &Expr) -> Ty {
        let ExprKind::TypeCall { name, name_span, type_args, args } = &e.kind else { unreachable!("not a type call") };
        if let Some(&sid) = self.t.struct_by_name.get(name) {
            let Some(list) = self.written_types(GenericOwner::Struct(sid), name, *name_span, type_args) else {
                self.check_loose_args(args);
                return Ty::Error;
            };
            // The instance's fields are filled, so labels, defaults and binding work as for any struct.
            let instance = self.t.intern_struct(sid, list);
            self.t.generic.type_args.insert(e.id, list);
            return self.check_construct(e, instance, args);
        }
        if let Some(&fid) = self.t.fn_by_name.get(name) {
            let owner = GenericOwner::Fn(fid);
            let Some(list) = self.written_types(owner, name, *name_span, type_args) else {
                self.check_loose_args(args);
                return Ty::Error;
            };
            self.t.generic.type_args.insert(e.id, list);
            self.t.res.insert(e.id, Res::Fn(fid));
            self.check_callee_purity(*name_span, fid);
            let sig = self.subst_call_sig(CallSig::of(self.t, fid), Subst { owner, args: list });
            return self.check_args(e, sig, args, None, None);
        }
        if let Some(&eid) = self.t.enum_by_name.get(name) {
            let prefix = if self.t.enm(eid).params.is_empty() { name.to_string() } else { self.int_types(GenericOwner::Enum(eid), name) };
            let example = self.variant_example(eid, &prefix);
            self.error_help(
                *name_span,
                format!("`{name}` is an enum, so it can't be called like a struct"),
                format!("build a value from one of its variants, like {example}"),
            );
        } else if self.t.iface_by_name.contains_key(name) {
            self.error(*name_span, format!("`{name}` takes no types"));
        } else {
            self.error(*name_span, format!("unknown struct or function `{name}`"));
        }
        self.check_loose_args(args);
        Ty::Error
    }

    /// `Name[types]` as a value: a generic function is a function value with its types fixed (R8);
    /// a struct or enum is a type, not a value.
    pub(crate) fn check_type_apply(&mut self, e: &Expr) -> Ty {
        let ExprKind::TypeApply { name, name_span, type_args } = &e.kind else { unreachable!("not a type application") };
        if let Some(&fid) = self.t.fn_by_name.get(name) {
            let owner = GenericOwner::Fn(fid);
            // `scores[i]` of a plain function was meant as an index of what it returns.
            if self.t.func(fid).type_params.is_empty() {
                let args = if self.t.func(fid).params.is_empty() { "" } else { "..." };
                self.error_help(*name_span, format!("`{name}` takes no types"), format!("to index what `{name}` returns, call it first: `{name}({args})[...]`"));
                return Ty::Error;
            }
            let Some(list) = self.written_types(owner, name, *name_span, type_args) else { return Ty::Error };
            self.t.generic.type_args.insert(e.id, list);
            self.t.res.insert(e.id, Res::FnValue(fid));
            self.note_fn_value(fid, e.span);
            let sig = self.t.subst_sig(self.t.fn_sig(fid), Subst { owner, args: list });
            return Ty::Fn(sig);
        }
        let owner = match (self.t.struct_by_name.get(name), self.t.enum_by_name.get(name)) {
            (Some(&sid), _) => GenericOwner::Struct(sid),
            (_, Some(&eid)) => GenericOwner::Enum(eid),
            _ => {
                let msg = if self.t.iface_by_name.contains_key(name) { format!("`{name}` takes no types") } else { format!("unknown name `{name}`") };
                self.error(*name_span, msg);
                return Ty::Error;
            }
        };
        let Some(list) = self.written_types(owner, name, *name_span, type_args) else { return Ty::Error };
        let shown = self.written_name(name, list);
        // An enum is built from a variant, like the variant help says.
        let example = match owner {
            GenericOwner::Enum(eid) => self.variant_example(eid, &shown),
            _ => format!("`{shown}(...)`"),
        };
        self.error(e.span, format!("`{shown}` is a type; build one with {example}"));
        Ty::Error
    }

    /// For a name that isn't one but is a variant of one or more enums: the message that says
    /// how to write it, "`Slime` is a variant of `Enemy`: write `Enemy.Slime(...)`" (with `(...)`
    /// when it has fields, and the types written for a generic enum's variant without fields,
    /// which they can't be worked out for).
    pub(crate) fn bare_variant(&self, name: &str) -> Option<String> {
        let mut owners: Vec<(&String, EnumId, bool)> = self
            .t
            .enum_by_name
            .iter()
            .filter_map(|(en, &eid)| self.t.enm(eid).variants.iter().find(|v| v.name == name).map(|v| (en, eid, v.fields.is_empty())))
            .collect();
        owners.sort_by_key(|(en, ..)| *en);
        if owners.is_empty() {
            return None;
        }
        if let [(en, eid, true)] = owners[..] {
            if !self.t.enm(eid).params.is_empty() {
                let typed = self.int_types(GenericOwner::Enum(eid), en);
                return Some(format!("`{name}` is a variant of `{en}`: write it with its types, like `{typed}.{name}`"));
            }
        }
        let written = |(en, _, plain): &(&String, EnumId, bool)| format!("`{en}.{name}{}`", if *plain { "" } else { "(...)" });
        let enums: Vec<String> = owners.iter().map(|(en, ..)| format!("`{en}`")).collect();
        let ways: Vec<String> = owners.iter().map(written).collect();
        Some(format!("`{name}` is a variant of {}: write {}", either(&enums, "and"), either(&ways, "or")))
    }

    /// How to build a value of enum `eid` written `prefix` (`Maybe`, `Maybe[int]`): from its
    /// first variant, "`Maybe[int].Nothing`" or "`Result[int, string].Ok(...)`".
    pub(crate) fn variant_example(&self, eid: EnumId, prefix: &str) -> String {
        match self.t.enm(eid).variants.first() {
            Some(v) if v.fields.is_empty() => format!("`{prefix}.{}`", v.name),
            Some(v) => format!("`{prefix}.{}(...)`", v.name),
            None => format!("`{prefix}.Variant` once it has a variant"),
        }
    }

    /// The types written for `owner` (`name`'s placeholders), once there are as many as it takes
    /// and each meets R3 and its rules; `None` after the error. They are checked at the type
    /// written (G20).
    pub(crate) fn written_types(&mut self, owner: GenericOwner, name: &str, name_span: Span, type_args: &[TypeExpr]) -> Option<TyListId> {
        let (count, found) = (self.t.owner_params(owner).len(), type_args.len());
        let msg = match count {
            0 => Some(format!("`{name}` takes no types")),
            1 if found != 1 => Some(format!("`{name}` takes 1 type, found {found}")),
            n if n != found => Some(format!("`{name}` takes {n} types, found {found}")),
            _ => None,
        };
        if let Some(msg) = msg {
            self.error(name_span, msg);
            return None;
        }
        let tys: Vec<Ty> = type_args.iter().map(|a| self.resolve(a)).collect();
        if tys.contains(&Ty::Error) {
            return None;
        }
        let spans: Vec<Span> = type_args.iter().map(|a| a.span).collect();
        // The same scope `resolve` uses: a type's own bare name means itself inside it (G2).
        let own = self.own.filter(|o| !matches!(o, GenericOwner::Fn(_)));
        let scope = TypeScope { params: &self.type_scope, own };
        if self.t.check_written_args(owner, &tys, &spans, &scope, self.diags) {
            return None;
        }
        Some(self.t.intern_ty_list(tys))
    }

    /// `Stack[int]`: `name` with the types of `list`, as a message writes it.
    pub(super) fn written_name(&self, name: &str, list: TyListId) -> String {
        let tys: Vec<String> = self.t.ty_list(list).iter().map(|&t| self.tn(t)).collect();
        format!("{name}[{}]", tys.join(", "))
    }

    /// `name[int, int]` with `int` for each placeholder of `owner`.
    pub(crate) fn int_types(&self, owner: GenericOwner, name: &str) -> String {
        let ints = vec!["int"; self.t.owner_params(owner).len()].join(", ");
        format!("{name}[{ints}]")
    }

    /// Whether `fid` is a generic function (not a method of a generic type).
    pub(crate) fn is_generic_fn(&self, fid: FnId) -> bool {
        matches!(self.t.func(fid).generic, Some(GenericOwner::Fn(_)))
    }
}

/// `a`, `a and b`, or `a, b and c` (with `or` for `word` "or").
fn either(items: &[String], word: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} {word} {last}", rest.join(", ")),
    }
}
