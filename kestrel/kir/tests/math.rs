use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(src))
}

fn main_body(m: &Module) -> &[Stmt] {
    &m.funcs.iter().find(|f| f.name == "main").unwrap().body
}

#[test]
fn float_functions_lower_to_math_nodes() {
    let m = lower_src("main() {\n let a = sqrt(16)\n let w: f64 = 2\n let b = pow(2, w)\n}\n");
    match &main_body(&m)[0] {
        Stmt::Let(_, Expr { kind: ExprKind::Math(MathOp::Sqrt, args), ty: KTy::F32, .. }) => {
            assert_eq!(args.len(), 1);
            assert!(matches!(args[0].kind, ExprKind::Float(f) if f == 16.0));
            assert_eq!(args[0].ty, KTy::F32);
        }
        other => panic!("{other:?}"),
    }
    match &main_body(&m)[2] {
        Stmt::Let(_, Expr { kind: ExprKind::Math(MathOp::Pow, args), ty: KTy::F64, .. }) => {
            assert_eq!(args.len(), 2);
            assert!(args.iter().all(|a| a.ty == KTy::F64), "both operands have the result's type: {args:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn pi_and_tau_are_float_literals_of_the_expression_type() {
    let m = lower_src("main() {\n let a = PI\n let b: f64 = TAU\n}\n");
    let body = main_body(&m);
    assert!(matches!(&body[0], Stmt::Let(_, Expr { kind: ExprKind::Float(f), ty: KTy::F32, .. }) if *f == std::f64::consts::PI));
    assert!(matches!(&body[1], Stmt::Let(_, Expr { kind: ExprKind::Float(f), ty: KTy::F64, .. }) if *f == std::f64::consts::TAU));
}

#[test]
fn pi_and_tau_widen_without_losing_precision() {
    // No hint reaches `PI`, so it types as float; meeting an f64 it must still be the f64 constant.
    let m = lower_src("main() {\n let w: f64 = 2\n let a = PI * w\n let b = -TAU * w\n}\n");
    let body = main_body(&m);
    match &body[1] {
        Stmt::Let(_, Expr { kind: ExprKind::Binary(BinOp::Mul, l, _), ty: KTy::F64, .. }) => {
            assert!(matches!(l.kind, ExprKind::Float(f) if f == std::f64::consts::PI), "{l:?}");
            assert_eq!(l.ty, KTy::F64);
        }
        other => panic!("{other:?}"),
    }
    match &body[2] {
        Stmt::Let(_, Expr { kind: ExprKind::Binary(BinOp::Mul, l, _), ty: KTy::F64, .. }) => {
            assert!(matches!(l.kind, ExprKind::Float(f) if f == -std::f64::consts::TAU), "{l:?}");
            assert_eq!(l.ty, KTy::F64);
        }
        other => panic!("{other:?}"),
    }
}
