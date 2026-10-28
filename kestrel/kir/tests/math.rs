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
fn random_numbers_lower_to_their_own_nodes() {
    let m = lower_src("main() {\n rand_seed(7)\n let a = randi_range(1, 6)\n let x = randf()\n let w: f64 = 1\n let r = randf_range(2, w)\n randf()\n}\n");
    let body = main_body(&m);
    assert!(matches!(&body[0], Stmt::RandSeed(Expr { kind: ExprKind::Int(7), ty: KTy::I32, .. })), "{:?}", body[0]);
    match &body[1] {
        Stmt::Let(_, Expr { kind: ExprKind::RandInt(lo, hi), ty: KTy::I32, .. }) => {
            assert!(matches!((&lo.kind, &hi.kind), (ExprKind::Int(1), ExprKind::Int(6))));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&body[2], Stmt::Let(_, Expr { kind: ExprKind::RandFloat { ranged: false }, ty: KTy::F32, .. })), "{:?}", body[2]);
    // `lo + (hi - lo) * draw`, at the result type, with `hi` read from a temporary.
    match &body[4] {
        Stmt::Let(_, Expr { kind: ExprKind::Seq(pre, value), ty: KTy::F64, .. }) => {
            assert_eq!(pre.len(), 1, "{pre:?}");
            let ExprKind::Binary(BinOp::Add, lo, offset) = &value.kind else { panic!("{value:?}") };
            assert!(matches!(lo.kind, ExprKind::Float(f) if f == 2.0), "{lo:?}");
            let ExprKind::Binary(BinOp::Mul, width, draw) = &offset.kind else { panic!("{offset:?}") };
            assert!(matches!(width.kind, ExprKind::Binary(BinOp::Sub, ..)), "{width:?}");
            assert!(matches!(draw.kind, ExprKind::RandFloat { ranged: true }) && draw.ty == KTy::F64, "{draw:?}");
        }
        other => panic!("{other:?}"),
    }
    // A discarded draw is still taken.
    assert!(matches!(&body[5], Stmt::Let(_, Expr { kind: ExprKind::RandFloat { ranged: false }, .. })), "{:?}", body[5]);
}

#[test]
fn randf_range_evaluates_both_ends_once_and_in_order() {
    let m = lower_src("noisy(n: int) -> float {\n float(n)\n}\nmain() {\n let r = randf_range(noisy(1), noisy(2))\n}\n");
    let Stmt::Let(_, Expr { kind: ExprKind::Seq(pre, value), .. }) = &main_body(&m)[0] else { panic!("{:?}", main_body(&m)) };
    assert_eq!(pre.len(), 2, "{pre:?}");
    for (stmt, n) in pre.iter().zip([1, 2]) {
        match stmt {
            Stmt::Let(_, Expr { kind: ExprKind::Call(_, args), .. }) => {
                assert!(matches!(&args[0], Arg::Value(Expr { kind: ExprKind::Int(i), .. }) if *i == n), "{args:?}");
            }
            other => panic!("{other:?}"),
        }
    }
    // The arithmetic only reads the temporaries.
    let mut calls = 0;
    kestrel_kir::visit::walk(&[Stmt::Return(Some((**value).clone()))], &mut |n| {
        calls += usize::from(matches!(n, kestrel_kir::visit::Node::Expr(Expr { kind: ExprKind::Call(..), .. })));
    });
    assert_eq!(calls, 0);
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
