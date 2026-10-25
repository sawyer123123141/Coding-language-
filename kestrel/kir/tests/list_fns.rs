//! `filter`, `map`, `any` and `all`: a generated loop per call site that calls the literal's code
//! directly with its stack environment, so `inline_small` can inline the closure into the loop.
//! A function value that isn't a literal is called indirectly.

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
    m.funcs.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no function {name}"))
}

/// The generated loops, in order.
fn helpers(m: &Module) -> Vec<&Func> {
    m.funcs.iter().filter(|f| f.name.starts_with("__kestrel_list_fn_")).collect()
}

/// The functions `f` calls directly, by name, and how many indirect calls it makes.
fn calls(m: &Module, f: &Func) -> (Vec<String>, usize) {
    let (mut direct, mut indirect) = (Vec::new(), 0);
    visit::walk(&f.body, &mut |n| match n {
        visit::Node::Expr(Expr { kind: ExprKind::Call(callee, _), .. }) | visit::Node::Stmt(Stmt::Call(callee, _, _)) => {
            direct.push(m.funcs[callee.0 as usize].name.clone())
        }
        visit::Node::Expr(Expr { kind: ExprKind::CallIndirect { .. }, .. }) | visit::Node::Stmt(Stmt::CallIndirect { .. }) => indirect += 1,
        _ => {}
    });
    (direct, indirect)
}

fn closure_calls(m: &Module, f: &Func) -> usize {
    calls(m, f).0.iter().filter(|n| n.starts_with("__kestrel_closure_")).count()
}

const FILTER: &str = "main() {
    let xs = [1, 2, 3]
    let k = 2
    print(xs.filter((e) { e > k }).len)
}
";

#[test]
fn the_loop_calls_the_literals_code_directly_with_its_stack_environment() {
    let m = lower_src(FILTER);
    assert_eq!(check_stack_envs(&m), Ok(()));
    let hs = helpers(&m);
    assert_eq!(hs.len(), 1);
    // The helper takes the list and the function value, and calls the code with the
    // environment word it reads from the value.
    assert_eq!(hs[0].params.len(), 2);
    assert_eq!(hs[0].locals[1].ty, KTy::Fn);
    assert_eq!(closure_calls(&m, hs[0]), 1);
    assert_eq!(calls(&m, hs[0]).1, 0);
    let mut env_reads = 0;
    visit::walk(&hs[0].body, &mut |n| {
        if matches!(n, visit::Node::Expr(Expr { kind: ExprKind::FnEnv(_), .. })) {
            env_reads += 1;
        }
    });
    assert_eq!(env_reads, 1);
    // `main` passes the literal straight to the helper, as a stack environment.
    let mut stack_args = 0;
    visit::walk(&func(&m, "main").body, &mut |n| {
        if let visit::Node::Expr(Expr { kind: ExprKind::Call(callee, args), .. }) = n {
            if m.funcs[callee.0 as usize].name.starts_with("__kestrel_list_fn_") {
                stack_args += args.iter().filter(|a| matches!(a, Arg::Value(Expr { kind: ExprKind::MakeFn { env: EnvKind::Stack(_), .. }, .. }))).count();
            }
        }
    });
    assert_eq!(stack_args, 1);
}

#[test]
fn the_literals_closure_is_inlined_into_the_loop() {
    let mut m = lower_src(FILTER);
    optimize(&mut m);
    let hs = helpers(&m);
    assert_eq!(hs.len(), 1);
    assert_eq!(closure_calls(&m, hs[0]), 0, "{:#?}", hs[0].body);
    assert_eq!(calls(&m, hs[0]).1, 0);
}

#[test]
fn every_method_inlines_its_literal() {
    let mut m = lower_src(
        "main() {
    let xs = [1, 2, 3]
    let k = 2
    print(xs.filter((e) { e > k }).len)
    print(xs.map((e) { e * k }).len)
    print(xs.any((e) { e == k }))
    print(xs.all((e) { e < k }))
}
",
    );
    let before = helpers(&m).len();
    assert_eq!(before, 4);
    optimize(&mut m);
    for h in helpers(&m) {
        assert_eq!(closure_calls(&m, h), 0, "{}: {:#?}", h.name, h.body);
    }
}

#[test]
fn a_function_value_that_is_not_a_literal_is_called_indirectly() {
    let mut m = lower_src(
        "main() {
    let xs = [1, 2, 3]
    let k = 2
    let f = (e: int) -> bool { e > k }
    print(xs.filter(f).len)
    print(xs.any(f))
}
",
    );
    assert_eq!(check_stack_envs(&m), Ok(()));
    optimize(&mut m);
    let hs = helpers(&m);
    assert_eq!(hs.len(), 2);
    for h in hs {
        assert_eq!(closure_calls(&m, h), 0);
        assert_eq!(calls(&m, h).1, 1, "{}: {:#?}", h.name, h.body);
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    Line(u32),
    Call(String),
}

/// The line stores and calls in `main`, in the order they are met.
fn steps(src: &str) -> Vec<Step> {
    let (mut p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    p.builtin_ranges = std::iter::once(src.find("// built-in").expect("a built-in part") as u32..src.len() as u32).collect();
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let m = lower(&p, &t.unwrap(), &LineIndex::new(src));
    let mut steps = Vec::new();
    visit::walk(&func(&m, "main").body, &mut |node| match node {
        visit::Node::Stmt(Stmt::SetLine(n)) => steps.push(Step::Line(*n)),
        visit::Node::Expr(Expr { kind: ExprKind::Call(f, _), .. }) | visit::Node::Stmt(Stmt::Call(f, _, _)) => {
            steps.push(Step::Call(m.funcs[f.0 as usize].name.clone()))
        }
        _ => {}
    });
    steps
}

fn is_line(s: &Step, line: u32) -> bool {
    matches!(s, Step::Line(n) if *n == line || (*n & 0xFFFF) == line)
}

#[test]
fn a_list_method_in_the_arguments_does_not_take_over_the_line() {
    // The loop calls a closure that calls `paint`, which stores the closure's line. So the
    // method is evaluated first, and the outer call's line is stored right before `paint`.
    let impure = "main() {
    let xs = [1, 2]
    paint(1, xs.map((e) {
        paint(e, e)
        e
    }).len)
}
// built-in
native paint(a: int, b: int)
";
    let s = steps(impure);
    let at = |name: &str| s.iter().position(|x| matches!(x, Step::Call(n) if n.starts_with(name))).unwrap_or_else(|| panic!("{s:?}"));
    let outer = s.iter().rposition(|x| is_line(x, 3)).unwrap_or_else(|| panic!("no line 3 in {s:?}"));
    assert!(at("__kestrel_list_fn_") < outer, "{s:?}");
    assert_eq!(s[outer + 1], Step::Call("paint".into()), "{s:?}");
    // The same for a function value that isn't of a `pure` type.
    let value = "main() {
    let xs = [1, 2]
    let f = (e: int) -> int {
        paint(e, e)
        e
    }
    paint(1, xs.map(f).len)
}
// built-in
native paint(a: int, b: int)
";
    let s = steps(value);
    let outer = s.iter().rposition(|x| is_line(x, 7)).unwrap_or_else(|| panic!("no line 7 in {s:?}"));
    assert!(s.iter().position(|x| matches!(x, Step::Call(n) if n.starts_with("__kestrel_list_fn_"))).unwrap() < outer, "{s:?}");
}

#[test]
fn a_pure_literal_leaves_the_arguments_in_place() {
    let src = "main() {
    let xs = [1, 2]
    paint(1, xs.map((e) { e }).len)
}
// built-in
native paint(a: int, b: int)
";
    let s = steps(src);
    let line = s.iter().position(|x| is_line(x, 3)).unwrap_or_else(|| panic!("{s:?}"));
    let call = s.iter().position(|x| matches!(x, Step::Call(n) if n.starts_with("__kestrel_list_fn_"))).unwrap();
    assert!(line < call, "a closure that can't call a built-in function needs no care: {s:?}");
}

/// The reads of a list item (an indexed place) in `stmts`.
fn item_reads(stmts: &[Stmt]) -> usize {
    let mut n = 0;
    visit::walk(stmts, &mut |node| {
        if let visit::Node::Expr(Expr { kind: ExprKind::Load(p), .. }) = node {
            if p.path.iter().any(|s| matches!(s, Proj::Index { .. })) {
                n += 1;
            }
        }
    });
    n
}

#[test]
fn filter_reads_a_scalar_item_once_and_an_unused_environment_word_not_at_all() {
    let mut m = lower_src("main() {
    let xs = [1, 2, 3]
    print(xs.filter((e) { e % 2 == 0 }).len)
}
");
    optimize(&mut m);
    let hs = helpers(&m);
    assert_eq!(hs.len(), 1);
    assert_eq!(closure_calls(&m, hs[0]), 0, "{:#?}", hs[0].body);
    // One read of `xs[i]` for the test and the copy.
    assert_eq!(item_reads(&hs[0].body), 1, "{:#?}", hs[0].body);
    // The literal captures nothing, so its inlined body never needs the environment word.
    let mut env_reads = 0;
    visit::walk(&hs[0].body, &mut |node| {
        if let visit::Node::Expr(Expr { kind: ExprKind::FnEnv(_), .. }) = node {
            env_reads += 1;
        }
    });
    assert_eq!(env_reads, 0, "{:#?}", hs[0].body);
}

#[test]
fn filter_of_structs_and_strings_still_copies_the_kept_item() {
    let mut m = lower_src("struct P {
    x: int
    name: string
}

main() {
    let ps = [P(x: 1, name: \"a\"), P(x: 2, name: \"b\")]
    let k = 1
    print(ps.filter((p) { p.x > k }).len)
}
");
    optimize(&mut m);
    let hs = helpers(&m);
    // The captured `k` is read through the environment word, so it stays.
    let mut env_reads = 0;
    visit::walk(&hs[0].body, &mut |node| {
        if let visit::Node::Expr(Expr { kind: ExprKind::FnEnv(_), .. }) = node {
            env_reads += 1;
        }
    });
    assert!(env_reads >= 1, "{:#?}", hs[0].body);
    assert!(hs[0].locals.iter().any(|l| l.name == "copy"), "{:#?}", hs[0].locals);
}
