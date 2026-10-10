//! Constants: top-level `let NAME = ...`, whose initializers may only use
//! literals, operators, pure built-ins, struct constructions and earlier
//! constants.

use kestrel_syntax::ast::{Arg, Expr, ExprKind, Item, Program};
use kestrel_syntax::Diagnostic;

use super::BodyChecker;
use crate::builtins::Builtin;
use crate::collect::resolve_type;
use crate::tables::{ConstId, Tables, BUILTIN_STRUCTS};
use crate::ty::Ty;

pub(crate) fn check_consts(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let consts: Vec<_> = program
        .items
        .iter()
        .filter_map(|it| match it {
            Item::Const(c) => Some(c),
            Item::Fn(_) | Item::Struct(_) | Item::Enum(_) | Item::Use(_) => None,
        })
        .enumerate()
        .collect();
    // A module's constants (`draw.RED`) come first, so the entry file's
    // constants can use them (`let BG = draw.BLACK`). Each group keeps its
    // source order; one module's constants can't use another's yet.
    let (modules, entry): (Vec<_>, Vec<_>) = consts.into_iter().partition(|(_, c)| c.bind.name.contains('.'));
    for (i, c) in modules.into_iter().chain(entry) {
        let mut bc = BodyChecker::new(t, diags, None, Ty::Unit, false);
        let want = c.ty.as_ref().map(|ty| resolve_type(ty, bc.t, bc.diags));
        let ty = if is_const_expr(&c.value, bc.t, true) {
            bc.check_value(&c.value, want)
        } else {
            bc.error(c.value.span, "constants can only use literals, operators and earlier constants");
            Ty::Error
        };
        t.consts[i].ty = ty;
        t.const_by_name.insert(c.bind.name.clone(), ConstId(i as u32));
    }
}

/// Whether `e` is a constant expression. A constant may build structs
/// (`user_structs`); a field default may build only vectors, because a default
/// is built again at every construction: one that builds a struct could
/// build itself forever, and one that holds memory could outlive its arena.
pub(crate) fn is_const_expr(e: &Expr, t: &Tables, user_structs: bool) -> bool {
    let all = |args: &[Arg]| args.iter().all(|a| is_const_expr(&a.value, t, user_structs));
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::None | ExprKind::Str(_) | ExprKind::Ident(_) => true,
        ExprKind::Call { name, args, .. } if name == "Arena" => args.is_empty(),
        // Pure built-ins (`sqrt(2.0)`) are fine; the random ones change state.
        ExprKind::Call { name, args, .. } if Builtin::global(name).is_some_and(Builtin::is_pure) => all(args),
        // A struct construction (`Color(1, 0, 0)`, `vec2(0, 1)`) is a value like any other.
        ExprKind::Call { name, args, .. } if t.struct_by_name.get(name).is_some_and(|s| user_structs || s.0 < BUILTIN_STRUCTS) => all(args),
        ExprKind::Unary { expr, .. } => is_const_expr(expr, t, user_structs),
        ExprKind::Binary { lhs, rhs, .. } => is_const_expr(lhs, t, user_structs) && is_const_expr(rhs, t, user_structs),
        // Each use builds a fresh array, like any other value.
        ExprKind::ArrayLit(items) => items.iter().all(|i| is_const_expr(i, t, user_structs)),
        ExprKind::MapLit(entries) => entries.iter().all(|(k, v)| is_const_expr(k, t, user_structs) && is_const_expr(v, t, user_structs)),
        ExprKind::TypeCall { args, .. } => all(args),
        ExprKind::Interp(_)
        | ExprKind::Index { .. }
        | ExprKind::Call { .. }
        | ExprKind::If { .. }
        | ExprKind::Field { .. }
        | ExprKind::MethodCall { .. }
        | ExprKind::Match { .. } => false,
    }
}

/// Whether the constant expression `e` names a constant that is, or holds, a
/// struct of the program's own (not a vector).
pub(crate) fn uses_struct_constant(e: &Expr, t: &Tables) -> bool {
    let any = |args: &[Arg]| args.iter().any(|a| uses_struct_constant(&a.value, t));
    match &e.kind {
        ExprKind::Ident(name) => t.const_by_name.get(name).is_some_and(|c| holds_user_struct(t, t.consts[c.0 as usize].ty)),
        ExprKind::Call { args, .. } | ExprKind::TypeCall { args, .. } => any(args),
        ExprKind::Unary { expr, .. } => uses_struct_constant(expr, t),
        ExprKind::Binary { lhs, rhs, .. } => uses_struct_constant(lhs, t) || uses_struct_constant(rhs, t),
        ExprKind::ArrayLit(items) => items.iter().any(|i| uses_struct_constant(i, t)),
        ExprKind::MapLit(entries) => entries.iter().any(|(k, v)| uses_struct_constant(k, t) || uses_struct_constant(v, t)),
        _ => false,
    }
}

fn holds_user_struct(t: &Tables, ty: Ty) -> bool {
    match ty {
        Ty::Struct(s) => s.0 >= BUILTIN_STRUCTS,
        Ty::Array(e) | Ty::List(e) | Ty::Optional(e) | Ty::Map(_, e) => holds_user_struct(t, t.elem(e)),
        Ty::Enum(_) => true,
        _ => false,
    }
}
