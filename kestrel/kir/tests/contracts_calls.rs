//! What contracts follow through calls: the summaries' chains for checks, unknown calls, give-ups,
//! cuts and recursion, and what an interface call and growth count for. (The errors themselves:
//! `tests/errors/prove_*.kes` and `cli/tests/contracts.rs`.)

mod range_common;

use kestrel_kir::range::events::{events, Event};
use kestrel_kir::range::summary::{summarize, Summary};
use kestrel_kir::range::contracts::check;
use kestrel_kir::range::{remove_checks, Analysis, Arith, FuncChecks};
use kestrel_kir::*;
use range_common::{idx, line, optimized};

/// Every function's events.
fn all_events(m: &Module) -> Vec<Vec<Event>> {
    m.funcs.iter().map(|f| if f.native.is_some() { Vec::new() } else { events(m, f) }).collect()
}

/// `src` optimised and analysed (checked), and every function's summary.
fn summaries(src: &str) -> (Module, Vec<Summary>) {
    let mut m = optimized(src);
    let a = remove_checks(&mut m, Arith::Checked);
    let s = summarize(&m, &a, &all_events(&m), None);
    (m, s)
}

fn of<'a>(m: &Module, s: &'a [Summary], name: &str) -> &'a Summary {
    &s[idx(m, name)]
}

/// A function that stays a call: it has an early `return`.
const KEEP: &str = "if d < 0 {\n        return 0\n    }";

#[test]
fn the_checks_chain_follows_calls_and_cycles() {
    // `a` and `b` call each other; `b` has a bounds check. `c` and `d` call each other and have
    // none (`n - 1` is proven: `n > 0`).
    let src = format!(
        "pub a(xs: list[int], i: int, d: int) -> int {{\n    {KEEP}\n    b(xs, i, d - 1)\n}}
pub b(xs: list[int], i: int, d: int) -> int {{\n    {KEEP}\n    a(xs, i, d) ^ xs[i]\n}}
pub top(xs: list[int]) -> int {{ a(xs, 0, 3) }}
pub c(d: int) -> int {{\n    {KEEP}\n    d2(d)\n}}
pub d2(d: int) -> int {{\n    if d > 0 {{\n        return c(d - 1)\n    }}\n    0\n}}"
    );
    let (m, s) = summaries(&src);
    for name in ["a", "b", "top"] {
        let chain = of(&m, &s, name).checks.as_ref().unwrap_or_else(|| panic!("`{name}` reaches the check"));
        assert_eq!(chain.reason.via, "has a bounds check", "{name}");
        assert_eq!(line(chain.reason.line), 11, "{name}");
    }
    assert!(of(&m, &s, "b").checks.as_ref().unwrap().steps.is_empty(), "`b` has it itself");
    for name in ["c", "d2"] {
        assert_eq!(of(&m, &s, name).checks, None, "{name}");
    }
}

#[test]
fn recursion_is_found_through_a_cycle() {
    let src = format!(
        "pub a(d: int) -> int {{\n    {KEEP}\n    b(d)\n}}
pub b(d: int) -> int {{\n    {KEEP}\n    a(d)\n}}
pub top(d: int) -> int {{ a(d) }}
pub self_call(d: int) -> int {{\n    {KEEP}\n    self_call(d)\n}}
pub leaf(d: int) -> int {{ d ^ 1 }}"
    );
    let (m, s) = summaries(&src);
    for name in ["a", "b", "self_call"] {
        let r = of(&m, &s, name).recursion.as_ref().unwrap_or_else(|| panic!("`{name}` is on a cycle"));
        assert!(r.steps.is_empty(), "{name}: its own");
        assert_eq!((r.reason.via.as_str(), r.reason.tail.as_str()), ("can call itself", ", so the stack may run out"));
    }
    let top = of(&m, &s, "top").recursion.as_ref().expect("`top` reaches a cycle");
    assert_eq!(top.steps.len(), 1);
    assert_eq!(of(&m, &s, "leaf").recursion, None);
}

#[test]
fn an_unknown_call_below_a_call_is_unknown_to_the_caller() {
    // `relay` stays a call (it calls itself) and calls through what it is given; `Box.size`
    // promises `no_alloc` only.
    let src = format!(
        "interface Shape {{\n    area(self) -> int\n}}
interface Sized {{\n    @prove(no_alloc)\n    size(self) -> int\n}}
pub relay(s: Shape, d: int) -> int {{\n    {KEEP}\n    s.area() ^ relay(s, d - 1)\n}}
pub relay_fn(f: (int) -> int, d: int) -> int {{\n    {KEEP}\n    f(d) ^ relay_fn(f, d - 1)\n}}
pub relay_sized(s: Sized, d: int) -> int {{\n    {KEEP}\n    s.size() ^ relay_sized(s, d - 1)\n}}
pub top(s: Shape, f: (int) -> int, t: Sized) -> int {{ relay(s, 1) ^ relay_fn(f, 1) ^ relay_sized(t, 1) }}"
    );
    let (m, s) = summaries(&src);
    // (checks, alloc, panic) for each.
    let unknown = |name: &str| {
        let x = of(&m, &s, name);
        (x.unknown_checks.is_some(), x.unknown_alloc.is_some(), x.unknown_panic.is_some())
    };
    assert_eq!(unknown("relay"), (true, true, true));
    assert_eq!(unknown("relay_fn"), (true, true, true));
    assert_eq!(unknown("relay_sized"), (true, false, true), "it keeps `no_alloc` only");
    assert_eq!(unknown("top"), (true, true, true), "below a call");
    let chain = of(&m, &s, "top").unknown_checks.as_ref().unwrap();
    assert_eq!(chain.steps.len(), 1);
    assert!(chain.reason.via.starts_with("calls a method of the interface value `s`"), "{:?}", chain.reason);
    assert_eq!(chain.reason.tail, "; its body isn't known here");
}

/// `pub name(x: int) -> int` with 300 nested `if`s: range analysis gives it up.
fn deep(name: &str) -> String {
    let mut src = format!("pub {name}(x: int) -> int {{\n    var y = 0\n");
    for d in 0..300 {
        src.push_str(&format!("if x > {d} {{\n"));
    }
    src.push_str("y = x ^ 2\n");
    for _ in 0..300 {
        src.push_str("}\n");
    }
    src.push_str("y\n}\n");
    src
}

#[test]
fn a_give_up_is_unknown_to_every_caller() {
    let src = format!("{}\npub mid(x: int) -> int {{ g(x) }}\npub top(x: int) -> int {{\n    {KEEP_X}\n    mid(x) ^ top(x)\n}}\n", deep("g"), KEEP_X = "if x < 0 {\n        return 0\n    }");
    let big = |f: Box<dyn FnOnce() + Send>| std::thread::Builder::new().stack_size(64 << 20).spawn(f).unwrap().join().unwrap();
    big(Box::new(move || {
        let (m, s) = summaries(&src);
        for name in ["g", "mid", "top"] {
            let x = of(&m, &s, name);
            for (what, chain) in [("checks", &x.unknown_checks), ("alloc", &x.unknown_alloc), ("panic", &x.unknown_panic)] {
                let chain = chain.as_ref().unwrap_or_else(|| panic!("`{name}`: unknown for {what}"));
                assert_eq!(chain.reason.tail, ", where range analysis stopped early (its code nests too deeply)", "{name}");
            }
        }
        assert!(of(&m, &s, "g").unknown_checks.as_ref().unwrap().steps.is_empty());
    }));
}

#[test]
fn a_cut_walk_is_unknown_to_every_caller() {
    let big = |f: Box<dyn FnOnce() + Send>| std::thread::Builder::new().stack_size(256 << 20).spawn(f).unwrap().join().unwrap();
    big(Box::new(|| {
        let mut m = optimized("pub g() {}\npub f() {\n    g()\n}");
        let truth = Expr { kind: ExprKind::Bool(true), ty: KTy::Bool, line: 0 };
        let mut body = Vec::new();
        for _ in 0..5000 {
            body = vec![Stmt::If { cond: truth.clone(), then_body: body, else_body: Vec::new() }];
        }
        let (g, f) = (idx(&m, "g"), idx(&m, "f"));
        m.funcs[g].body = body;
        m.funcs[f].body = vec![Stmt::Call(FuncIdx(g as u32), Vec::new(), pack_line(0, 3))];
        // An analysis that found nothing wrong: only the cut says the walk stopped.
        let a = Analysis { funcs: vec![FuncChecks::default(); m.funcs.len()], assumptions: Vec::new() };
        let s = summarize(&m, &a, &all_events(&m), None);
        for (name, steps) in [("g", 0), ("f", 1)] {
            let x = &s[idx(&m, name)];
            for chain in [&x.unknown_checks, &x.unknown_alloc, &x.unknown_panic] {
                let chain = chain.as_ref().unwrap_or_else(|| panic!("`{name}` is unknown"));
                assert_eq!(chain.steps.len(), steps, "{name}");
                assert_eq!(chain.reason.tail, ", where range analysis stopped early (its code nests too deeply)");
            }
        }
    }));
}

#[test]
fn growth_is_not_a_failure_for_contracts() {
    let src = "pub push(var ys: list[int]) {\n    ys.add(1)\n}
pub put(var xs: list[int], i: int) {\n    xs.insert(i, 1)\n}
pub sized(n: int) -> int {\n    let a = array[int](n)\n    a.len\n}";
    let (m, s) = summaries(src);
    let push = of(&m, &s, "push");
    assert!(push.fails.is_some(), "the report lists the item limit");
    assert_eq!(push.panics, None, "running out of resources isn't a way to fail for `no_panic`");
    // `insert`'s bad index is a logic failure; the report's item says both.
    let put = of(&m, &s, "put");
    assert_eq!(put.panics.as_ref().map(|c| c.reason.direct.clone()), Some(Some("`insert` fails if `i` is outside `xs`".to_string())));
    assert!(put.fails.as_ref().unwrap().reason.direct.as_ref().unwrap().contains("or `xs` would hold more than"), "{:?}", put.fails);
    // A length out of range is the caller's logic.
    assert!(of(&m, &s, "sized").panics.is_some());
}

#[test]
fn a_promised_interface_call_is_not_an_allocation() {
    let src = "interface Shape {\n    @prove(no_alloc)\n    area(self) -> int\n}
interface Sized {\n    size(self) -> int\n}
pub f(s: Shape) -> int { s.area() }
pub g(s: Sized) -> int { s.size() }";
    let (m, s) = summaries(src);
    let f = of(&m, &s, "f");
    assert_eq!((f.allocates.as_ref(), f.unknown_alloc.as_ref()), (None, None), "{f:?}");
    assert!(f.fails.is_some() && f.unknown_panic.is_some() && f.unknown_checks.is_some(), "it promises nothing else");
    let g = of(&m, &s, "g");
    assert!(g.allocates.is_some() && g.unknown_alloc.is_some());
    // The call carries what the method promises.
    let mut keeps = Vec::new();
    visit::walk(&m.funcs[idx(&m, "f")].body, &mut |n| {
        if let visit::Node::Expr(Expr { kind: ExprKind::CallVirtual { keeps: k, method, .. }, .. }) = n {
            keeps.push((*k, m.funcs[method.0 as usize].name.clone()));
        }
    });
    assert_eq!(keeps, [(ContractSet::NO_ALLOC, "Shape.area".to_string())]);
}

#[test]
fn a_loop_whose_block_is_nowhere_fails() {
    let src = "pub f(n: int) -> int {\n    var s = 0\n    @prove(no_checks)\n    for i from 0 to n {\n        s = s ^ i\n    }\n    s\n}\npub g() {}";
    let mut m = optimized(src);
    let a = remove_checks(&mut m, Arith::Checked);
    assert_eq!(check(&m, &a), [], "it holds where it is");
    // As if lowering had moved the loop without saying where, or never said.
    for lost in [FuncIdx(idx(&m, "g") as u32), FuncIdx(u32::MAX)] {
        let mut m = m.clone();
        let ProveTarget::Loop { func, .. } = &mut m.contracts[0].target else { panic!("a loop site") };
        *func = lost;
        let errors: Vec<String> = check(&m, &a).into_iter().map(|e| e.message).collect();
        assert_eq!(errors, ["`@prove` lost track of the loop at line 4; this is a compiler bug"], "{lost:?}");
    }
}
