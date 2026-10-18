//! Closure literals in KIR: one function per literal, `MakeFn`, and what the passes do with them.

use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str, passes: &[&str]) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
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

fn closures(m: &Module) -> Vec<(FuncIdx, &Func)> {
    m.funcs.iter().enumerate().filter(|(_, f)| f.name.starts_with("__kestrel_closure_")).map(|(i, f)| (FuncIdx(i as u32), f)).collect()
}

/// The `MakeFn` nodes in `stmts`: the code each makes and its type.
fn make_fns(stmts: &[Stmt]) -> Vec<(FuncIdx, KTy)> {
    let mut out = Vec::new();
    visit::walk(stmts, &mut |n| {
        if let visit::Node::Expr(Expr { kind: ExprKind::MakeFn { code, env: EnvKind::None }, ty, .. }) = n {
            out.push((*code, *ty));
        }
    });
    out
}

fn returns(stmts: &[Stmt]) -> usize {
    let mut n = 0;
    visit::walk(stmts, &mut |node| {
        if let visit::Node::Stmt(Stmt::Return(_)) = node {
            n += 1;
        }
    });
    n
}

const SRC: &str = "main() {
    let sq = (x: int) -> int { x * x }
    let hurt = (n: int, s: string) { print(n, s) }
    print(sq(3))
    hurt(1, \"a\")
}
";

#[test]
fn a_closure_is_a_function_taking_the_environment_first() {
    let m = lower_src(SRC, &[]);
    let cs = closures(&m);
    assert_eq!(cs.len(), 2);
    let (_, sq) = cs[0];
    // `(env: i64, x: int) -> int`
    assert_eq!(sq.params, vec![LocalIdx(0), LocalIdx(1)]);
    assert_eq!((sq.locals[0].name.as_str(), sq.locals[0].ty, sq.locals[0].kind), ("env", KTy::I64, LocalKind::Value));
    assert_eq!((sq.locals[1].name.as_str(), sq.locals[1].ty), ("x", KTy::I32));
    assert_eq!(sq.ret, Some(KTy::I32));
    assert!(sq.is_pure && !sq.exported && sq.native.is_none());
    // A string parameter is passed by pointer, like a function's.
    let (_, hurt) = cs[1];
    assert_eq!(hurt.params.len(), 3);
    assert_eq!((hurt.locals[2].ty, hurt.locals[2].kind), (KTy::Str, LocalKind::RefParam));
    assert_eq!(hurt.ret, None);
    assert!(!hurt.is_pure && hurt.io);
}

#[test]
fn the_literal_makes_a_function_value_of_the_code_with_no_environment() {
    let m = lower_src(SRC, &[]);
    let main = func(&m, "main");
    let made = make_fns(&main.body);
    let cs = closures(&m);
    assert_eq!(made, vec![(cs[0].0, KTy::Fn), (cs[1].0, KTy::Fn)]);
    // The closures' locals are theirs: `main` has its own two (the function values).
    assert!(main.locals.iter().all(|l| l.name != "x" && l.name != "n"));
}

#[test]
fn return_leaves_the_closure_and_nothing_else() {
    let m = lower_src(
        "main() {
    let early = (x: int) -> int {
        if x < 0 { return 0 }
        x
    }
    print(early(5))
}
",
        &[],
    );
    let (_, early) = closures(&m)[0];
    assert_eq!(returns(&early.body), 2);
    assert_eq!(returns(&func(&m, "main").body), 0);
}

#[test]
fn a_field_default_is_one_function_however_often_it_is_built() {
    let m = lower_src(
        "struct Button {
    on_click: (int) = (n) { print(n) }
}
main() {
    let a = Button()
    let b = Button()
    var i = 0
    while i < 2 {
        let c = Button()
        i += 1
    }
}
",
        &[],
    );
    let cs = closures(&m);
    assert_eq!(cs.len(), 1);
    let made = make_fns(&func(&m, "main").body);
    assert_eq!(made.len(), 3);
    assert!(made.iter().all(|&(code, _)| code == cs[0].0));
}

#[test]
fn a_constant_closure_is_one_function_at_every_use() {
    let m = lower_src(
        "let DOUBLE = (x: int) -> int { x * 2 }
main() {
    print(DOUBLE(1), DOUBLE(2))
    let f = DOUBLE
    print(f(3))
}
",
        &[],
    );
    assert_eq!(closures(&m).len(), 1);
}

#[test]
fn a_closure_in_a_hoisted_constant_is_built_before_main() {
    let m = lower_src(
        "let TABLE = [(x: int) -> int { x + 1 }, (x: int) -> int { x * 2 }]
main() {
    print(TABLE[1](5))
}
",
        &[],
    );
    assert_eq!(closures(&m).len(), 2);
    let init = m.init.expect("a heap constant has an initializer");
    assert_eq!(make_fns(&m.funcs[init.0 as usize].body).len(), 2);
}

#[test]
fn nested_literals_are_functions_of_their_own() {
    let m = lower_src(
        "main() {
    let outer = () -> (int) -> int { (x: int) -> int { x + 1 } }
    print(outer()(1))
}
",
        &[],
    );
    let cs = closures(&m);
    assert_eq!(cs.len(), 2);
    // The outer one makes the inner one; `main` makes only the outer.
    assert_eq!(make_fns(&func(&m, "main").body).len(), 1);
    let inner = cs.iter().find(|(_, f)| make_fns(&f.body).is_empty()).expect("the inner closure");
    let outer = cs.iter().find(|(_, f)| !make_fns(&f.body).is_empty()).expect("the outer closure");
    assert_eq!(make_fns(&outer.1.body), vec![(inner.0, KTy::Fn)]);
}

#[test]
fn the_passes_leave_closures_working() {
    // The visitors, the renamer and the inliner all see `MakeFn`, and the closure is not inlined
    // into the caller by name (it is only reached through the function value).
    let m = lower_src(
        "inc(f: (int) -> int, x: int) -> int { f(x) }
main() {
    let a = (x: int) -> int { x + 1 }
    print(inc(a, 1))
    print(inc((x) { x * 3 }, 2))
}
",
        &["inline", "scalarize", "promote"],
    );
    assert_eq!(closures(&m).len(), 2);
    assert_eq!(make_fns(&func(&m, "main").body).len(), 2);
}
