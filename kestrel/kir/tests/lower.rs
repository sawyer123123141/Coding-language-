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
        Stmt::Assign(Place { root: LocalIdx(0), .. }, Expr { kind: ExprKind::Binary(BinOp::Add, l, r), .. }) => {
            assert!(matches!(l.kind, ExprKind::Load(Place { root: LocalIdx(0), .. })));
            assert!(matches!(r.kind, ExprKind::Int(1)));
        }
        other => panic!("{other:?}"),
    }
    let main = func(&m, "main");
    assert_eq!(main.locals[0].kind, LocalKind::Addressed);
    assert!(matches!(&main.body[1], Stmt::Call(FuncIdx(0), args) if args == &vec![Arg::Ref(Place::local(LocalIdx(0)))]));
}

#[test]
fn coercions_become_casts_and_literals_adapt() {
    let m = lower_src("main() {\n let a = 1\n let b: f64 = a\n let c: float = 3\n}\n");
    let main = func(&m, "main");
    match &main.body[1] {
        Stmt::Let(LocalIdx(1), Expr { kind: ExprKind::Cast(inner), ty: KTy::F64, .. }) => {
            assert!(matches!(inner.kind, ExprKind::Load(Place { root: LocalIdx(0), .. })));
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
            assert!(matches!(&args[1], PrintArg::Value(Expr { kind: ExprKind::Load(Place { root: LocalIdx(0), .. }), .. })));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn labelled_arguments_are_in_parameter_order() {
    let m = lower_src("f(a: int, b: int) -> int { a - b }\nmain() {\n let x = f(b: 1, a: 2)\n}\n");
    // `b: 1` is evaluated first into one temporary, `a: 2` into the next;
    // the call then passes them in parameter order (a, b).
    match &func(&m, "main").body[0] {
        Stmt::Let(_, Expr { kind: ExprKind::Seq(pre, call), .. }) => {
            let [Stmt::Let(b_tmp, _), Stmt::Let(a_tmp, _)] = &pre[..] else { panic!("{pre:?}") };
            let ExprKind::Call(FuncIdx(0), args) = &call.kind else { panic!("{call:?}") };
            assert!(matches!(&args[0], Arg::Value(Expr { kind: ExprKind::Load(Place { root, .. }), .. }) if root == a_tmp));
            assert!(matches!(&args[1], Arg::Value(Expr { kind: ExprKind::Load(Place { root, .. }), .. }) if root == b_tmp));
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

const SHAPES: &str = "struct Point {\n    x: int\n    y: int = 7\n\n    pure sum(self) -> int { self.x + self.y }\n    shift(var self, by: int) {\n        self.x += by\n    }\n}\n\nstruct Line {\n    a: Point\n    b: Point\n}\n\nmake(n: int) -> Point {\n    Point(n)\n}\n\nmain() {\n    var line = Line(Point(1, 2), make(3))\n    line.b.y = 4\n    line.a.shift(by: 5)\n    let s = make(1).sum()\n    let r = make(2).x\n    print(line)\n}\n";

#[test]
fn struct_layouts_are_c_like() {
    let m = lower_src("struct S {\n    a: u8\n    b: i64\n    c: u8\n}\nstruct T {\n    s: S\n    d: int\n}\nmain() {}\n");
    // Structs 0 and 1 are the built-in vec2 and vec3.
    let s = &m.structs[2];
    assert_eq!(s.fields.iter().map(|f| f.offset).collect::<Vec<_>>(), vec![0, 8, 16]);
    assert_eq!((s.size, s.align), (24, 8));
    let t = &m.structs[3];
    assert_eq!(t.fields.iter().map(|f| f.offset).collect::<Vec<_>>(), vec![0, 24]);
    assert_eq!((t.size, t.align), (32, 8));
}

#[test]
fn struct_locals_live_in_memory_and_params_by_pointer() {
    let m = lower_src(SHAPES);
    let main = func(&m, "main");
    assert_eq!(main.locals[0].kind, LocalKind::Addressed);
    assert!(main.locals[0].ty.is_aggregate());
    let sum = func(&m, "Point.sum");
    assert_eq!(sum.locals[0].kind, LocalKind::RefParam);
    assert_eq!(func(&m, "make").ret, Some(KTy::Struct(StructIdx(2))));
}

#[test]
fn constructors_fill_defaults_and_fields_are_places() {
    let m = lower_src(SHAPES);
    let make = func(&m, "make");
    match &make.body[..] {
        [Stmt::Return(Some(Expr { kind: ExprKind::StructLit(StructIdx(2), fields), .. }))] => {
            assert!(matches!(fields[0].kind, ExprKind::Load(_)));
            assert!(matches!(fields[1].kind, ExprKind::Int(7)));
        }
        other => panic!("{other:?}"),
    }
    let main = func(&m, "main");
    assert!(matches!(&main.body[1], Stmt::Assign(Place { root: LocalIdx(0), path }, _) if path == &vec![Proj::Field(1), Proj::Field(1)]));
    match &main.body[2] {
        Stmt::Call(_, args) => assert_eq!(args[0], Arg::Ref(Place { root: LocalIdx(0), path: vec![Proj::Field(0)] })),
        other => panic!("{other:?}"),
    }
    match &main.body[4] {
        Stmt::Let(_, Expr { kind: ExprKind::FieldOf(base, 0), .. }) => assert!(matches!(base.kind, ExprKind::Call(..))),
        other => panic!("{other:?}"),
    }
}

#[test]
fn labelled_arguments_are_evaluated_in_source_order() {
    let m = lower_src("sub(a: int, b: int) -> int { a - b }\nstruct P {\n    x: int\n    y: int\n}\nmain() {\n    let d = sub(b: 1, a: 2)\n    let p = P(y: 3, x: 4)\n}\n");
    let main = func(&m, "main");
    match &main.body[0] {
        Stmt::Let(_, Expr { kind: ExprKind::Seq(pre, call), .. }) => {
            assert!(matches!(&pre[..], [Stmt::Let(_, Expr { kind: ExprKind::Int(1), .. }), Stmt::Let(_, Expr { kind: ExprKind::Int(2), .. })]));
            assert!(matches!(call.kind, ExprKind::Call(..)));
        }
        other => panic!("{other:?}"),
    }
    match &main.body[1] {
        Stmt::Let(_, Expr { kind: ExprKind::Seq(pre, lit), .. }) => {
            assert!(matches!(&pre[..], [Stmt::Let(_, Expr { kind: ExprKind::Int(3), .. }), Stmt::Let(_, Expr { kind: ExprKind::Int(4), .. })]));
            assert!(matches!(lit.kind, ExprKind::StructLit(..)));
        }
        other => panic!("{other:?}"),
    }
}

const ENEMIES: &str = "enum Enemy {\n    Zombie { hp: int, speed: float }\n    Skeleton { hp: int, arrows: i64 }\n    Ghost\n\n    take_damage(var self, dmg: int) {\n        match self {\n            Zombie z { z.hp -= dmg }\n            Skeleton s { s.hp -= dmg * 2 }\n            Ghost { }\n        }\n    }\n}\n\nmain() {\n    var e = Enemy.Skeleton(arrows: 4, hp: 10)\n    e.take_damage(3)\n    let n = match e {\n        Skeleton s { s.hp }\n        else { 0 }\n    }\n    let g = Enemy.Ghost\n}\n";

#[test]
fn enum_layout_is_a_tag_then_a_shared_payload() {
    let m = lower_src(ENEMIES);
    let e = &m.enums[0];
    // The payload is aligned for the i64 in Skeleton, so it starts at 8.
    assert_eq!((e.size, e.align), (24, 8));
    assert_eq!(e.variants[0].fields.iter().map(|f| f.offset).collect::<Vec<_>>(), vec![8, 12]);
    assert_eq!(e.variants[1].fields.iter().map(|f| f.offset).collect::<Vec<_>>(), vec![8, 16]);
    assert!(e.variants[2].fields.is_empty());
}

#[test]
fn match_becomes_an_if_chain_on_the_tag_and_bindings_are_places() {
    let m = lower_src(ENEMIES);
    let td = func(&m, "Enemy.take_damage");
    // The binding locals aren't storage: only `self` and `dmg` remain.
    assert_eq!(td.locals.len(), 2);
    match &td.body[..] {
        [Stmt::If { cond, then_body, else_body }] => {
            assert!(matches!(&cond.kind, ExprKind::Binary(BinOp::Eq, tag, v)
                if matches!(tag.kind, ExprKind::Tag(_)) && matches!(v.kind, ExprKind::Int(0))));
            assert!(matches!(&then_body[..], [Stmt::Assign(Place { root: LocalIdx(0), path }, _)] if path == &vec![Proj::Variant(0, 0)]));
            // The last arm (Ghost) runs without a test once the others fail.
            assert!(matches!(&else_body[..], [Stmt::If { else_body: ghost, .. }] if ghost.is_empty()));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn variants_are_built_in_declaration_order() {
    let m = lower_src(ENEMIES);
    let main = func(&m, "main");
    match &main.body[0] {
        Stmt::Let(_, Expr { kind: ExprKind::Seq(pre, lit), .. }) => {
            assert_eq!(pre.len(), 2, "labelled fields out of order are evaluated in source order first");
            assert!(matches!(lit.kind, ExprKind::VariantLit(EnumIdx(0), 1, _)));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&main.body[3], Stmt::Let(_, Expr { kind: ExprKind::VariantLit(EnumIdx(0), 2, fields), .. }) if fields.is_empty()));
}
