//! `hoist::hoist_math`: pure math whose operands a loop never changes moves out of the loop, and
//! nothing else does.

mod hoist_common;

use hoist_common::*;
use kestrel_kir::visit::{walk, walk_expr, Node};
use kestrel_kir::*;

const STEP: &str = "let KEEP = 0.5\nstruct Particle {\n pos: vec2\n vel: vec2\n pure step(var self, dt: float, target: vec2) {\n  self.vel += (target - self.pos).normalized() * 900.0 * dt\n  self.vel *= pow(KEEP, dt)\n  self.pos += self.vel * dt\n }\n}\nrun(var ps: list[Particle], dt: float, target: vec2) {\n par for p in ps {\n  p.step(dt, target)\n }\n}\nwalk(var ps: list[Particle], dt: float, target: vec2) {\n for p in ps {\n  p.step(dt, target)\n }\n}\nmain() {}\n";

/// Checks that `f`'s first loop has `pow(KEEP, dt)` bound right before it, reading `dt` itself
/// (not the inliner's temporary), and no `pow` left in any loop.
fn pow_leaves_the_loop(f: &Func) {
    let lets = hoisted_before(f, &f.body);
    assert_eq!(lets.len(), 1, "{:#?}", f.body);
    let args = math_args(lets[0], MathOp::Pow);
    assert!(matches!(args[0].kind, ExprKind::Float(k) if k == 0.5), "{:#?}", lets[0]);
    let dt = loaded(&args[1]);
    assert_eq!((name(f, dt), f.locals[dt.0 as usize].kind), ("dt", LocalKind::Value), "{:#?}", lets[0]);
    assert!(!math_in_loops(&f.body, MathOp::Pow), "{:#?}", f.body);
}

#[test]
fn pow_of_a_constant_and_a_capture_leaves_the_particle_loop() {
    let m = optimized(STEP);
    pow_leaves_the_loop(par_body(&m));
    pow_leaves_the_loop(func(&m, "walk"));
}

#[test]
fn pow_read_through_an_inner_loops_temporary_leaves_the_outer_loop_too() {
    // The inliner binds `dt` to a temporary inside `for p in ps`; the `pow` hoisted out of that
    // loop is invariant in the frame loop too, so it moves on before it, once.
    let src = STEP.replace("main() {}", "frames(var ps: list[Particle], dt: float, target: vec2) {
 for k from 0 to 10 {
  for p in ps {
   p.step(dt, target)
  }
 }
}
main() {}");
    let m = optimized(&src);
    pow_leaves_the_loop(func(&m, "frames"));
}

#[test]
fn a_par_for_beside_the_loop_leaves_its_float_captures_alone() {
    // `hoist_math.kes` case 1: the `par for` gets a copy of `dt`, so `dt` keeps its register and
    // the sequential loop's `pow` leaves both loops.
    let src = STEP.replace("main() {}", "both(var ps: list[Particle], target: vec2) {
 let dt = 1.0 / 60.0
 for k from 0 to 10 {
  for p in ps {
   p.step(dt, target)
  }
  par for p in ps {
   p.step(dt, target)
  }
 }
}
main() {}");
    let m = optimized(&src);
    pow_leaves_the_loop(func(&m, "both"));
    // Each `par for` body's parameter keeps the captured local's name.
    for fi in &m.par_bodies {
        let body = &m.funcs[fi.0 as usize];
        assert!(body.params.iter().any(|&p| name(body, p) == "dt"), "{:#?}", body.locals);
    }
}

const SCALE: &str ="pure scale(v: float, k: float) -> float { v * pow(k, 2.0) }\n";

#[test]
fn an_inliner_temporary_is_seen_through() {
    let src = format!("{SCALE}pure f(k: float, n: int) -> float {{\n var s = 0.0\n for i from 0 to n {{\n  s += scale(float(i), k)\n }}\n s\n}}\nmain() {{}}\n");
    let m = optimized(&src);
    let f = func(&m, "f");
    let lets = hoisted_before(f, &f.body);
    assert_eq!(lets.len(), 1, "{:#?}", f.body);
    let args = math_args(lets[0], MathOp::Pow);
    assert_eq!(loaded(&args[0]), f.params[0], "{:#?}", lets[0]);
    assert!(!math_in_loops(&f.body, MathOp::Pow), "{:#?}", f.body);
}

#[test]
fn hoist_math_case_1_computes_pow_once_before_its_loops() {
    let m = optimized(include_str!("../../tests/programs/hoist_math.kes"));
    let f = func(&m, "particles");
    // The sequential loop's `pow`, once, before the frame loop (the `par for` body has its own).
    assert!(!math_in_loops(&f.body, MathOp::Pow), "{:#?}", f.body);
    let pows = f.body.iter().filter(|s| stmt_has_math(s, MathOp::Pow)).count();
    assert_eq!(pows, 1, "{:#?}", f.body);
}

#[test]
fn a_temporary_whose_initializer_left_the_loop_is_read_through_its_hoisted_local() {
    // The inliner binds `let t = sin(k)` in the loop, and `sin(k)` is hoisted; `exp(t)` then reads
    // that hoisted local instead of computing `sin(k)` a second time.
    let src = "pure g(t: float) -> float { exp(t) }\npure f(k: float, n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  s += g(sin(k))\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    let lets = hoisted_before(f, &f.body);
    let sins = lets.iter().filter(|e| has_math(e, MathOp::Sin)).count();
    assert_eq!((lets.len(), sins), (2, 1), "{:#?}", f.body);
    let args = math_args(lets[1], MathOp::Exp);
    assert!(is_hoisted(f, loaded(&args[0])), "{:#?}", lets[1]);
    assert!(!math_in_loops(&f.body, MathOp::Sin) && !math_in_loops(&f.body, MathOp::Exp), "{:#?}", f.body);
}

#[test]
fn a_temporary_bound_inside_an_if_is_not_seen_through() {
    let src = format!("{SCALE}pure f(k: float, n: int) -> float {{\n var s = 0.0\n for i from 0 to n {{\n  if i > 3 {{\n   s += scale(float(i), k)\n  }}\n }}\n s\n}}\nmain() {{}}\n");
    let m = optimized(&src);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Pow), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

/// `f` from source whose loop body is `s += exp(t)` then `t = k`, with the assignment turned into
/// a `Let` and the two statements in the order `swap` says (the `Let` first when true).
fn temporary_after_its_use(swap: bool) -> Module {
    let src = "pure f(k: float, n: int) -> float {\n var s = 0.0\n var t = k\n for i from 0 to n {\n  s += exp(t)\n  t = k\n }\n s\n}\nmain() {}\n";
    let mut m = lowered(src);
    let fi = m.funcs.iter().position(|f| f.name == "f").unwrap();
    let f = &mut m.funcs[fi];
    let t = LocalIdx(f.locals.iter().position(|l| l.name == "t").unwrap() as u32);
    assert_eq!(f.locals[t.0 as usize].kind, LocalKind::Value);
    let Some(Stmt::ForRange { body, .. }) = f.body.iter_mut().find(|s| is_loop(s)) else { panic!("{:#?}", f.body) };
    assert_eq!(body.len(), 2, "{body:#?}");
    let Stmt::Assign(p, e) = body.pop().unwrap() else { panic!("{body:#?}") };
    assert_eq!(p.root, t);
    body.push(Stmt::Let(t, e));
    if swap {
        body.swap(0, 1);
    }
    hoist::hoist_math(&mut m);
    m
}

#[test]
fn a_temporary_used_before_its_let_is_not_seen_through() {
    let m = temporary_after_its_use(false);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Exp), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
    // The same `Let` before the use is seen through.
    let m = temporary_after_its_use(true);
    let f = func(&m, "f");
    assert!(!math_in_loops(&f.body, MathOp::Exp), "{:#?}", f.body);
    assert_eq!(hoisted_before(f, &f.body).len(), 1, "{:#?}", f.body);
}

#[test]
fn a_temporary_used_outside_its_seq_is_not_seen_through() {
    // Built as KIR: `s = { let t = k; s } + exp(t)`. Codegen may evaluate `exp(t)` before the `Seq`.
    let src = "pure f(k: float, n: int) -> float {
 var s = 0.0
 var t = k
 for i from 0 to n {
  s += exp(t)
 }
 s
}
main() {}
";
    let mut m = lowered(src);
    let fi = m.funcs.iter().position(|f| f.name == "f").unwrap();
    let f = &mut m.funcs[fi];
    let at = |name: &str| LocalIdx(f.locals.iter().position(|l| l.name == name).unwrap() as u32);
    let (s, t, k) = (at("s"), at("t"), f.params[0]);
    let load = |l: LocalIdx| Expr { kind: ExprKind::Load(Place::local(l)), ty: KTy::F32, line: 0 };
    let float = |kind: ExprKind| Expr { kind, ty: KTy::F32, line: 0 };
    let seq = float(ExprKind::Seq(vec![Stmt::Let(t, load(k))], Box::new(load(s))));
    let exp = float(ExprKind::Math(MathOp::Exp, vec![load(t)]));
    let Some(Stmt::ForRange { body, .. }) = f.body.iter_mut().find(|s| is_loop(s)) else { panic!("{:#?}", f.body) };
    *body = vec![Stmt::Assign(Place::local(s), float(ExprKind::Binary(BinOp::Add, Box::new(seq), Box::new(exp))))];
    hoist::hoist_math(&mut m);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Exp), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn a_temporary_assigned_again_is_not_seen_through() {
    let src = "pure f(k: float, n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  var t = k\n  s += exp(t)\n  t += 1.0\n  s += t\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Exp), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn a_temporary_of_a_temporary_is_not_seen_through() {
    let src = "pure f(k: float, n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  let a = k\n  let b = a\n  s += exp(b)\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Exp), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn math_of_a_variable_the_loop_changes_stays() {
    let src = "pure f(x0: float, n: int) -> float {\n var s = 0.0\n var x = x0\n for i from 0 to n {\n  s += sin(x)\n  x += 1.0\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Sin), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn math_of_the_loop_variable_stays() {
    let src = "pure f(n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  s += sin(float(i))\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Sin), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn math_changed_only_by_an_outer_loop_leaves_the_inner_loop_only() {
    let src = "pure f(n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  let a = float(i)\n  for j from 0 to n {\n   s += exp(a)\n  }\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    let outer = first_loop(&f.body);
    assert!(hoisted_before(f, &f.body).is_empty(), "{:#?}", f.body);
    let lets = hoisted_before(f, body_of(outer));
    assert_eq!(lets.len(), 1, "{:#?}", f.body);
    math_args(lets[0], MathOp::Exp);
    assert!(!math_in_loops(body_of(outer), MathOp::Exp), "{:#?}", f.body);
}

#[test]
fn math_in_a_branch_of_the_loop_is_hoisted() {
    let src = "pure f(x: float, n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  if i > 5 {\n   s += log(x)\n  }\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    let lets = hoisted_before(f, &f.body);
    assert_eq!(lets.len(), 1, "{:#?}", f.body);
    math_args(lets[0], MathOp::Log);
    assert!(!math_in_loops(&f.body, MathOp::Log), "{:#?}", f.body);
}

#[test]
fn an_operand_with_integer_arithmetic_stays() {
    let src = "pure f(n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  s += pow(2.0, float(n * 2))\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Pow), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn float_remainder_by_an_invariant_zero_is_hoisted() {
    let src = "pure f(x: float, n: int) -> float {\n let z = 0.0\n var s = 0.0\n for i from 0 to n {\n  s += x % z\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    let lets = hoisted_before(f, &f.body);
    assert_eq!(lets.len(), 1, "{:#?}", f.body);
    assert!(matches!(lets[0].kind, ExprKind::Binary(BinOp::Rem, ..)), "{:#?}", lets[0]);
    let mut rem_in_loop = false;
    walk(body_of(first_loop(&f.body)), &mut |n| rem_in_loop |= matches!(n, Node::Expr(Expr { kind: ExprKind::Binary(BinOp::Rem, ..), .. })));
    assert!(!rem_in_loop, "{:#?}", f.body);
}

#[test]
fn cheap_math_is_left_to_cranelift() {
    let src = "pure f(x: float, n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  s += sqrt(x)\n }\n s\n}\npure g(x: float, n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  s += sqrt(x) * sin(x)\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
    let g = func(&m, "g");
    let lets = hoisted_before(g, &g.body);
    assert_eq!((lets.len(), hoisted_locals(g)), (1, 1), "{:#?}", g.body);
    let ExprKind::Binary(BinOp::Mul, a, b) = &lets[0].kind else { panic!("{:#?}", lets[0]) };
    math_args(a, MathOp::Sqrt);
    math_args(b, MathOp::Sin);
    assert!(!math_in_loops(&g.body, MathOp::Sin) && !math_in_loops(&g.body, MathOp::Sqrt), "{:#?}", g.body);
}

#[test]
fn nested_invariant_math_hoists_as_one_expression() {
    let src = "pure f(a: float, b: float, n: int) -> float {\n var s = 0.0\n for i from 0 to n {\n  s += pow(sin(a), b)\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    let lets = hoisted_before(f, &f.body);
    assert_eq!((lets.len(), hoisted_locals(f)), (1, 1), "{:#?}", f.body);
    math_args(&math_args(lets[0], MathOp::Pow)[0], MathOp::Sin);
}

#[test]
fn math_in_a_while_condition_is_hoisted() {
    let src = "pure f(x: float) -> float {\n var s = 0.0\n while s < exp(x) {\n  s += 1.0\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    let lets = hoisted_before(f, &f.body);
    assert_eq!(lets.len(), 1, "{:#?}", f.body);
    math_args(lets[0], MathOp::Exp);
    let Stmt::While { cond, .. } = first_loop(&f.body) else { panic!("{:#?}", f.body) };
    let mut reads = false;
    walk_expr(cond, &mut |n| reads |= matches!(n, Node::Place(p, false) if is_hoisted(f, p.root)));
    assert!(reads && !math_in_loops(&f.body, MathOp::Exp), "{:#?}", f.body);
}

#[test]
fn f64_math_is_hoisted_too() {
    let src = "pure f(b: f64, e: f64, n: int) -> f64 {\n var s: f64 = 0\n for i from 0 to n {\n  s += pow(b, e)\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    let lets = hoisted_before(f, &f.body);
    assert_eq!(lets.len(), 1, "{:#?}", f.body);
    math_args(lets[0], MathOp::Pow);
    assert_eq!(lets[0].ty, KTy::F64);
    assert!(!math_in_loops(&f.body, MathOp::Pow), "{:#?}", f.body);
}

#[test]
fn a_var_argument_in_the_loop_keeps_math_of_that_variable() {
    // `bump` calls itself, so it is never inlined and `x` stays `Addressed`: the rule that only
    // `Value` locals are read keeps `sin(x)` here, before the `var` argument's write is looked at.
    // (`hoist_rules.rs` tests each of the two rules on its own.)
    let src = "bump(var x: float, n: int) {\n if n > 0 {\n  x += 0.5\n  bump(var x, n - 1)\n }\n}\nf(x0: float, n: int) -> float {\n var s = 0.0\n var x = x0\n for i from 0 to n {\n  s += sin(x)\n  bump(var x, 2)\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Sin), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}

#[test]
fn an_operand_a_for_loops_end_changes_stays() {
    // The hoisted `Let` runs before the loop's bounds, so a bound that writes `x` (an inlined call
    // with a `var` argument) keeps `sin(x)` in the loop.
    let src = "pure next(var x: float) -> int {\n x += 1.0\n 3\n}\npure f(x0: float) -> float {\n var x = x0\n var s = 0.0\n for i from 0 to next(var x) {\n  s += sin(x)\n }\n s\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    assert!(math_in_loops(&f.body, MathOp::Sin), "{:#?}", f.body);
    assert_eq!(hoisted_locals(f), 0, "{:#?}", f.body);
}
