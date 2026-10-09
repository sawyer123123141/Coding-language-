use std::collections::HashSet;

use kestrel_syntax::ast::{Item, Program, TypeExpr, TypeKind};
use kestrel_syntax::{Diagnostic, Span};

use crate::tables::{ConstInfo, FnId, FnInfo, ParamSig, Tables};
use crate::ty::Ty;

const BUILTINS: &[&str] = &["print", "int", "i64", "u8", "float", "f64", "bool", "string"];

pub(crate) fn resolve_type(ty: &TypeExpr, diags: &mut Vec<Diagnostic>) -> Ty {
    let msg = match &ty.kind {
        TypeKind::Named { name, args } if args.is_empty() => match Ty::from_name(name) {
            Some(Ty::Str) => "string values aren't supported yet (only as `print` arguments)".to_string(),
            Some(t) => return t,
            None => format!("unknown type `{name}`"),
        },
        TypeKind::Named { name, .. } => format!("`{name}[...]` types aren't supported yet"),
        TypeKind::Array(_) => "array types aren't supported yet".to_string(),
        TypeKind::Optional(_) => "optional types aren't supported yet".to_string(),
    };
    diags.push(Diagnostic::error(ty.span, msg));
    Ty::Error
}

/// Records every function signature and constant name before bodies are checked.
pub(crate) fn collect(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let mut seen: HashSet<String> = HashSet::new();
    for (item_idx, item) in program.items.iter().enumerate() {
        let (name, span) = match item {
            Item::Fn(f) => (&f.name, f.name_span),
            Item::Const(c) => (&c.bind.name, c.bind.span),
            Item::Struct(s) => {
                diags.push(Diagnostic::error(s.name_span, "structs aren't supported yet"));
                continue;
            }
        };
        if BUILTINS.contains(&name.as_str()) {
            diags.push(Diagnostic::error(span, format!("`{name}` is a built-in name; pick another")));
        } else if !seen.insert(name.clone()) {
            diags.push(Diagnostic::error(span, format!("`{name}` is defined twice")));
        }
        match item {
            Item::Fn(f) => {
                let mut params = Vec::new();
                for p in &f.params {
                    if f.is_pure && p.is_var {
                        diags.push(Diagnostic::error(p.bind.span, "pure functions can't take `var` parameters"));
                    }
                    let ty = resolve_type(&p.ty, diags);
                    params.push(ParamSig { name: p.bind.name.clone(), ty, is_var: p.is_var });
                }
                let ret = f.ret.as_ref().map_or(Ty::Unit, |r| resolve_type(r, diags));
                let id = FnId(t.fns.len() as u32);
                if f.name == "main" && t.main.is_none() {
                    if !f.params.is_empty() || f.ret.is_some() {
                        diags.push(Diagnostic::error(f.name_span, "`main` takes no parameters and returns nothing"));
                    }
                    t.main = Some(id);
                }
                t.fn_by_name.entry(f.name.clone()).or_insert(id);
                t.fns.push(FnInfo {
                    name: f.name.clone(),
                    item: item_idx,
                    is_pure: f.is_pure,
                    params,
                    ret,
                    span: f.name_span,
                    locals: Vec::new(),
                    io: false,
                    mutates_params: false,
                });
            }
            Item::Struct(_) => {}
            Item::Const(c) => {
                t.consts.push(ConstInfo { name: c.bind.name.clone(), ty: Ty::Error, item: item_idx });
            }
        }
    }
    if t.main.is_none() {
        diags.push(Diagnostic::error(Span::new(0, 0), "program needs a `main()` function"));
    }
}
