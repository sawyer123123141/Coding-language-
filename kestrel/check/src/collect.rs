use std::collections::{HashMap, HashSet};

use kestrel_syntax::ast::{FnDecl, Item, Program, StructDecl, TypeExpr, TypeKind};
use kestrel_syntax::{Diagnostic, Span};

use crate::tables::{ConstInfo, FieldInfo, FnId, FnInfo, ParamSig, StructId, StructInfo, Tables};
use crate::ty::Ty;

const BUILTINS: &[&str] = &["print", "int", "i64", "u8", "float", "f64", "bool", "string"];

pub(crate) fn resolve_type(ty: &TypeExpr, t: &Tables, diags: &mut Vec<Diagnostic>) -> Ty {
    let msg = match &ty.kind {
        TypeKind::Named { name, args } if args.is_empty() => match Ty::from_name(name) {
            Some(Ty::Str) => "string values aren't supported yet (only as `print` arguments)".to_string(),
            Some(found) => return found,
            None => match t.struct_by_name.get(name) {
                Some(&id) => return Ty::Struct(id),
                None => format!("unknown type `{name}`"),
            },
        },
        TypeKind::Named { name, .. } => format!("`{name}[...]` types aren't supported yet"),
        TypeKind::Array(_) => "array types aren't supported yet".to_string(),
        TypeKind::Optional(_) => "optional types aren't supported yet".to_string(),
    };
    diags.push(Diagnostic::error(ty.span, msg));
    Ty::Error
}

/// Records every struct, function signature and constant before bodies are
/// checked. Struct names come first so any type can refer to any struct.
pub(crate) fn collect(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    check_names(program, diags);
    for (item_idx, item) in program.items.iter().enumerate() {
        if let Item::Struct(s) = item {
            let id = StructId(t.structs.len() as u32);
            t.struct_by_name.entry(s.name.clone()).or_insert(id);
            let info = StructInfo { name: s.name.clone(), item: item_idx, span: s.name_span, fields: Vec::new(), methods: HashMap::new() };
            t.structs.push(info);
        }
    }
    let mut struct_id = 0;
    for (item_idx, item) in program.items.iter().enumerate() {
        match item {
            Item::Fn(f) => {
                let id = add_fn(f, item_idx, None, t, diags);
                if f.name == "main" && t.main.is_none() {
                    if !f.params.is_empty() || f.ret.is_some() {
                        diags.push(Diagnostic::error(f.name_span, "`main` takes no parameters and returns nothing"));
                    }
                    t.main = Some(id);
                }
                t.fn_by_name.entry(f.name.clone()).or_insert(id);
            }
            Item::Const(c) => {
                t.consts.push(ConstInfo { name: c.bind.name.clone(), ty: Ty::Error, item: item_idx });
            }
            Item::Struct(s) => {
                add_struct_members(s, item_idx, StructId(struct_id), t, diags);
                struct_id += 1;
            }
        }
    }
    check_self_containment(t, diags);
    if t.main.is_none() {
        diags.push(Diagnostic::error(Span::new(0, 0), "program needs a `main()` function"));
    }
}

/// Top-level names must be unique and can't reuse a built-in name.
fn check_names(program: &Program, diags: &mut Vec<Diagnostic>) {
    let mut seen: HashSet<&str> = HashSet::new();
    for item in &program.items {
        let (name, span) = match item {
            Item::Fn(f) => (&f.name, f.name_span),
            Item::Const(c) => (&c.bind.name, c.bind.span),
            Item::Struct(s) => (&s.name, s.name_span),
        };
        if BUILTINS.contains(&name.as_str()) {
            diags.push(Diagnostic::error(span, format!("`{name}` is a built-in name; pick another")));
        } else if !seen.insert(name) {
            diags.push(Diagnostic::error(span, format!("`{name}` is defined twice")));
        }
    }
}

fn add_struct_members(s: &StructDecl, item_idx: usize, id: StructId, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let mut fields: Vec<FieldInfo> = Vec::new();
    for f in &s.fields {
        if fields.iter().any(|g| g.name == f.name) {
            diags.push(Diagnostic::error(f.name_span, format!("`{}` has two fields named `{}`", s.name, f.name)));
            continue;
        }
        let ty = resolve_type(&f.ty, t, diags);
        fields.push(FieldInfo { name: f.name.clone(), ty, span: f.name_span, has_default: f.default.is_some() });
    }
    let mut methods = HashMap::new();
    for (mi, m) in s.methods.iter().enumerate() {
        if fields.iter().any(|g| g.name == m.name) {
            diags.push(Diagnostic::error(m.name_span, format!("`{}` already has a field named `{}`", s.name, m.name)));
            continue;
        }
        if methods.contains_key(&m.name) {
            diags.push(Diagnostic::error(m.name_span, format!("`{}` has two methods named `{}`", s.name, m.name)));
            continue;
        }
        let fid = add_fn(m, item_idx, Some((mi, id, &s.name)), t, diags);
        methods.insert(m.name.clone(), fid);
    }
    let info = &mut t.structs[id.0 as usize];
    info.fields = fields;
    info.methods = methods;
}

/// Registers a function's signature. `method` is (index in struct, struct, struct name).
fn add_fn(f: &FnDecl, item_idx: usize, method: Option<(usize, StructId, &str)>, t: &mut Tables, diags: &mut Vec<Diagnostic>) -> FnId {
    let mut params = Vec::new();
    for p in &f.params {
        if f.is_pure && p.is_var {
            diags.push(Diagnostic::error(p.bind.span, "pure functions can't take `var` parameters"));
        }
        let ty = resolve_type(&p.ty, t, diags);
        params.push(ParamSig { name: p.bind.name.clone(), ty, is_var: p.is_var });
    }
    let ret = f.ret.as_ref().map_or(Ty::Unit, |r| resolve_type(r, t, diags));
    let id = FnId(t.fns.len() as u32);
    let name = match method {
        Some((_, _, owner)) => format!("{owner}.{}", f.name),
        None => f.name.clone(),
    };
    t.fns.push(FnInfo {
        name,
        item: item_idx,
        method: method.map(|(mi, _, _)| mi),
        owner: method.map(|(_, sid, _)| sid),
        is_pure: f.is_pure,
        params,
        ret,
        span: f.name_span,
        locals: Vec::new(),
        io: false,
        mutates_params: false,
    });
    id
}

/// A struct can't hold itself, directly or through other structs: it would be infinitely large.
fn check_self_containment(t: &Tables, diags: &mut Vec<Diagnostic>) {
    fn reaches(t: &Tables, from: StructId, target: StructId, seen: &mut Vec<StructId>) -> bool {
        for f in &t.strukt(from).fields {
            if let Ty::Struct(s) = f.ty {
                if s == target {
                    return true;
                }
                if !seen.contains(&s) {
                    seen.push(s);
                    if reaches(t, s, target, seen) {
                        return true;
                    }
                }
            }
        }
        false
    }
    for (i, s) in t.structs.iter().enumerate() {
        let id = StructId(i as u32);
        if reaches(t, id, id, &mut Vec::new()) {
            diags.push(Diagnostic::error(s.span, format!("struct `{}` contains itself", s.name)));
        }
    }
}
