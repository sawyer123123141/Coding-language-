//! Constants: top-level `let NAME = ...`, whose initializers may only use
//! literals, operators, pure built-ins, struct constructions and earlier
//! constants.

use kestrel_syntax::ast::{Expr, ExprKind, Item, Program};
use kestrel_syntax::Diagnostic;

use super::BodyChecker;
use crate::builtins::Builtin;
use crate::collect::resolve_type;
use crate::tables::{ConstId, Tables};
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
        let ty = if is_const_expr(&c.value, bc.t) {
            bc.check_value(&c.value, want)
        } else {
            bc.error(c.value.span, "constants can only use literals, operators and earlier constants");
            Ty::Error
        };
        t.consts[i].ty = ty;
        t.const_by_name.insert(c.bind.name.clone(), ConstId(i as u32));
    }
}

pub(crate) fn is_const_expr(e: &Expr, t: &Tables) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::None | ExprKind::Str(_) | ExprKind::Ident(_) => true,
        ExprKind::Call { name, args, .. } if name == "Arena" => args.is_empty(),
        // Pure built-ins (`sqrt(2.0)`) are fine; the random ones change state.
        ExprKind::Call { name, args, .. } if Builtin::global(name).is_some_and(Builtin::is_pure) => {
            args.iter().all(|a| is_const_expr(&a.value, t))
        }
        // A struct construction (`Color(1, 0, 0)`, `vec2(0, 1)`) is a value like any other.
        ExprKind::Call { name, args, .. } if t.struct_by_name.contains_key(name) => args.iter().all(|a| is_const_expr(&a.value, t)),
        ExprKind::Unary { expr, .. } => is_const_expr(expr, t),
        ExprKind::Binary { lhs, rhs, .. } => is_const_expr(lhs, t) && is_const_expr(rhs, t),
        // Each use builds a fresh array, like any other value.
        ExprKind::ArrayLit(items) => items.iter().all(|i| is_const_expr(i, t)),
        ExprKind::MapLit(entries) => entries.iter().all(|(k, v)| is_const_expr(k, t) && is_const_expr(v, t)),
        ExprKind::TypeCall { args, .. } => args.iter().all(|a| is_const_expr(&a.value, t)),
        ExprKind::Interp(_)
        | ExprKind::Index { .. }
        | ExprKind::Call { .. }
        | ExprKind::If { .. }
        | ExprKind::Field { .. }
        | ExprKind::MethodCall { .. }
        | ExprKind::Match { .. } => false,
    }
}
