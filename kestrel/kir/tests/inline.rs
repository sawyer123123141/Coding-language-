mod range_common;

use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_inlined(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = lower(&p, &t.unwrap(), &LineIndex::new(src));
    inline::inline_small(&mut m);
    m
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

fn calls(stmts: &[Stmt]) -> usize {
    let mut n = 0;
    visit::walk(stmts, &mut |node| {
        if matches!(node, visit::Node::Expr(Expr { kind: ExprKind::Call(..), .. }) | visit::Node::Stmt(Stmt::Call(..))) {
            n += 1;
        }
    });
    n
}

#[test]
fn small_functions_are_inlined_and_recursive_ones_are_not() {
    let src = "pure sq(x: int) -> int { x * x }\npure fib(n: int) -> int {\n if n < 2 { n } else { fib(n - 1) + fib(n - 2) }\n}\nf(a: int) -> int { sq(a) + sq(a + 1) + fib(a) }\nmain() {}\n";
    let m = lower_inlined(src);
    // Only the call to `fib` is left.
    assert_eq!(calls(&func(&m, "f").body), 1);
    assert_eq!(calls(&func(&m, "fib").body), 2);
}

#[test]
fn var_self_methods_inline_onto_the_receiver_place() {
    let src = "struct P {\n x: int\n bump(var self, by: int) {\n  self.x += by\n }\n}\nmain() {\n var ps = [P(1), P(2)]\n ps[1].bump(5)\n print(ps[1].x)\n}\n";
    let m = lower_inlined(src);
    let main = func(&m, "main");
    assert_eq!(calls(&main.body), 0, "{:#?}", main.body);
    // The receiver's index was evaluated once, into a temporary, and the body assigns through it.
    let assigns: Vec<&Place> = main.body.iter().filter_map(|s| if let Stmt::Assign(p, _) = s { Some(p) } else { None }).collect();
    assert!(matches!(assigns[..], [p] if matches!(p.path[..], [Proj::Index { .. }, Proj::Field(0)])), "{:#?}", main.body);
}

/// A pure function of `x` whose body is a sum of `terms` products: about
/// four KIR nodes per term.
fn sum_fn(name: &str, terms: usize) -> String {
    let body: Vec<String> = (1..=terms).map(|i| format!("x * {i}")).collect();
    format!("pure {name}(x: int) -> int {{ {} }}\n", body.join(" + "))
}

#[test]
fn mid_sized_functions_are_inlined() {
    // Like a noise octave: too big for the old 64-node limit.
    let src = format!("{}f(a: int) -> int {{ mid(a) + mid(a + 1) }}\nmain() {{}}\n", sum_fn("mid", 30));
    let m = lower_inlined(&src);
    assert_eq!(calls(&func(&m, "f").body), 0);
}

#[test]
fn calls_in_the_arguments_of_inlined_calls_are_inlined() {
    // Like `mix(lattice(..), lattice(..), t)` in a noise octave.
    let src = "pure sq(x: int) -> int { x * x }\npure add(a: int, b: int) -> int { a + b }\nf(a: int) -> int { add(sq(a), sq(sq(a + 1))) }\nmain() {}\n";
    let m = lower_inlined(src);
    assert_eq!(calls(&func(&m, "f").body), 0, "{:#?}", func(&m, "f").body);
}

#[test]
fn chains_deeper_than_four_calls_are_inlined() {
    // solids -> density -> octave -> lattice -> hash, inside a mesher.
    let src = "pure e(x: int) -> int { x + 1 }\npure d(x: int) -> int { e(x) * 2 }\npure c(x: int) -> int { d(x) + 3 }\npure b(x: int) -> int { c(x) - 4 }\npure a(x: int) -> int { b(x) * 5 }\nf(x: int) -> int { a(x) }\nmain() {}\n";
    let m = lower_inlined(src);
    assert_eq!(calls(&func(&m, "f").body), 0, "{:#?}", func(&m, "f").body);
}

#[test]
fn inlining_stops_when_the_caller_has_grown_enough() {
    let uses: Vec<String> = (0..60).map(|i| format!("mid(a + {i})")).collect();
    let src = format!("{}f(a: int) -> int {{ {} }}\nmain() {{}}\n", sum_fn("mid", 30), uses.join(" + "));
    let m = lower_inlined(&src);
    let left = calls(&func(&m, "f").body);
    assert!(left > 0 && left < 60, "{left} of 60 calls left");
}

#[test]
fn functions_with_early_returns_stay_calls() {
    let src = "pick(a: int) -> int {\n if a > 0 { return 1 }\n 2\n}\nf() -> int { pick(3) }\nmain() {}\n";
    let m = lower_inlined(src);
    assert_eq!(calls(&func(&m, "f").body), 1);
}

#[test]
fn the_constants_init_is_correct_whether_or_not_it_is_inlined_into_main() {
    let m = lower_inlined("let ITEMS = [1, 2, 3]\nmain() {\n print(ITEMS[0])\n print(\"after\")\n}\n");
    let main = func(&m, "main");
    let init = m.init.expect("a used heap constant needs the init function");
    let (mut returns, mut prints, mut pops) = (0, 0, 0);
    visit::walk(&main.body, &mut |n| match n {
        visit::Node::Stmt(Stmt::Return(_)) => returns += 1,
        visit::Node::Stmt(Stmt::Print(_)) => prints += 1,
        visit::Node::Stmt(Stmt::PopArena { .. }) => pops += 1,
        _ => {}
    });
    // Still called, or spliced in with its build; either way nothing in it ends `main` early.
    assert!(main.body.contains(&Stmt::Call(init, Vec::new(), 0)) || pops == 1, "{:#?}", main.body);
    assert_eq!(returns, 0, "{:#?}", main.body);
    assert_eq!(prints, 2, "{:#?}", main.body);
}

#[test]
fn a_return_inside_the_tail_if_keeps_the_function_a_call() {
    let src = "g(x: int) -> int { if x > 0 { return 1 } else { 7 } }
f(a: int) -> int { g(a) + g(a - 1) }
main() {}
";
    let m = lower_inlined(src);
    assert_eq!(calls(&func(&m, "f").body), 2, "{:#?}", func(&m, "f").body);
}

/// The KIR nodes in `stmts`.
fn nodes(stmts: &[Stmt]) -> usize {
    let mut n = 0;
    visit::walk(stmts, &mut |_| n += 1);
    n
}

/// The calls in `stmts` to the function called `name`.
fn calls_to(m: &Module, stmts: &[Stmt], name: &str) -> usize {
    let target = FuncIdx(m.funcs.iter().position(|f| f.name == name).unwrap() as u32);
    let mut n = 0;
    visit::walk(stmts, &mut |node| match node {
        visit::Node::Expr(Expr { kind: ExprKind::Call(c, _), .. }) | visit::Node::Stmt(Stmt::Call(c, _, _)) if *c == target => n += 1,
        _ => {}
    });
    n
}

/// The reasons recorded for calls from `caller` to `callee`.
fn reasons(m: &Module, caller: &str, callee: &str) -> Vec<String> {
    let name = |i: FuncIdx| m.funcs[i.0 as usize].name.as_str();
    m.not_inlined.iter().filter(|n| name(n.caller) == caller && name(n.callee) == callee).map(|n| n.reason.clone()).collect()
}

/// A number with a comma every three digits, as the inliner's reasons write sizes.
fn commas(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `mid`: a loop of 30 calls to `leaf` (a 40-term sum): small as written, over 4,096 nodes once
/// its calls are inlined.
fn leaf_and_mid() -> String {
    let uses: Vec<String> = (0..30).map(|i| format!("leaf(i + {i})")).collect();
    format!("{}pure mid(n: int) -> int {{\n var t = 0\n for i from 0 to n {{\n  t += {}\n }}\n t\n}}\n", sum_fn("leaf", 40), uses.join(" + "))
}

#[test]
fn a_big_middle_function_stays_a_call_and_keeps_its_leaves_inlined() {
    let src = format!("{}root(n: int) -> int {{ mid(n) + 1 }}\nmain() {{}}\n", leaf_and_mid());
    let m = lower_inlined(&src);
    let mid = func(&m, "mid");
    assert_eq!(calls_to(&m, &mid.body, "leaf"), 0, "every leaf call is inlined into `mid`");
    let post = nodes(&mid.body);
    assert!(post > 4096, "`mid` is {post} nodes after its own inlining");
    assert_eq!(calls_to(&m, &func(&m, "root").body, "mid"), 1);
    let want = format!("it is {} nodes after its own inlining; the limit is 4,096", commas(post));
    assert_eq!(reasons(&m, "root", "mid"), [want]);
}

#[test]
fn mutual_recursion_inlines_one_level() {
    let src = "a(n: int) -> int { if n <= 0 { 0 } else { b(n - 1) + 1 } }\nb(n: int) -> int { if n <= 0 { 1 } else { a(n - 1) * 2 } }\nmain() { print(a(5)) }\n";
    let m = lower_inlined(src);
    let a = func(&m, "a");
    // `b`'s body is in `a` once (one copy of its parameter), and its call back into `a` stays.
    assert_eq!(a.locals.iter().filter(|l| l.name == "b.n").count(), 1, "{:?}", a.locals);
    assert_eq!((calls_to(&m, &a.body, "a"), calls_to(&m, &a.body, "b")), (1, 0), "{:#?}", a.body);
    assert_eq!(reasons(&m, "a", "a"), ["it calls back into `a`"]);
    // And the other way round in `b`.
    assert_eq!(reasons(&m, "b", "b"), ["it calls back into `b`"]);
    assert!(m.not_inlined.iter().all(|n| !n.reason.contains("nest more than")), "{:?}", m.not_inlined);
}

#[test]
fn left_calls_are_reported_under_each_caller() {
    // `mid` stays a call in `g` (too big after its own inlining); `g` is inlined into `f1` and
    // `f2`, each of which then holds that call.
    let src = format!("{}g(n: int) -> int {{ mid(n) * 2 }}\nf1(n: int) -> int {{ g(n) + 1 }}\nf2(n: int) -> int {{ g(n) - 1 }}\nmain() {{}}\n", leaf_and_mid());
    let m = lower_inlined(&src);
    let own = reasons(&m, "g", "mid");
    let [why] = &own[..] else { panic!("{:?}", m.not_inlined) };
    assert!(why.ends_with("nodes after its own inlining; the limit is 4,096"), "{why}");
    assert_eq!(calls_to(&m, &func(&m, "f1").body, "g"), 0);
    assert_eq!(reasons(&m, "f1", "mid"), own);
    assert_eq!(reasons(&m, "f2", "mid"), own);
}

#[test]
fn a_twice_inlined_local_keeps_its_name() {
    use kestrel_kir::range::render::render;
    let src = "inner(a: int) -> int {\n let n = a * 3\n n * 1000\n}\nouter(a: int) -> int {\n inner(a)\n}\npub top(a: int) -> int {\n outer(a)\n}";
    let (m, r) = range_common::reported(src, kestrel_kir::range::Arith::Checked);
    let top = func(&m, "top");
    // One prefix: the function the local was written in.
    assert!(top.locals.iter().any(|l| l.name == "inner.n"), "{:?}", top.locals);
    let text = render(&r, &m, &|_| true);
    assert!(text.contains("(in `inner`): overflow check: `n * 1000` may exceed `int`"), "{text}");
    assert!(!text.contains("(...)"), "{text}");
}

#[test]
fn a_fan_out_chain_stays_small() {
    // Six functions, each summing 20 calls of the next.
    let mut src = String::from("pure f5(x: int) -> int { x * 3 + 1 }\n");
    for i in (0..5).rev() {
        let uses: Vec<String> = (0..20).map(|k| format!("f{}(x + {k})", i + 1)).collect();
        src.push_str(&format!("pure f{i}(x: int) -> int {{ {} }}\n", uses.join(" + ")));
    }
    src.push_str("main() {\n var s = 0\n for i from 0 to 10 {\n  s += f0(i)\n }\n print(s)\n}\n");
    let start = std::time::Instant::now();
    let m = lower_inlined(&src);
    let took = start.elapsed();
    let total: usize = m.funcs.iter().map(|f| nodes(&f.body)).sum();
    assert!(total < 100_000, "{total} KIR nodes after inlining");
    assert!(took < std::time::Duration::from_secs(10), "inlining took {took:?}");
}
