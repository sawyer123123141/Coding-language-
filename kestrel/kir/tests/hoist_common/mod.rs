//! Helpers shared by the hoisting tests: a program lowered (and optimized), and the hoisted
//! locals, loops and math found in one of its functions.
#![allow(dead_code)]

use kestrel_kir::visit::{walk, walk_expr, Node};
use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

pub fn lowered(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(src))
}

pub fn optimized(src: &str) -> Module {
    let mut m = lowered(src);
    optimize(&mut m);
    m
}

pub fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

pub fn par_body(m: &Module) -> &Func {
    &m.funcs[m.par_bodies[0].0 as usize]
}

pub fn is_loop(s: &Stmt) -> bool {
    matches!(s, Stmt::While { .. } | Stmt::ForRange { .. })
}

pub fn body_of(s: &Stmt) -> &[Stmt] {
    match s {
        Stmt::While { body, .. } | Stmt::ForRange { body, .. } => body,
        _ => panic!("not a loop: {s:?}"),
    }
}

/// The first loop in `list`, at its top level.
pub fn first_loop(list: &[Stmt]) -> &Stmt {
    list.iter().find(|s| is_loop(s)).unwrap_or_else(|| panic!("no loop in {list:#?}"))
}

pub fn is_hoisted(f: &Func, l: LocalIdx) -> bool {
    f.locals[l.0 as usize].name == "__hoisted"
}

pub fn hoisted_locals(f: &Func) -> usize {
    f.locals.iter().filter(|l| l.name == "__hoisted").count()
}

/// The initializers of the hoisted locals bound right before the first loop in `list`.
pub fn hoisted_before<'a>(f: &Func, list: &'a [Stmt]) -> Vec<&'a Expr> {
    let at = list.iter().position(is_loop).unwrap_or_else(|| panic!("no loop in {list:#?}"));
    let mut out: Vec<&Expr> = list[..at]
        .iter()
        .rev()
        .map_while(|s| match s {
            Stmt::Let(l, e) if is_hoisted(f, *l) => Some(e),
            _ => None,
        })
        .collect();
    out.reverse();
    out
}

pub fn has_math(e: &Expr, op: MathOp) -> bool {
    let mut found = false;
    walk_expr(e, &mut |n| found |= matches!(n, Node::Expr(Expr { kind: ExprKind::Math(o, _), .. }) if *o == op));
    found
}

/// Whether a `Math(op)` is inside a loop's body or a `while`'s condition anywhere in `stmts`.
pub fn math_in_loops(stmts: &[Stmt], op: MathOp) -> bool {
    let mut found = false;
    walk(stmts, &mut |n| match n {
        Node::Stmt(Stmt::While { cond, body }) => {
            found |= has_math(cond, op) || body.iter().any(|s| stmt_has_math(s, op));
        }
        Node::Stmt(Stmt::ForRange { body, .. }) => found |= body.iter().any(|s| stmt_has_math(s, op)),
        _ => {}
    });
    found
}

pub fn stmt_has_math(s: &Stmt, op: MathOp) -> bool {
    let mut found = false;
    walk(std::slice::from_ref(s), &mut |n| found |= matches!(n, Node::Expr(Expr { kind: ExprKind::Math(o, _), .. }) if *o == op));
    found
}

pub fn math_args(e: &Expr, op: MathOp) -> &[Expr] {
    match &e.kind {
        ExprKind::Math(o, xs) if *o == op => xs,
        _ => panic!("not a {op:?}: {e:#?}"),
    }
}

/// The local a whole-local `Load` reads.
pub fn loaded(e: &Expr) -> LocalIdx {
    match &e.kind {
        ExprKind::Load(p) if p.path.is_empty() => p.root,
        _ => panic!("not a load of a local: {e:#?}"),
    }
}

pub fn name(f: &Func, l: LocalIdx) -> &str {
    &f.locals[l.0 as usize].name
}
