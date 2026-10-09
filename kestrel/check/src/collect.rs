use std::collections::{HashMap, HashSet};

use kestrel_syntax::ast::{EnumDecl, FieldDecl, FnDecl, Item, Program, StructDecl, TypeExpr, TypeKind};
use kestrel_syntax::{Diagnostic, Span};

use crate::tables::{
    ConstInfo, EnumId, EnumInfo, FieldInfo, FnId, FnInfo, ParamSig, StructId, StructInfo, Tables, VariantInfo,
    BUILTIN_STRUCTS,
};
use crate::ty::Ty;

pub(crate) const BUILTINS: &[&str] =
    &["print", "int", "i64", "u8", "float", "f64", "bool", "string", "vec2", "vec3", "array", "list", "map"];

pub(crate) fn resolve_type(ty: &TypeExpr, t: &mut Tables, diags: &mut Vec<Diagnostic>) -> Ty {
    let msg = match &ty.kind {
        TypeKind::Named { name, args } if args.is_empty() => match Ty::from_name(name) {
            Some(found) => return found,
            None => {
                if let Some(&id) = t.struct_by_name.get(name) {
                    return Ty::Struct(id);
                }
                if let Some(&id) = t.enum_by_name.get(name) {
                    return Ty::Enum(id);
                }
                format!("unknown type `{name}`")
            }
        },
        TypeKind::Named { name, args } if name == "list" && args.len() == 1 => {
            let elem = resolve_type(&args[0], t, diags);
            return if elem == Ty::Error { Ty::Error } else { Ty::List(t.intern_elem(elem)) };
        }
        TypeKind::Named { name, .. } if name == "list" => "`list[...]` takes one element type".to_string(),
        TypeKind::Named { name, .. } if name == "array" => "write array types as `[T]`".to_string(),
        TypeKind::Named { name, .. } => format!("`{name}[...]` types aren't supported yet"),
        TypeKind::Array(inner) => {
            let elem = resolve_type(inner, t, diags);
            return if elem == Ty::Error { Ty::Error } else { Ty::Array(t.intern_elem(elem)) };
        }
        TypeKind::Optional(_) => "optional types aren't supported yet".to_string(),
    };
    diags.push(Diagnostic::error(ty.span, msg));
    Ty::Error
}

/// Records every type, function signature and constant before bodies are
/// checked. Type names come first so any type can refer to any other.
pub(crate) fn collect(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    check_names(program, diags);
    add_builtin_vectors(t);
    for (item_idx, item) in program.items.iter().enumerate() {
        match item {
            Item::Struct(s) => {
                let id = StructId(t.structs.len() as u32);
                t.struct_by_name.entry(s.name.clone()).or_insert(id);
                let info = StructInfo { name: s.name.clone(), item: item_idx, span: s.name_span, fields: Vec::new(), methods: HashMap::new() };
                t.structs.push(info);
            }
            Item::Enum(e) => {
                let id = EnumId(t.enums.len() as u32);
                t.enum_by_name.entry(e.name.clone()).or_insert(id);
                let info = EnumInfo { name: e.name.clone(), item: item_idx, span: e.name_span, variants: Vec::new(), methods: HashMap::new() };
                t.enums.push(info);
            }
            Item::Fn(_) | Item::Const(_) => {}
        }
    }
    let (mut struct_id, mut enum_id) = (BUILTIN_STRUCTS, 0);
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
            Item::Enum(e) => {
                add_enum_members(e, item_idx, EnumId(enum_id), t, diags);
                enum_id += 1;
            }
        }
    }
    check_self_containment(t, diags);
    if t.main.is_none() {
        diags.push(Diagnostic::error(Span::new(0, 0), "program needs a `main()` function"));
    }
}

/// `vec2` and `vec3`: structs of floats, always `StructId` 0 and 1.
fn add_builtin_vectors(t: &mut Tables) {
    for (name, comps) in [("vec2", &["x", "y"][..]), ("vec3", &["x", "y", "z"][..])] {
        let id = StructId(t.structs.len() as u32);
        t.struct_by_name.insert(name.to_string(), id);
        let fields = comps
            .iter()
            .map(|c| FieldInfo { name: c.to_string(), ty: Ty::Float, span: Span::default(), has_default: false })
            .collect();
        t.structs.push(StructInfo { name: name.to_string(), item: usize::MAX, span: Span::default(), fields, methods: HashMap::new() });
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
            Item::Enum(e) => (&e.name, e.name_span),
        };
        if BUILTINS.contains(&name.as_str()) {
            diags.push(Diagnostic::error(span, format!("`{name}` is a built-in name; pick another")));
        } else if !seen.insert(name) {
            diags.push(Diagnostic::error(span, format!("`{name}` is defined twice")));
        }
    }
}

/// Resolves field types; reports duplicate field names.
fn fields_of(owner: &str, decls: &[FieldDecl], t: &mut Tables, diags: &mut Vec<Diagnostic>) -> Vec<FieldInfo> {
    let mut fields: Vec<FieldInfo> = Vec::new();
    for f in decls {
        if fields.iter().any(|g| g.name == f.name) {
            diags.push(Diagnostic::error(f.name_span, format!("`{owner}` has two fields named `{}`", f.name)));
            continue;
        }
        let ty = resolve_type(&f.ty, t, diags);
        fields.push(FieldInfo { name: f.name.clone(), ty, span: f.name_span, has_default: f.default.is_some() });
    }
    fields
}

/// Registers methods; `taken` names (fields or variants) can't be reused.
fn methods_of(
    owner: &str,
    item_idx: usize,
    decls: &[FnDecl],
    taken: &[&str],
    taken_noun: &str,
    t: &mut Tables,
    diags: &mut Vec<Diagnostic>,
) -> HashMap<String, FnId> {
    let mut methods = HashMap::new();
    for (mi, m) in decls.iter().enumerate() {
        if taken.contains(&m.name.as_str()) {
            diags.push(Diagnostic::error(m.name_span, format!("`{owner}` already has a {taken_noun} named `{}`", m.name)));
            continue;
        }
        if methods.contains_key(&m.name) {
            diags.push(Diagnostic::error(m.name_span, format!("`{owner}` has two methods named `{}`", m.name)));
            continue;
        }
        let fid = add_fn(m, item_idx, Some((mi, owner)), t, diags);
        methods.insert(m.name.clone(), fid);
    }
    methods
}

fn add_struct_members(s: &StructDecl, item_idx: usize, id: StructId, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let fields = fields_of(&s.name, &s.fields, t, diags);
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    let methods = methods_of(&s.name, item_idx, &s.methods, &names, "field", t, diags);
    let info = &mut t.structs[id.0 as usize];
    info.fields = fields;
    info.methods = methods;
}

fn add_enum_members(e: &EnumDecl, item_idx: usize, id: EnumId, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let mut variants: Vec<VariantInfo> = Vec::new();
    for v in &e.variants {
        if variants.iter().any(|w| w.name == v.name) {
            diags.push(Diagnostic::error(v.name_span, format!("`{}` has two variants named `{}`", e.name, v.name)));
            continue;
        }
        let fields = fields_of(&v.name, &v.fields, t, diags);
        variants.push(VariantInfo { name: v.name.clone(), span: v.name_span, fields });
    }
    let names: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();
    let methods = methods_of(&e.name, item_idx, &e.methods, &names, "variant", t, diags);
    let info = &mut t.enums[id.0 as usize];
    info.variants = variants;
    info.methods = methods;
}

/// Registers a function's signature. `method` is (index in its type, type name).
fn add_fn(f: &FnDecl, item_idx: usize, method: Option<(usize, &str)>, t: &mut Tables, diags: &mut Vec<Diagnostic>) -> FnId {
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
        Some((_, owner)) => format!("{owner}.{}", f.name),
        None => f.name.clone(),
    };
    t.fns.push(FnInfo {
        name,
        item: item_idx,
        method: method.map(|(mi, _)| mi),
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

/// The types a value of `ty` holds directly inside it.
fn contained(t: &Tables, ty: Ty) -> Vec<Ty> {
    match ty {
        Ty::Struct(s) => t.strukt(s).fields.iter().map(|f| f.ty).collect(),
        Ty::Enum(e) => t.enm(e).variants.iter().flat_map(|v| v.fields.iter().map(|f| f.ty)).collect(),
        _ => Vec::new(),
    }
}

/// A type can't hold itself, directly or through others: it would be infinitely large.
fn check_self_containment(t: &Tables, diags: &mut Vec<Diagnostic>) {
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
            diags.push(Diagnostic::error(s.span, format!("struct `{}` contains itself", s.name)));
        }
    }
    for (i, e) in t.enums.iter().enumerate() {
        let ty = Ty::Enum(EnumId(i as u32));
        if reaches(t, ty, ty, &mut Vec::new()) {
            diags.push(Diagnostic::error(e.span, format!("enum `{}` contains itself", e.name)));
        }
    }
}
