//! Walks KIR, reporting every place it reads or writes.

use crate::ir::*;

/// Calls `f(place, written)` for each place in `stmts`, including places
/// nested in index expressions and in calls inside expressions. A place
/// passed as a `var` argument counts as written.
pub fn places(stmts: &[Stmt], f: &mut impl FnMut(&Place, bool)) {
    for s in stmts {
        stmt(s, f);
    }
}

fn stmt(s: &Stmt, f: &mut impl FnMut(&Place, bool)) {
    match s {
        Stmt::Let(_, e) | Stmt::Return(Some(e)) => expr(e, f),
        Stmt::Assign(p, e) | Stmt::ListAdd(p, e) => {
            expr(e, f);
            place(p, true, f);
        }
        Stmt::Bind(_, p) => place(p, false, f),
        Stmt::Dup(p) => place(p, true, f),
        Stmt::If { cond, then_body, else_body } => {
            expr(cond, f);
            places(then_body, f);
            places(else_body, f);
        }
        Stmt::While { cond, body } => {
            expr(cond, f);
            places(body, f);
        }
        Stmt::ForRange { start, end, body, .. } => {
            expr(start, f);
            expr(end, f);
            places(body, f);
        }
        Stmt::Call(_, args) => self::args(args, f),
        Stmt::Print(parts) | Stmt::Write(parts) => print_args(parts, f),
        Stmt::Break | Stmt::Continue | Stmt::Return(None) => {}
    }
}

fn place(p: &Place, written: bool, f: &mut impl FnMut(&Place, bool)) {
    for proj in &p.path {
        if let Proj::Index { at, .. } = proj {
            expr(at, f);
        }
    }
    f(p, written);
}

fn args(args: &[Arg], f: &mut impl FnMut(&Place, bool)) {
    for a in args {
        match a {
            Arg::Value(e) => expr(e, f),
            Arg::Ref(p) => place(p, true, f),
        }
    }
}

fn print_args(parts: &[PrintArg], f: &mut impl FnMut(&Place, bool)) {
    for p in parts {
        match p {
            PrintArg::Value(e) => expr(e, f),
            PrintArg::Interp(inner) => print_args(inner, f),
            PrintArg::Str(_) => {}
        }
    }
}

fn expr(e: &Expr, f: &mut impl FnMut(&Place, bool)) {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::StrLit(_) | ExprKind::ListNew => {}
        ExprKind::Load(p) | ExprKind::Tag(p) | ExprKind::Len(p) => place(p, false, f),
        ExprKind::Unary(_, x) | ExprKind::Cast(x) | ExprKind::FieldOf(x, _) | ExprKind::ArrayNew(x) => expr(x, f),
        ExprKind::Binary(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) => {
            expr(a, f);
            expr(b, f);
        }
        ExprKind::Call(_, a) => args(a, f),
        ExprKind::StructLit(_, xs) | ExprKind::VariantLit(_, _, xs) | ExprKind::ArrayLit(xs) => {
            for x in xs {
                expr(x, f);
            }
        }
        ExprKind::Seq(stmts, v) => {
            places(stmts, f);
            expr(v, f);
        }
        ExprKind::If { cond, then_body, then_val, else_body, else_val } => {
            expr(cond, f);
            places(then_body, f);
            places(else_body, f);
            for v in [then_val, else_val].into_iter().flatten() {
                expr(v, f);
            }
        }
    }
}
