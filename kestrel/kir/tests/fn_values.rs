//! Function values in KIR: thunks, `FnRef`, indirect calls, and what the passes do with them.

use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str, passes: &[&str]) -> Module {
    let (mut p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    if let Some(at) = src.find("// built-in") {
        p.builtin_ranges = std::iter::once(at as u32..src.len() as u32).collect();
    }
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = lower(&p, &t.unwrap(), &LineIndex::new(src));
    for pass in passes {
        match *pass {
            "inline" => inline::inline_small(&mut m),
            "scalarize" => scalarize::scalarize(&mut m),
            "promote" => promote::promote(&mut m),
            other => panic!("no pass {other}"),
        }
    }
    m
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no function {name}"))
}

fn index(m: &Module, name: &str) -> FuncIdx {
    FuncIdx(m.funcs.iter().position(|f| f.name == name).unwrap() as u32)
}

fn thunks(m: &Module) -> Vec<&Func> {
    m.funcs.iter().filter(|f| f.name.starts_with("__kestrel_fn_")).collect()
}

/// The indirect calls in `stmts`, as (callee, argument count, signature).
fn indirect(stmts: &[Stmt]) -> Vec<(Place, usize, KSig)> {
    let mut out = Vec::new();
    visit::walk(stmts, &mut |n| match n {
        visit::Node::Stmt(Stmt::CallIndirect { callee, args, sig })
        | visit::Node::Expr(Expr { kind: ExprKind::CallIndirect { callee, args, sig }, .. }) => {
            out.push((callee.clone(), args.len(), sig.clone()));
        }
        _ => {}
    });
    out
}

const SRC: &str = "pure square(x: int) -> int { x * x }
bump(var p: int, by: int) { p += by }
named(s: string) -> string { s }
main() {
    let f = square
    let g = square
    print(f(2), g(3))
}
";

#[test]
fn a_named_function_value_is_its_thunk_and_a_call_goes_through_it() {
    let m = lower_src(SRC, &[]);
    // One thunk per function, however often the function is named as a value.
    let ts = thunks(&m);
    assert_eq!(ts.len(), 1);
    let thunk = ts[0];
    let (square, thunk_idx) = (index(&m, "square"), index(&m, &thunk.name));
    // `(env: i64, x: int) -> int`, calling `square` with `x` and ignoring the environment.
    assert_eq!(thunk.params, vec![LocalIdx(0), LocalIdx(1)]);
    assert_eq!(thunk.locals[0].ty, KTy::I64);
    assert_eq!(thunk.locals[1].ty, KTy::I32);
    assert_eq!(thunk.ret, Some(KTy::I32));
    assert!(!thunk.exported);
    match &thunk.body[..] {
        [Stmt::Return(Some(Expr { kind: ExprKind::Call(callee, args), .. }))] => {
            assert_eq!(*callee, square);
            assert!(matches!(&args[..], [Arg::Value(Expr { kind: ExprKind::Load(p), .. })] if *p == Place::local(LocalIdx(1))));
        }
        other => panic!("{other:?}"),
    }
    let main = func(&m, "main");
    let refs = main.body.iter().filter(|s| matches!(s, Stmt::Let(_, Expr { kind: ExprKind::FnRef(t), ty: KTy::Fn, .. }) if *t == thunk_idx)).count();
    assert_eq!(refs, 2, "{:#?}", main.body);
    assert!(main.locals.iter().filter(|l| l.ty == KTy::Fn).all(|l| l.kind == LocalKind::Addressed));
    let calls = indirect(&main.body);
    assert_eq!(calls.len(), 2);
    for (callee, n, sig) in calls {
        assert!(callee.path.is_empty() && main.locals[callee.root.0 as usize].ty == KTy::Fn);
        assert_eq!(n, 1);
        assert_eq!(sig, KSig { params: vec![(KTy::I32, false)], ret: Some(KTy::I32), pure: true });
    }
}

#[test]
fn by_pointer_parameters_and_results_follow_the_function_rules() {
    let src = "struct P {
    x: int
    y: int
}
bump(var p: int, by: int) { p += by }
mk(s: string, q: P) -> P { q }
main() {
    var n = 1
    let b = bump
    b(var n, 2)
    let m = mk
    let r = m(\"a\", P(1, 2))
    print(n, r)
}
";
    let m = lower_src(src, &[]);
    let by_name = |f: &str| thunks(&m).into_iter().find(|t| matches!(&t.body[..], [Stmt::Call(c, _, _) | Stmt::Return(Some(Expr { kind: ExprKind::Call(c, _), .. }))] if *c == index(&m, f))).unwrap();
    // A `var` parameter is passed on by place, a read-only struct or string by its pointer.
    let bump = by_name("bump");
    assert_eq!([bump.locals[1].kind, bump.locals[2].kind], [LocalKind::RefParam, LocalKind::Value]);
    assert!(matches!(&bump.body[..], [Stmt::Call(_, a, _)] if a[0] == Arg::Ref(Place::local(LocalIdx(1))) && matches!(&a[1], Arg::Value(_))));
    assert_eq!(bump.ret, None);
    let mk = by_name("mk");
    assert_eq!([mk.locals[1].kind, mk.locals[2].kind], [LocalKind::RefParam, LocalKind::RefParam]);
    assert!(matches!(mk.ret, Some(KTy::Struct(_))));
    let sigs: Vec<KSig> = indirect(&func(&m, "main").body).into_iter().map(|c| c.2).collect();
    let struct_ty = mk.ret.unwrap();
    assert_eq!(sigs[0], KSig { params: vec![(KTy::I32, true), (KTy::I32, false)], ret: None, pure: false });
    assert_eq!(sigs[1], KSig { params: vec![(KTy::Str, true), (struct_ty, true)], ret: Some(struct_ty), pure: false });
}

#[test]
fn an_indirect_calls_var_argument_keeps_its_variable_in_memory() {
    let src = "bump(var p: int, by: int) { p += by }
main() {
    var n = 1
    let b = bump
    b(var n, 2)
    print(n)
}
";
    let m = lower_src(src, &["inline", "scalarize", "promote"]);
    let main = func(&m, "main");
    let n = main.locals.iter().position(|l| l.name == "n").unwrap();
    // A number passed as `var` to a direct call that inlining removed goes back to a register; one
    // passed through a function value may be changed behind its address.
    assert_eq!(main.locals[n].kind, LocalKind::Addressed, "{:#?}", main.body);
    assert_eq!(indirect(&main.body).len(), 1);
}

#[test]
fn inlining_renames_an_indirect_call_into_the_caller() {
    let src = "pure square(x: int) -> int { x * x }
apply(f: (int) -> int, x: int) -> int { f(x) }
main() {
    let g = square
    print(apply(g, 3))
}
";
    let m = lower_src(src, &["inline"]);
    let main = func(&m, "main");
    let calls = indirect(&main.body);
    assert_eq!(calls.len(), 1, "{:#?}", main.body);
    // `f` was a pointer to the caller's `g`, so the call reads `g` directly.
    assert_eq!(main.locals[calls[0].0.root.0 as usize].name, "g");
    assert!(!main.body.iter().any(|s| matches!(s, Stmt::Call(c, _, _) if m.funcs[c.0 as usize].name == "apply")));
    // The thunk is a small function too: `square` is inlined into it.
    let thunk = thunks(&m)[0];
    assert!(!thunk.body.iter().any(|s| matches!(s, Stmt::Return(Some(Expr { kind: ExprKind::Call(..), .. })))), "{:#?}", thunk.body);
}

#[derive(Debug, PartialEq)]
enum Step {
    Indirect,
    Line(u32),
    Call(String),
}

fn steps(m: &Module, name: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    visit::walk(&func(m, name).body, &mut |node| match node {
        visit::Node::Stmt(Stmt::SetLine(n)) => steps.push(Step::Line(*n)),
        visit::Node::Stmt(Stmt::CallIndirect { .. }) | visit::Node::Expr(Expr { kind: ExprKind::CallIndirect { .. }, .. }) => {
            steps.push(Step::Indirect);
        }
        visit::Node::Expr(Expr { kind: ExprKind::Call(f, _), .. }) | visit::Node::Stmt(Stmt::Call(f, _, _)) => {
            steps.push(Step::Call(m.funcs[f.0 as usize].name.clone()));
        }
        _ => {}
    });
    steps
}

#[test]
fn a_call_through_a_value_in_the_arguments_does_not_take_over_the_line() {
    let src = "main() {
    let f = double
    paint(1, f(2))
}
double(x: int) -> int { x * 2 }
// built-in
native paint(a: int, b: int)
";
    // The function value may be a function that sets a line of its own, so the call is evaluated
    // first and this call's line is stored right before `paint`.
    let m = lower_src(src, &[]);
    assert_eq!(steps(&m, "main"), [Step::Indirect, Step::Line(3), Step::Call("paint".into())]);
}

#[test]
fn promote_leaves_a_variable_alone_when_an_indirect_call_gets_it_as_var() {
    let src = "struct V {
    x: float
    y: float
}
step(var v: V, f: (var V)) {
    v.x = 1.0
    f(var v)
    v.y = v.x
}
plain(var v: V) {
    v.x = 1.0
    v.y = v.x
}
main() {}
";
    let m = lower_src(src, &["promote"]);
    let promoted = |name: &str| func(&m, name).locals.iter().any(|l| l.name.starts_with("v."));
    assert!(promoted("plain"), "control: a variable only read and written is promoted");
    assert!(!promoted("step"), "{:#?}", func(&m, "step").body);
}
