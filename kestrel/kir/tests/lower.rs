use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(src))
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

#[test]
fn tail_expression_becomes_return() {
    let m = lower_src("pure sq(x: int) -> int { x * x }\nmain() {}\n");
    let sq = func(&m, "sq");
    assert_eq!(sq.ret, Some(KTy::I32));
    assert_eq!(sq.params, vec![LocalIdx(0)]);
    assert_eq!(sq.locals[0].kind, LocalKind::Value);
    assert!(matches!(
        &sq.body[..],
        [Stmt::Return(Some(Expr { kind: ExprKind::Binary(BinOp::Mul, _, _), ty: KTy::I32, .. }))]
    ));
    assert_eq!(m.main, FuncIdx(1));
}

#[test]
fn var_params_and_addressed_locals() {
    let m = lower_src("bump(var n: int) {\n n += 1\n}\nmain() {\n var t = 0\n bump(var t)\n}\n");
    let bump = func(&m, "bump");
    assert_eq!(bump.locals[0].kind, LocalKind::RefParam);
    match &bump.body[0] {
        Stmt::Assign(LocalIdx(0), Expr { kind: ExprKind::Binary(BinOp::Add, l, r), .. }) => {
            assert!(matches!(l.kind, ExprKind::Local(LocalIdx(0))));
            assert!(matches!(r.kind, ExprKind::Int(1)));
        }
        other => panic!("{other:?}"),
    }
    let main = func(&m, "main");
    assert_eq!(main.locals[0].kind, LocalKind::Addressed);
    assert!(matches!(&main.body[1], Stmt::Call(FuncIdx(0), args) if args == &vec![Arg::Ref(LocalIdx(0))]));
}

#[test]
fn coercions_become_casts_and_literals_adapt() {
    let m = lower_src("main() {\n let a = 1\n let b: f64 = a\n let c: float = 3\n}\n");
    let main = func(&m, "main");
    match &main.body[1] {
        Stmt::Let(LocalIdx(1), Expr { kind: ExprKind::Cast(inner), ty: KTy::F64, .. }) => {
            assert!(matches!(inner.kind, ExprKind::Local(LocalIdx(0))));
            assert_eq!(inner.ty, KTy::I32);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&main.body[2], Stmt::Let(_, Expr { kind: ExprKind::Float(f), ty: KTy::F32, .. }) if *f == 3.0));
}

#[test]
fn constants_are_inlined() {
    let m = lower_src("let N = 4\nmain() {\n let x = N * 2\n}\n");
    match &func(&m, "main").body[0] {
        Stmt::Let(_, Expr { kind: ExprKind::Binary(BinOp::Mul, l, r), .. }) => {
            assert!(matches!(l.kind, ExprKind::Int(4)));
            assert!(matches!(r.kind, ExprKind::Int(2)));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn if_expression_and_print() {
    let m = lower_src("main() {\n let x = if true { 1 } else { 2 }\n print(\"x is\", x)\n}\n");
    let main = func(&m, "main");
    assert!(matches!(&main.body[0], Stmt::Let(_, Expr { kind: ExprKind::If { then_val: Some(_), else_val: Some(_), .. }, .. })));
    match &main.body[1] {
        Stmt::Print(args) => {
            assert_eq!(args[0], PrintArg::Str("x is".into()));
            assert!(matches!(&args[1], PrintArg::Value(Expr { kind: ExprKind::Local(LocalIdx(0)), .. })));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn labelled_arguments_are_in_parameter_order() {
    let m = lower_src("f(a: int, b: int) -> int { a - b }\nmain() {\n let x = f(b: 1, a: 2)\n}\n");
    match &func(&m, "main").body[0] {
        Stmt::Let(_, Expr { kind: ExprKind::Call(FuncIdx(0), args), .. }) => {
            assert!(matches!(&args[0], Arg::Value(Expr { kind: ExprKind::Int(2), .. })));
            assert!(matches!(&args[1], Arg::Value(Expr { kind: ExprKind::Int(1), .. })));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn nested_diverging_tail_is_a_statement() {
    let src = "f(x: int) -> int {\n if x > 0 { if x > 5 { return 1 } else { return 2 } } else { return 3 }\n}\nmain() {}\n";
    let m = lower_src(src);
    assert!(matches!(&func(&m, "f").body[..], [Stmt::If { .. }]));
}

#[test]
fn diverging_tail_adds_no_return() {
    let m = lower_src("f(x: int) -> int {\n if x > 0 { return 1 } else { return 2 }\n}\nmain() {}\n");
    assert!(matches!(&func(&m, "f").body[..], [Stmt::If { .. }]));
}

#[test]
fn widened_float_literals_are_f64_literals_not_casts() {
    let m = lower_src("main() {\n let x: f64 = 0.1\n let a = 0.1 * x\n let b = -0.1 * x\n let c = 3 * x\n}\n");
    let main = func(&m, "main");
    let lhs = |i: usize| match &main.body[i] {
        Stmt::Let(_, Expr { kind: ExprKind::Binary(BinOp::Mul, l, _), .. }) => (**l).clone(),
        other => panic!("{other:?}"),
    };
    assert!(matches!(lhs(1).kind, ExprKind::Float(v) if v == 0.1) && lhs(1).ty == KTy::F64);
    assert!(matches!(lhs(2).kind, ExprKind::Float(v) if v == -0.1) && lhs(2).ty == KTy::F64);
    assert!(matches!(lhs(3).kind, ExprKind::Float(v) if v == 3.0) && lhs(3).ty == KTy::F64);
}
