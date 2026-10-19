//! Resolving written types, map types, and the check that a type can't contain itself.

use std::collections::HashSet;

use kestrel_syntax::ast::{TypeExpr, TypeKind};
use kestrel_syntax::{Diagnostic, Span};

use crate::sig::{Sig, SigParam};
use crate::tables::{EnumId, StructId, Tables};
use crate::ty::Ty;

/// `((int) -> bool)?` can't be told from a function that takes a function, so an optional
/// function type isn't offered; a do-nothing default does the same job.
const OPTIONAL_FN: &str =
    "optional function types aren't supported yet; give the field a do-nothing default instead: `on_hit: (int) = (n) { }`";

pub(crate) fn resolve_type(ty: &TypeExpr, t: &mut Tables, diags: &mut Vec<Diagnostic>) -> Ty {
    let msg = match &ty.kind {
        TypeKind::Named { name, args } if args.is_empty() => match Ty::from_name(name) {
            Some(Ty::Gpu) if !t.in_builtin(ty.span) => "`GpuHandle` is only for Kestrel's built-in modules".to_string(),
            Some(found) => return found,
            None => {
                if let Some(&id) = t.struct_by_name.get(name) {
                    return Ty::Struct(id);
                }
                if let Some(&id) = t.enum_by_name.get(name) {
                    return Ty::Enum(id);
                }
                if let Some(&id) = t.iface_by_name.get(name) {
                    if t.iface_ty_ok {
                        return Ty::Iface(id);
                    }
                    diags.push(Diagnostic::error(ty.span, "interface types can't be used yet"));
                    return Ty::Error;
                }
                format!("unknown type `{name}`")
            }
        },
        TypeKind::Named { name, args } if name == "list" && args.len() == 1 => {
            let elem = resolve_type(&args[0], t, diags);
            return if elem == Ty::Error { Ty::Error } else { Ty::List(t.intern_elem(elem)) };
        }
        TypeKind::Named { name, .. } if name == "list" => "`list[...]` takes one element type".to_string(),
        TypeKind::Named { name, args } if name == "map" && args.len() == 2 => {
            let k = resolve_type(&args[0], t, diags);
            let v = resolve_type(&args[1], t, diags);
            return map_ty(k, v, args[0].span, ty.span, t, diags);
        }
        TypeKind::Named { name, .. } if name == "map" => "`map[...]` takes a key type and a value type".to_string(),
        TypeKind::Named { name, .. } if name == "array" => "write array types as `[T]`".to_string(),
        TypeKind::Named { name, .. } => format!("`{name}[...]` types aren't supported yet"),
        TypeKind::Array(inner) => {
            let elem = resolve_type(inner, t, diags);
            return if elem == Ty::Error { Ty::Error } else { Ty::Array(t.intern_elem(elem)) };
        }
        TypeKind::Optional(inner) => {
            let elem = resolve_type(inner, t, diags);
            if matches!(elem, Ty::Fn(_)) {
                OPTIONAL_FN.to_string()
            } else {
                return if elem == Ty::Error { Ty::Error } else { Ty::Optional(t.intern_elem(elem)) };
            }
        }
        TypeKind::Fn { params, ret, is_pure } => {
            let params: Vec<SigParam> =
                params.iter().map(|p| SigParam { ty: resolve_type(&p.ty, t, diags), is_var: p.is_var }).collect();
            let ret = ret.as_ref().map_or(Ty::Unit, |r| resolve_type(r, t, diags));
            if ret == Ty::Error || params.iter().any(|p| p.ty == Ty::Error) {
                return Ty::Error;
            }
            return Ty::Fn(t.intern_sig(Sig { params, ret, is_pure: *is_pure }));
        }
    };
    diags.push(Diagnostic::error(ty.span, msg));
    Ty::Error
}

/// `map[k, v]`, once `k` is known to work as a key (else an error at
/// `key_span`). Value types are checked by `check_map_values`, which needs
/// to know which types own an arena.
pub(crate) fn map_ty(k: Ty, v: Ty, key_span: Span, span: Span, t: &mut Tables, diags: &mut Vec<Diagnostic>) -> Ty {
    if k == Ty::Error || v == Ty::Error {
        return Ty::Error;
    }
    if !(k.is_int() || matches!(k, Ty::Bool | Ty::Str)) {
        let msg = if k.is_float() {
            format!("map keys must be integers, bools or strings; `{}` keys would be unreliable", t.ty_name(k))
        } else {
            format!("map keys must be integers, bools or strings, not `{}`", t.ty_name(k))
        };
        diags.push(Diagnostic::error(key_span, msg));
        return Ty::Error;
    }
    let ty = Ty::Map(t.intern_elem(k), t.intern_elem(v));
    t.map_types.push((ty, span));
    ty
}

/// Reports map types whose values own an arena or a GPU object (once per type).
pub(crate) fn check_map_values(t: &Tables, diags: &mut Vec<Diagnostic>) {
    let mut seen = HashSet::new();
    for &(ty, span) in &t.map_types {
        let Ty::Map(_, v) = ty else { continue };
        let v = t.elem(v);
        if t.is_owning(v) && seen.insert(ty) {
            let what = if t.holds_arena(v) { "an arena" } else { "a GPU object" };
            let msg = format!(
                "maps can't hold values that own {what} yet (`{}` has one); keep them in a list and map keys to indexes",
                t.ty_name(v)
            );
            diags.push(Diagnostic::error(span, msg));
        }
    }
}

/// The types a value of `ty` holds directly inside it. An optional holds its
/// payload in place, so `Node?` inside `Node` is as endless as `Node` is; an
/// array, list or map keeps its items elsewhere, so those can hold their own type.
fn contained(t: &Tables, ty: Ty) -> Vec<Ty> {
    match ty {
        Ty::Optional(e) => vec![t.elem(e)],
        Ty::Struct(s) => t.strukt(s).fields.iter().map(|f| f.ty).collect(),
        Ty::Enum(e) => t.enm(e).variants.iter().flat_map(|v| v.fields.iter().map(|f| f.ty)).collect(),
        _ => Vec::new(),
    }
}

/// How to build a recursive type anyway.
fn self_help(name: &str) -> String {
    format!("to make `{name}` recursive, hold a `list[{name}]` or an array `[{name}]` instead of a `{name}` or `{name}?`; they keep their items elsewhere")
}

/// A type can't hold itself, directly or through others: it would be infinitely large.
pub(crate) fn check_self_containment(t: &Tables, diags: &mut Vec<Diagnostic>) {
    fn reaches(t: &Tables, from: Ty, target: Ty, seen: &mut Vec<Ty>) -> bool {
        for inner in contained(t, from) {
            if inner == target {
                return true;
            }
            if !seen.contains(&inner) {
                seen.push(inner);
                if reaches(t, inner, target, seen) {
                    return true;
                }
            }
        }
        false
    }
    for (i, s) in t.structs.iter().enumerate() {
        let ty = Ty::Struct(StructId(i as u32));
        if reaches(t, ty, ty, &mut Vec::new()) {
            diags.push(Diagnostic::error(s.span, format!("struct `{}` contains itself", s.name)).with_help(self_help(&s.name)));
        }
    }
    for (i, e) in t.enums.iter().enumerate() {
        let ty = Ty::Enum(EnumId(i as u32));
        if reaches(t, ty, ty, &mut Vec::new()) {
            diags.push(Diagnostic::error(e.span, format!("enum `{}` contains itself", e.name)).with_help(self_help(&e.name)));
        }
    }
}
