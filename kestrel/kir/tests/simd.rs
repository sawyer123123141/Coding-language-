//! `vec4` in KIR: one `F32X4` value with its lane type's answers, laid out 16/16 everywhere,
//! operated on whole, with lanes as the last step of a place, and never split into lanes.

use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lowered(src: &str, optimize_it: bool) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = lower(&p, &t.unwrap(), &LineIndex::new(src));
    if optimize_it {
        optimize(&mut m);
    }
    m
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

/// Every expression in `f`'s body.
fn exprs(f: &Func) -> Vec<Expr> {
    let mut out = Vec::new();
    visit::walk(&f.body, &mut |n| {
        if let visit::Node::Expr(e) = n {
            out.push(e.clone());
        }
    });
    out
}

/// The value `f` returns (its body's last `return`).
fn returned(f: &Func) -> &Expr {
    match f.body.last() {
        Some(Stmt::Return(Some(e))) => e,
        other => panic!("no return value: {other:?}"),
    }
}

#[test]
fn vector_types_answer_as_lanes() {
    for (v, lane) in [(KTy::F32X4, KTy::F32), (KTy::I32X4, KTy::I32)] {
        assert!(v.is_vector());
        assert!(!v.is_int() && !v.is_float() && !v.is_signed() && !v.is_aggregate(), "{v:?}");
        assert_eq!(v.lane(), lane);
        assert_eq!(lane.lane(), lane);
        assert!(!lane.is_vector());
    }
}

#[test]
fn vector_layouts() {
    let src = "struct P {\n id: u8\n pos: vec4\n}\nmain() {\n let n: u8 = 3\n let v = vec4(1)\n let f = () -> float { float(n) + v.x }\n let xs = list[vec4]()\n let o: vec4? = none\n let m = map[int, vec4]()\n print(P(id: 1, pos: v), f(), xs, o, m)\n}\n";
    let m = lowered(src, false);
    assert_eq!(m.size_align(KTy::F32X4), (16, 16));
    assert_eq!(m.size_align(KTy::I32X4), (16, 16));
    let p = m.structs.iter().position(|s| s.name == "P").unwrap();
    let layout = m.layout(StructIdx(p as u32));
    assert_eq!((layout.fields[1].offset, layout.size, layout.align), (16, 32, 16));
    let elem = |want: fn(KTy) -> bool| m.elems.iter().position(|&e| want(e)).map(|i| ElemIdx(i as u32));
    let v4 = elem(|e| e == KTy::F32X4).expect("vec4 is an element type");
    assert_eq!(m.size_align(m.elem(v4)).0, 16);
    assert_eq!(m.payload_offset(v4), 16);
    let int = elem(|e| e == KTy::I32).expect("int is a key type");
    let l = m.map_layout(int, v4);
    assert_eq!((l.val_off, l.entry_align), (16, 16));
    let mut offsets = Vec::new();
    visit::walk(&func(&m, "main").body, &mut |n| {
        if let visit::Node::Expr(Expr { kind: ExprKind::MakeFn { env: EnvKind::Heap(env), .. }, .. }) = n {
            offsets.extend(env.layout.offsets.iter().copied().zip(env.values.iter().map(|v| v.ty)));
        }
    });
    let at = offsets.iter().find(|(_, ty)| *ty == KTy::F32X4).expect("the closure captures the vec4").0;
    assert_eq!(at % 16, 0, "{offsets:?}");
}

#[test]
fn vector_math_is_one_value() {
    let m = lowered("pure f(a: vec4, b: vec4) -> vec4 {\n a * b + a\n}\nmain() {}\n", true);
    let f = func(&m, "f");
    let ExprKind::Binary(BinOp::Add, mul, a) = &returned(f).kind else { panic!("{:?}", returned(f)) };
    assert!(matches!(mul.kind, ExprKind::Binary(BinOp::Mul, ..)) && mul.ty == KTy::F32X4);
    assert_eq!(a.ty, KTy::F32X4);
    assert_eq!(returned(f).ty, KTy::F32X4);
    assert!(!exprs(f).iter().any(|e| matches!(e.kind, ExprKind::StructLit(..))));
    assert!(f.locals.iter().all(|l| l.ty == KTy::F32X4), "{:#?}", f.locals);
}

#[test]
fn a_number_operand_is_splat() {
    let src = "pure f(v: vec4) -> vec4 {\n v * 2.0\n}\npure g(v: vec4) -> vec4 {\n var w = v\n w *= 2.0\n w\n}\nmain() {}\n";
    let m = lowered(src, false);
    let ExprKind::Binary(BinOp::Mul, _, two) = &returned(func(&m, "f")).kind else { panic!() };
    assert!(matches!(&two.kind, ExprKind::Splat(x) if matches!(x.kind, ExprKind::Float(n) if n == 2.0)), "{two:?}");
    assert_eq!(two.ty, KTy::F32X4);
    let g = func(&m, "g");
    let assign = g.body.iter().find_map(|s| if let Stmt::Assign(p, e) = s { Some((p, e)) } else { None }).expect("`w *= 2.0`");
    let ExprKind::Binary(BinOp::Mul, cur, two) = &assign.1.kind else { panic!("{:?}", assign.1) };
    assert!(matches!(cur.kind, ExprKind::Load(_)) && cur.ty == KTy::F32X4);
    assert!(matches!(two.kind, ExprKind::Splat(_)) && two.ty == KTy::F32X4, "{two:?}");
}

#[test]
fn a_swizzle_is_a_shuffle() {
    let src = "pure f(v: vec4) -> vec4 {\n v.wzyx\n}\npure g(v: vec4) -> vec2 {\n v.xy\n}\nmain() {}\n";
    let m = lowered(src, false);
    assert!(matches!(returned(func(&m, "f")).kind, ExprKind::Shuffle(_, [3, 2, 1, 0])), "{:?}", returned(func(&m, "f")));
    let ExprKind::StructLit(_, lanes) = &returned(func(&m, "g")).kind else { panic!("{:?}", returned(func(&m, "g"))) };
    let picked: Vec<_> = lanes
        .iter()
        .map(|e| match &e.kind {
            ExprKind::Load(p) => p.path.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(picked, vec![vec![Proj::Lane(0)], vec![Proj::Lane(1)]]);
}

#[test]
fn a_lane_write_is_a_place() {
    let m = lowered("pure f() -> vec4 {\n var v = vec4()\n v.x = 1.0\n v\n}\nmain() {}\n", false);
    let f = func(&m, "f");
    let assigned = f.body.iter().find_map(|s| if let Stmt::Assign(p, _) = s { Some(p.clone()) } else { None }).unwrap();
    assert_eq!(assigned.path, vec![Proj::Lane(0)]);
    assert_eq!(f.locals[assigned.root.0 as usize].ty, KTy::F32X4);
}

#[test]
fn scalar_replacement_keeps_vectors_whole() {
    let src = "struct P {\n pos: vec4\n vel: vec4\n}\npure f(a: vec4) -> float {\n var p = P(pos: a, vel: a * 2)\n p.pos += p.vel\n p.pos.y + p.vel.w\n}\nmain() {}\n";
    let m = lowered(src, true);
    let f = func(&m, "f");
    let leaves: Vec<&Local> = f.locals.iter().filter(|l| l.name.starts_with("p.")).collect();
    assert_eq!(leaves.iter().map(|l| (l.name.as_str(), l.ty)).collect::<Vec<_>>(), vec![("p.pos", KTy::F32X4), ("p.vel", KTy::F32X4)]);
    assert!(!f.locals.iter().any(|l| l.ty == KTy::F32 && l.kind != LocalKind::Unused), "{:#?}", f.locals);
    let pos = LocalIdx(f.locals.iter().position(|l| l.name == "p.pos").unwrap() as u32);
    let reads_lane_1 = exprs(f).iter().any(|e| matches!(&e.kind, ExprKind::Load(p) if p.root == pos && p.path == [Proj::Lane(1)]));
    assert!(reads_lane_1, "{:#?}", f.body);
}

#[test]
fn a_vector_zero_is_a_splat() {
    let src = "struct B {\n p: vec4\n}\nid(x: int, d: int) -> int {\n if d == 0 {\n return x\n }\n id(x, d - 1)\n}\nmain() {\n let b = if id(1, 2) > 0 { B(p: vec4(1)) } else { B(p: vec4(2)) }\n print(b)\n}\n";
    let m = lowered(src, true);
    let main = func(&m, "main");
    let zeroed = main.body.iter().any(|s| {
        matches!(s, Stmt::Let(_, Expr { kind: ExprKind::Splat(x), ty: KTy::F32X4, .. }) if matches!(x.kind, ExprKind::Float(z) if z == 0.0))
    });
    assert!(zeroed, "{:#?}", main.body);
}

#[test]
fn vector_forms_read_as_kestrel() {
    let src = "pure g(v: vec4) -> vec4 {\n v\n}\npure a(v: vec4) -> vec4 {\n v.wzyx\n}\npure b() -> vec4 {\n vec4(2.5)\n}\npure c(x: float, y: float) -> vec4 {\n vec4(x, y, 1, 2)\n}\npure d(v: vec4) -> float {\n g(v).y\n}\npure e(v: vec4) -> float {\n v.z\n}\nmain() {}\n";
    let m = lowered(src, false);
    let text = |name: &str| range::show::expr_text(&m, func(&m, name), returned(func(&m, name)));
    assert_eq!(text("a"), "v.wzyx");
    assert_eq!(text("b"), "vec4(2.5)");
    assert_eq!(text("c"), "vec4(x, y, 1, 2)");
    assert_eq!(text("d"), "g(...).y");
    assert_eq!(text("e"), "v.z");
}
