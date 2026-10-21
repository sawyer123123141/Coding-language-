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

/// The site ids of `regions`, outermost first.
fn ids(regions: &[Region]) -> Vec<u32> {
    regions.iter().map(|r| r.id).collect()
}

/// (line, site ids of its regions) of each check left in `name`.
fn regions_left(m: &Module, a: &Analysis, name: &str) -> Vec<(u32, Vec<u32>)> {
    checks(m, a, name).left.iter().map(|c: &CheckLeft| (line(c.line), ids(&c.regions))).collect()
}

#[test]
fn checks_and_events_know_their_region() {
    let run: [fn(&mut Module, Arith) -> Analysis; 2] = [remove_checks, mark_proven];
    for analyse in run {
        let mut m = optimized(F);
        let a = analyse(&mut m, Arith::Checked);
        let left = regions_left(&m, &a, "f");
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
        .map(|e| (line(e.line), ids(&e.regions)))
        .collect();
    assert_eq!(allocs, [(6, vec![0]), (8, vec![])]);
}

#[test]
fn a_function_given_up_on_keeps_its_regions() {
    // The analysis gives `g` up (300 nested `if`s), so every site is listed as it is, with the
    // copy of each loop's block (line 4: copy 0, line 5: copy 1).
    let mut src = String::from(
        "g(xs: list[int], x: int) -> int {\n    var s = 0\n    @prove(no_checks)\n    for i from 0 to x { s = s + xs[i] }\n    @prove(no_alloc) for i from 0 to x { s = s + xs[i] }\n",
    );
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
        let left: Vec<(u32, Vec<(u32, u32)>)> = c.left.iter().map(|c| (line(c.line), c.regions.iter().map(|r| (r.id, r.copy)).collect())).collect();
        tx.send((c.gave_up, left)).unwrap();
    }));
    let (gave_up, left) = rx.recv().unwrap();
    assert!(gave_up);
    for l in [4, 5] {
        assert!(left.iter().filter(|(at, _)| *at == l).count() >= 2, "{left:?}");
    }
    for (l, regions) in &left {
        let want: &[(u32, u32)] = match l {
            4 => &[(0, 0)],
            5 => &[(1, 1)],
            _ => &[],
        };
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
        assert_eq!((&last.what, &last.regions[..]), (&What::Cut, &[Region { id: 7, at: 0, copy: 0 }][..]));
        assert_eq!(found.iter().filter(|e| e.what == What::Cut).count(), 1);
        // The walk stopped well inside: nothing after the cut.
        assert!(found.len() <= 2, "{found:?}");
    }));
}

/// The `Prove` blocks in `stmts` as (id, line of `at`), in walk order: block `copy` is the `copy`th.
fn blocks(stmts: &[Stmt]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    visit::walk(stmts, &mut |n| {
        if let visit::Node::Stmt(Stmt::Prove { id, at, .. }) = n {
            out.push((*id, line(*at)));
        }
    });
    out
}

#[test]
fn two_copies_of_one_callee_are_told_apart() {
    // `m` inlined twice into `step`: on line 40 the multiply is proven (`x & 255`), on line 50 it
    // isn't. The check left is the line-50 copy's.
    let mut src = String::from("@prove(no_checks)\npure m(a: int) -> int { a * 1000 }\n");
    src.push_str(&"\n".repeat(36));
    src.push_str("pure step(x: int) -> int {\n    let p = m(x & 255)\n");
    src.push_str(&"\n".repeat(9));
    src.push_str("    let q = m(x)\n    p ^ q\n}");
    let run: [fn(&mut Module, Arith) -> Analysis; 2] = [remove_checks, mark_proven];
    for analyse in run {
        let mut m = optimized(&src);
        let step = idx(&m, "step");
        assert_eq!(blocks(&m.funcs[step].body), [(0, 40), (0, 50)]);
        let a = analyse(&mut m, Arith::Checked);
        let left = &checks(&m, &a, "step").left;
        let [c] = &left[..] else { panic!("{left:?}") };
        let [r] = c.regions[..] else { panic!("{c:?}") };
        assert_eq!((r.id, line(r.at), r.copy), (0, 50, 1), "{c:?}");
        assert_eq!(line(c.line), 2, "the multiply is on `m`'s line");
    }
    // The events walk numbers the copies the same way.
    let m = optimized("@prove(no_alloc)
pure mk(a: int) -> int { [a, a].len }
pure step(x: int) -> int {
    let p = mk(x)
    p + mk(1)
}");
    let found: Vec<(u32, u32, u32)> = events(&m, &m.funcs[idx(&m, "step")])
        .iter()
        .filter(|e| matches!(e.what, What::Alloc(_)))
        .map(|e| {
            let [r] = e.regions[..] else { panic!("{e:?}") };
            (r.id, line(r.at), r.copy)
        })
        .collect();
    assert_eq!(found, [(0, 4, 0), (0, 5, 1)]);
}

#[test]
fn a_copy_inside_a_copy_has_both_regions() {
    let src = "@prove(no_checks)\npure inner(a: int) -> int { a * 1000 }\n@prove(no_alloc)\npure outer(a: int) -> int { inner(a) + 1 }\npure top(x: int) -> int {\n    outer(x)\n}";
    let mut m = optimized(src);
    let top = idx(&m, "top");
    // `outer`'s copy (site 1, at line 6) holds `inner`'s (site 0, at `outer`'s call, line 4).
    assert_eq!(blocks(&m.funcs[top].body), [(1, 6), (0, 4)]);
    let a = remove_checks(&mut m, Arith::Checked);
    // (line, each region's (id, line of `at`, copy))
    type Left = (u32, Vec<(u32, u32, u32)>);
    let left: Vec<Left> =
        checks(&m, &a, "top").left.iter().map(|c| (line(c.line), c.regions.iter().map(|r| (r.id, line(r.at), r.copy)).collect())).collect();
    // The multiply is in both copies, outer first; the `+ 1` only in `outer`'s.
    assert_eq!(left, [(2, vec![(1, 6, 0), (0, 4, 1)]), (4, vec![(1, 6, 0)])]);
}

#[test]
fn a_copy_in_a_par_for_body_or_a_closure_is_a_region_there() {
    let src = "@prove(no_checks)\npure m(a: int) -> int { a * 1000 }\n\nf(var ps: list[int]) {\n    par for p in ps { p = m(p) }\n}\n\ng(n: int) -> int {\n    let h: (int) -> int = (k) { m(k) }\n    h(n)\n}";
    let mut m = optimized(src);
    let par = m.par_bodies[0].0 as usize;
    let code = m.funcs.iter().position(|f| f.name.starts_with("__kestrel_closure_")).expect("the closure's code");
    assert_eq!(blocks(&m.funcs[par].body), [(0, 5)]);
    assert_eq!(blocks(&m.funcs[code].body), [(0, 9)]);
    let a = remove_checks(&mut m, Arith::Checked);
    for f in [par, code] {
        let left: Vec<Vec<u32>> = a.funcs[f].left.iter().map(|c| ids(&c.regions)).collect();
        assert_eq!(left, [vec![0]], "`{}`", m.funcs[f].name);
    }
}
