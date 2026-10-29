//! `hoist::hoist_math`: each rule that keeps math in its loop, tested on its own, so that taking
//! the rule out makes its test fail.

mod hoist_common;

use hoist_common::*;
use kestrel_kir::visit::{walk, Node};
use kestrel_kir::*;

fn local(f: &Func, name: &str) -> LocalIdx {
    LocalIdx(f.locals.iter().position(|l| l.name == name).unwrap_or_else(|| panic!("no local `{name}`")) as u32)
}

fn float_load(l: LocalIdx) -> Expr {
    Expr { kind: ExprKind::Load(Place::local(l)), ty: KTy::F32, line: 0 }
}

fn typed(kind: ExprKind, ty: KTy) -> Expr {
    Expr { kind, ty, line: 0 }
}

fn func_mut<'m>(m: &'m mut Module, name: &str) -> &'m mut Func {
    m.funcs.iter_mut().find(|f| f.name == name).unwrap()
}

/// The loop at `f`'s top level, to change.
fn loop_mut(f: &mut Func) -> &mut Stmt {
    f.body.iter_mut().find(|s| matches!(s, Stmt::While { .. } | Stmt::ForRange { .. })).unwrap()
}

#[test]
fn a_write_in_a_while_condition_keeps_math_of_what_it_writes() {
    // `dec` is inlined into the condition, so `x` is a `Value` local the condition writes on every
    // pass: `sin(x)` before the loop would read `x` before the first `dec`.
    let src = "pure dec(var x: float) -> bool {\n x -= 1.0\n x > 0.0\n}\npure f(x0: float) -> float {\n var x = x0\n var s = 0.0\n while dec(var x) {\n  s += sin(x)\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    assert_eq!(f.locals[local(f, "x").0 as usize].kind, LocalKind::Value, "{:#?}", f.body);
    assert!(math_in_loops(&f.body, MathOp::Sin), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn a_temporary_used_in_a_while_condition_is_not_seen_through() {
    // Built as KIR: `while s < exp(t) { let t = k; s += 1.0 }`, with `t` set to `t0` before the
    // loop. The condition's first pass reads `t0`, so `exp(t)` is not `exp(k)`.
    let src = "pure f(k: float, t0: float) -> float {\n var s = 0.0\n var t = t0\n while s < exp(t) {\n  t = k\n  s += 1.0\n }\n s\n}\nmain() {}\n";
    let mut m = lowered(src);
    let f = func_mut(&mut m, "f");
    let t = local(f, "t");
    assert_eq!(f.locals[t.0 as usize].kind, LocalKind::Value);
    let Stmt::While { body, .. } = loop_mut(f) else { panic!("not a while") };
    let Stmt::Assign(p, e) = body.remove(0) else { panic!("{body:#?}") };
    assert_eq!(p.root, t);
    body.insert(0, Stmt::Let(t, e));
    hoist::hoist_math(&mut m);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Exp), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn an_assumptions_condition_is_not_rewritten() {
    // Built as KIR (`@assume` takes no calls): an assumption in the loop whose condition holds
    // `sin(k)`. Range analysis reads facts from the condition as written.
    let src = "pure f(k: float, a: int, n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  @assume(a < 2)\n  s += 1.0\n }\n s\n}\nmain() {}\n";
    let mut m = lowered(src);
    let f = func_mut(&mut m, "f");
    let k = f.params[0];
    let mut found = 0;
    let Stmt::ForRange { body, .. } = loop_mut(f) else { panic!("not a for") };
    kestrel_kir::visit_mut::VisitMut::visit_stmts(&mut SinInAssume(&mut found, k), body);
    assert_eq!(found, 1, "{body:#?}");
    hoist::hoist_math(&mut m);
    let f = func(&m, "f");
    let mut kept = false;
    walk(&f.body, &mut |n| {
        if let Node::Stmt(Stmt::Assume { conds, .. }) = n {
            kept |= conds.iter().any(|(_, c)| has_math(c, MathOp::Sin));
        }
    });
    assert!(kept, "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

/// Replaces the first operand of each `Assume` condition's comparison with `int(sin(k))`.
struct SinInAssume<'a>(&'a mut u32, LocalIdx);

impl kestrel_kir::visit_mut::VisitMut for SinInAssume<'_> {
    fn visit_stmt(&mut self, s: &mut Stmt) {
        if let Stmt::Assume { conds, .. } = s {
            for (_, c) in conds.iter_mut() {
                let ExprKind::Binary(_, a, _) = &mut c.kind else { panic!("{c:#?}") };
                let sin = typed(ExprKind::Math(MathOp::Sin, vec![float_load(self.1)]), KTy::F32);
                **a = typed(ExprKind::Cast(Box::new(sin)), a.ty);
                *self.0 += 1;
            }
        }
        kestrel_kir::visit_mut::walk_stmt(self, s);
    }
}

#[test]
fn a_pointer_parameter_keeps_math_of_what_it_points_at() {
    // Without inlining, `x` is a `var` parameter: a pointer, which another name (`y`, if the
    // caller passed the same variable twice) may write. Only `Value` locals are read.
    let src = "pure f(var x: float, var y: float, n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  s += sin(x)\n  y += 1.0\n }\n s\n}\nmain() {}\n";
    let mut m = lowered(src);
    hoist::hoist_math(&mut m);
    let f = func(&m, "f");
    assert_eq!(f.locals[local(f, "x").0 as usize].kind, LocalKind::RefParam);
    assert!(math_in_loops(&f.body, MathOp::Sin), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn a_var_argument_keeps_math_of_the_local_it_writes() {
    // Built as KIR: `x` made a `Value` local although `bump(var x, 2)` (never inlined: it calls
    // itself) writes it, so only the `var` argument's write keeps `sin(x)` in the loop.
    let src = "bump(var x: float, n: int) {\n if n > 0 {\n  x += 0.5\n  bump(var x, n - 1)\n }\n}\nf(x0: float, n: int) -> float {\n var s = 0.0\n var x = x0\n for i from 0 to n {\n  s += sin(x)\n  bump(var x, 2)\n }\n s\n}\nmain() {}\n";
    let mut m = lowered(src);
    let f = func_mut(&mut m, "f");
    let x = local(f, "x");
    assert_eq!(f.locals[x.0 as usize].kind, LocalKind::Addressed);
    f.locals[x.0 as usize].kind = LocalKind::Value;
    hoist::hoist_math(&mut m);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Sin), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn a_temporary_assigned_again_before_its_use_is_not_seen_through() {
    // `exp(t)` reads `k + 1.0`, not `k`: seeing through `var t = k` would hoist `exp(k)`.
    let src = "pure f(k: float, n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  var t = k\n  t += 1.0\n  s += exp(t)\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Exp), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn a_temporary_bound_in_an_expression_ifs_condition_is_seen_through() {
    // Built as KIR: `s = (if { let t = k; exp(t) > 0.0 } { 1.0 } else { 2.0 }) + s`. The condition
    // runs on every pass and `t` is used only inside its `Seq`, so `exp(t)` is `exp(k)`.
    let src = "pure f(k: float, n: int) -> float {\n var s = 0.0\n var t = k\n for i from 0 to n {\n  s += exp(t)\n }\n s\n}\nmain() {}\n";
    let mut m = lowered(src);
    let f = func_mut(&mut m, "f");
    let (s, t, k) = (local(f, "s"), local(f, "t"), f.params[0]);
    let exp = typed(ExprKind::Math(MathOp::Exp, vec![float_load(t)]), KTy::F32);
    let gt = typed(ExprKind::Binary(BinOp::Gt, Box::new(exp), Box::new(typed(ExprKind::Float(0.0), KTy::F32))), KTy::Bool);
    let cond = typed(ExprKind::Seq(vec![Stmt::Let(t, float_load(k))], Box::new(gt)), KTy::Bool);
    let one = |x: f64| Some(Box::new(typed(ExprKind::Float(x), KTy::F32)));
    let pick = ExprKind::If { cond: Box::new(cond), then_body: Vec::new(), then_val: one(1.0), else_body: Vec::new(), else_val: one(2.0) };
    let sum = typed(ExprKind::Binary(BinOp::Add, Box::new(typed(pick, KTy::F32)), Box::new(float_load(s))), KTy::F32);
    let Stmt::ForRange { body, .. } = loop_mut(f) else { panic!("not a for") };
    *body = vec![Stmt::Assign(Place::local(s), sum)];
    hoist::hoist_math(&mut m);
    let f = func(&m, "f");
    let lets = hoisted_before(f, &f.body);
    assert_eq!(lets.len(), 1, "{:#?}", f.body);
    assert_eq!(loaded(&math_args(lets[0], MathOp::Exp)[0]), k, "{:#?}", lets[0]);
    assert!(!math_in_loops(&f.body, MathOp::Exp), "{:#?}", f.body);
}
