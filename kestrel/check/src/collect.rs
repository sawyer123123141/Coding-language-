use std::collections::{HashMap, HashSet};

use kestrel_syntax::ast::{EnumDecl, Expr, ExprKind, FieldDecl, FnDecl, Item, Program, StructDecl, TypeExpr, TypeKind};
use kestrel_syntax::{Diagnostic, Span};

use crate::builtins::Builtin;
use crate::tables::{
    ConstInfo, EnumId, EnumInfo, FieldInfo, FnId, FnInfo, ParamSig, StructId, StructInfo, Tables, VariantInfo,
    BUILTIN_STRUCTS,
};
use crate::ty::Ty;

pub(crate) const BUILTINS: &[&str] =
    &["print", "int", "i64", "u8", "float", "f64", "bool", "string", "vec2", "vec3", "array", "list", "map", "Arena", "GpuHandle"];

/// Names an item, local or parameter can't take: types and constructors (`BUILTINS`),
/// and every global built-in function or constant.
pub(crate) fn is_builtin_name(name: &str) -> bool {
    BUILTINS.contains(&name) || Builtin::GLOBAL_NAMES.contains(&name)
}

/// The prefix of the names of the functions the compiler makes (`__kestrel_constants`,
/// `__kestrel_copy_N`, ...). A user name with it would collide with one of them.
const COMPILER_PREFIX: &str = "__kestrel_";

/// Whether no item, local or parameter may take `name`: a built-in name, or one that starts with
/// `COMPILER_PREFIX` (see `reserved_name_error`).
pub fn is_reserved_name(name: &str) -> bool {
    is_builtin_name(name) || name.starts_with(COMPILER_PREFIX)
}

/// The error for a name an item, local or parameter can't take, if it is one: a built-in name,
/// or one that starts with `COMPILER_PREFIX`. `shown` is the name as the message writes it.
pub(crate) fn reserved_name_error(name: &str, shown: &str) -> Option<String> {
    if is_builtin_name(name) {
        Some(format!("`{shown}` is a built-in name; pick another"))
    } else if name.starts_with(COMPILER_PREFIX) {
        Some(format!("names starting with `{COMPILER_PREFIX}` are reserved for the compiler; pick another"))
    } else {
        None
    }
}

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
            return if elem == Ty::Error { Ty::Error } else { Ty::Optional(t.intern_elem(elem)) };
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

/// Records every type, function signature and constant before bodies are
/// checked. Type names come first so any type can refer to any other.
pub(crate) fn collect(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>, needs_main: bool) {
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
            Item::Fn(_) | Item::Const(_) | Item::Use(_) => {}
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
            Item::Use(_) => {}
        }
    }
    mark_default_allocation(program, t);
    check_self_containment(t, diags);
    crate::natives::check_natives(program, t, diags);
    if t.main.is_none() && needs_main {
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
            .map(|c| FieldInfo { name: c.to_string(), ty: Ty::Float, span: Span::default(), has_default: false, default_allocates: false })
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
            Item::Use(_) => continue,
        };
        // Items from other modules are named `module.name`.
        let own = name.rsplit('.').next().unwrap_or(name);
        if let Some(msg) = reserved_name_error(own, name) {
            diags.push(Diagnostic::error(span, msg));
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
        // `mark_default_allocation` fills in `default_allocates` once every field type is known.
        fields.push(FieldInfo { name: f.name.clone(), ty, span: f.name_span, has_default: f.default.is_some(), default_allocates: false });
    }
    fields
}

/// Sets `FieldInfo::default_allocates` for every struct field with a default. It needs all
/// field types (a struct's heap memory depends on its fields), so it runs after they are collected.
fn mark_default_allocation(program: &Program, t: &mut Tables) {
    let mut struct_id = BUILTIN_STRUCTS as usize;
    for item in &program.items {
        let Item::Struct(s) = item else { continue };
        for decl in &s.fields {
            let Some(default) = &decl.default else { continue };
            let Some(i) = t.structs[struct_id].fields.iter().position(|f| f.name == decl.name) else { continue };
            let ty = t.structs[struct_id].fields[i].ty;
            t.structs[struct_id].fields[i].default_allocates = default_allocates(default, ty, t);
        }
        struct_id += 1;
    }
}

/// Whether building `default` as a value of type `ty` puts memory in the current arena. Anything
/// that holds heap memory does, except the cases that point at no arena: `none`, a string literal
/// (or a string constant, which is one), and an empty array (an empty list still grows in its arena).
fn default_allocates(default: &Expr, ty: Ty, t: &Tables) -> bool {
    if !t.has_heap(ty) {
        return false;
    }
    // A `T?` default is a `T` or `none`.
    let inner = match ty {
        Ty::Optional(e) => t.elem(e),
        other => other,
    };
    match &default.kind {
        ExprKind::None | ExprKind::Str(_) => false,
        ExprKind::Ident(name) if inner == Ty::Str => !t.consts.iter().any(|c| &c.name == name),
        ExprKind::ArrayLit(items) if items.is_empty() => matches!(inner, Ty::List(_)),
        _ => true,
    }
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
    if e.variants.is_empty() {
        diags.push(Diagnostic::error(e.name_span, format!("`{}` needs at least one variant", e.name)));
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
        is_native: f.is_native,
        params,
        ret,
        span: f.name_span,
        locals: Vec::new(),
        // A native may do anything; only `pure` ones promise not to.
        io: f.is_native && !f.is_pure,
        mutates_params: false,
    });
    id
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
