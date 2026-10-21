//! Regions: each check range analysis leaves and each event of the report's walk carries the ids
//! of the `Stmt::Prove` blocks around it, so a contract can find what is in its code. (The sites
//! themselves: `contracts_sites.rs`.)

mod range_common;

use kestrel_kir::range::events::{events, What};
use kestrel_kir::range::{mark_proven, remove_checks, Analysis, Arith, CheckLeft};
use kestrel_kir::*;
use range_common::{checks, idx, line, optimized};

/// The loop's header (`n * 2`) and body (`s + xs[i]`, `xs[i]`, `ys.add(i)`) are in the region;
/// line 8 and 9 aren't.
const F: &str = "f(xs: list[int], var ys: list[int], n: int) -> int {
    var s = 0
    @prove(no_checks)
    for i from 0 to n * 2 {
        s = s + xs[i]
        ys.add(i)
    }
    ys.add(n)
    s + xs[n]
}";

/// (line, regions) of each check left in `f`.
fn regions_left(m: &Module, a: &Analysis) -> Vec<(u32, Vec<u32>)> {
    checks(m, a, "f").left.iter().map(|c: &CheckLeft| (line(c.line), c.regions.clone())).collect()
}

#[test]
fn checks_and_events_know_their_region() {
    let run: [fn(&mut Module, Arith) -> Analysis; 2] = [remove_checks, mark_proven];
    for analyse in run {
        let mut m = optimized(F);
        let a = analyse(&mut m, Arith::Checked);
        let left = regions_left(&m, &a);
        for line in [4, 5, 9] {
            assert!(left.iter().any(|(l, _)| *l == line), "a check left on line {line}: {left:?}");
        }
        for (l, regions) in &left {
            let want: &[u32] = if *l <= 7 { &[0] } else { &[] };
            assert_eq!(regions, want, "line {l}: {left:?}");
        }
    }
    let m = optimized(F);
    let allocs: Vec<(u32, Vec<u32>)> = events(&m, &m.funcs[idx(&m, "f")])
        .into_iter()
        .filter(|e| matches!(e.what, What::Alloc(_)))
        .map(|e| (line(e.line), e.regions))
        .collect();
    assert_eq!(allocs, [(6, vec![0]), (8, vec![])]);
}

#[test]
fn a_function_given_up_on_keeps_its_regions() {
    // The analysis gives `g` up (300 nested `if`s), so every site is listed as it is.
    let mut src = String::from("g(xs: list[int], x: int) -> int {\n    var s = 0\n    @prove(no_checks)\n    for i from 0 to x { s = s + xs[i] }\n");
    for d in 0..300 {
        src.push_str(&format!("if x > {d} {{\n"));
    }
    src.push_str("s = s + 1\n");
    for _ in 0..300 {
        src.push_str("}\n");
    }
    src.push_str("s\n}");
    let big = |f: Box<dyn FnOnce() + Send>| std::thread::Builder::new().stack_size(64 << 20).spawn(f).unwrap().join().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    big(Box::new(move || {
        let mut m = optimized(&src);
        let a = remove_checks(&mut m, Arith::Checked);
        let c = checks(&m, &a, "g");
        let left: Vec<(u32, Vec<u32>)> = c.left.iter().map(|c| (line(c.line), c.regions.clone())).collect();
        tx.send((c.gave_up, left)).unwrap();
    }));
    let (gave_up, left) = rx.recv().unwrap();
    assert!(gave_up);
    assert!(left.iter().filter(|(l, _)| *l == 4).count() >= 2, "{left:?}");
    for (l, regions) in &left {
        let want: &[u32] = if *l == 4 { &[0] } else { &[] };
        assert_eq!(regions, want, "line {l}");
    }
}

#[test]
fn the_events_walk_says_where_it_stopped() {
    let big = |f: Box<dyn FnOnce() + Send>| std::thread::Builder::new().stack_size(256 << 20).spawn(f).unwrap().join().unwrap();
    big(Box::new(|| {
        let mut m = optimized("f() {}");
        let truth = Expr { kind: ExprKind::Bool(true), ty: KTy::Bool, line: 0 };
        let mut body = vec![Stmt::Trap];
        for _ in 0..5000 {
            body = vec![Stmt::If { cond: truth.clone(), then_body: body, else_body: Vec::new() }];
        }
        let f = idx(&m, "f");
        m.funcs[f].body = vec![Stmt::Prove { id: 7, at: 0, body }];
        let found = events(&m, &m.funcs[f]);
        let last = found.last().expect("an event");
        assert_eq!((&last.what, &last.regions[..]), (&What::Cut, &[7][..]));
        assert_eq!(found.iter().filter(|e| e.what == What::Cut).count(), 1);
        // The walk stopped well inside: nothing after the cut.
        assert!(found.len() <= 2, "{found:?}");
    }));
}
