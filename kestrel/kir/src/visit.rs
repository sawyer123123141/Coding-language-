//! Walks KIR, reporting every statement, expression and place in it.

use crate::ir::*;

pub enum Node<'a> {
    Stmt(&'a Stmt),
    Expr(&'a Expr),
    /// A place and whether it's written (assigned, passed as `var`, moved from).
    Place(&'a Place, bool),
}

/// Calls `f` for each statement and expression in `stmts`, outermost
/// first, and for each place (after the index expressions inside it).
pub fn walk<'a>(stmts: &'a [Stmt], f: &mut impl FnMut(Node<'a>)) {
    for s in stmts {
        stmt(s, f);
    }
}

/// Calls `f(place, written)` for each place in `stmts`. A place passed as
/// a `var` argument counts as written.
pub fn places(stmts: &[Stmt], f: &mut impl FnMut(&Place, bool)) {
    walk(stmts, &mut |n| {
        if let Node::Place(p, written) = n {
            f(p, written);
        }
    });
}

fn stmt<'a>(s: &'a Stmt, f: &mut impl FnMut(Node<'a>)) {
    f(Node::Stmt(s));
    match s {
        Stmt::Let(_, e) | Stmt::Return(Some(e)) | Stmt::RandSeed(e) => expr(e, f),
        Stmt::Assign(p, e) | Stmt::ListAdd(p, e) => {
            expr(e, f);
            place(p, true, f);
        }
        Stmt::Bind(_, p) => place(p, false, f),
        Stmt::MapSet(p, k, v, _) => {
            expr(k, f);
            expr(v, f);
            place(p, true, f);
        }
        Stmt::MapUnlock(p, saved) => {
            expr(saved, f);
            place(p, true, f);
        }
        Stmt::ParFor { args: a, count, .. } => {
            expr(count, f);
            args(a, f);
        }
        Stmt::Dup(p) | Stmt::FreeArena(p) | Stmt::DropGpu(p) => place(p, true, f),
        Stmt::PushArena(Some(p)) => place(p, true, f),
        Stmt::PushArena(None) | Stmt::PopArena { .. } => {}
        Stmt::If { cond, then_body, else_body } => {
            expr(cond, f);
            walk(then_body, f);
            walk(else_body, f);
        }
        Stmt::While { cond, body } => {
            expr(cond, f);
            walk(body, f);
        }
        Stmt::ForRange { start, end, body, .. } => {
            expr(start, f);
            expr(end, f);
            walk(body, f);
        }
        Stmt::Call(_, args) => self::args(args, f),
        Stmt::Print(parts) | Stmt::Write(parts) => print_args(parts, f),
        Stmt::Break | Stmt::Continue | Stmt::Return(None) => {}
    }
}

fn place<'a>(p: &'a Place, written: bool, f: &mut impl FnMut(Node<'a>)) {
    for at in p.path.iter().filter_map(Proj::at) {
        expr(at, f);
    }
    f(Node::Place(p, written));
}

fn args<'a>(args: &'a [Arg], f: &mut impl FnMut(Node<'a>)) {
    for a in args {
        match a {
            Arg::Value(e) => expr(e, f),
            Arg::Ref(p) => place(p, true, f),
        }
    }
}

fn print_args<'a>(parts: &'a [PrintArg], f: &mut impl FnMut(Node<'a>)) {
    for p in parts {
        match p {
            PrintArg::Value(e) => expr(e, f),
            PrintArg::Interp(inner) => print_args(inner, f),
            PrintArg::Str(_) => {}
        }
    }
}

fn expr<'a>(e: &'a Expr, f: &mut impl FnMut(Node<'a>)) {
    f(Node::Expr(e));
    match &e.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::StrLit(_)
        | ExprKind::ListNew
        | ExprKind::MapNew
        | ExprKind::NewArena
        | ExprKind::NoneLit
        | ExprKind::RandFloat => {}
        ExprKind::Move(p) => place(p, true, f),
        ExprKind::Map(op, p, xs) => {
            for x in xs {
                expr(x, f);
            }
            place(p, matches!(op, MapOp::Remove | MapOp::Lock), f);
        }
        ExprKind::Format(parts) => print_args(parts, f),
        ExprKind::Load(p) | ExprKind::Tag(p) | ExprKind::Len(p) => place(p, false, f),
        ExprKind::Unary(_, x) | ExprKind::Cast(x) | ExprKind::FieldOf(x, _) | ExprKind::ArrayNew(x) | ExprKind::Some(x) => expr(x, f),
        ExprKind::Binary(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) | ExprKind::RandInt(a, b) => {
            expr(a, f);
            expr(b, f);
        }
        ExprKind::Call(_, a) => args(a, f),
        ExprKind::StructLit(_, xs) | ExprKind::VariantLit(_, _, xs) | ExprKind::ArrayLit(xs) | ExprKind::Math(_, xs) => {
            for x in xs {
                expr(x, f);
            }
        }
        ExprKind::Seq(stmts, v) => {
            walk(stmts, f);
            expr(v, f);
        }
        ExprKind::If { cond, then_body, then_val, else_body, else_val } => {
            expr(cond, f);
            walk(then_body, f);
            walk(else_body, f);
            for v in [then_val, else_val].into_iter().flatten() {
                expr(v, f);
            }
        }
    }
}
