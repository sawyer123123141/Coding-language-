//! `@assume` in range analysis: inside its block it teaches what `if cond` would, and nothing
//! outside its block sees it. Each proof has a near miss that keeps its check. (What `--release`
//! trusts is in `range_assume_release.rs`.)

mod range_common;

use kestrel_kir::range::Arith::{Checked, Wrapping};
use kestrel_kir::range::CheckKind::{Bounds, Division, Overflow};
use kestrel_kir::range::{Analysis, Arith};
use kestrel_kir::Module;
use range_common::*;

/// The lines of the bounds checks left in `name`, and how many were proven.
fn bounds_in(m: &Module, a: &Analysis, name: &str) -> (Vec<u32>, usize) {
    let left = left_of(m, a, name, Bounds).iter().map(|c| line(c.line)).collect();
    (left, proven_of(m, name, Bounds))
}

/// `f`'s bounds checks in `arith`.
fn bounds(src: &str, arith: Arith) -> (Vec<u32>, usize) {
    let (m, a) = analysed(src, arith);
    bounds_in(&m, &a, "f")
}

/// `(removed, bounds)` of each use recorded in `name`.
fn removed(m: &Module, a: &Analysis, name: &str) -> Vec<(u32, bool)> {
    uses(m, a, name).iter().map(|u| (u.removed, u.bounds)).collect()
}

pub const GET: &str = "pure get(xs: [int], i: int) -> int {\n @assume(i >= 0 and i < xs.len)\n xs[i]\n}";

#[test]
fn an_assumption_proves_its_bounds_check() {
    for arith in [Checked, Wrapping] {
        let (m, a) = analysed(GET, arith);
        assert_eq!(bounds_in(&m, &a, "get"), (vec![], 1), "{arith:?}");
        assert_eq!(removed(&m, &a, "get"), [(1, true)], "{arith:?}");
        assert_eq!(uses(&m, &a, "get")[0].ids, [0]);
    }
}

#[test]
fn without_the_lower_end_it_does_not() {
    let (m, a) = analysed("pure f(xs: [int], i: int) -> int {\n @assume(i < xs.len)\n xs[i]\n}", Checked);
    assert_eq!(bounds_in(&m, &a, "f"), (vec![3], 0));
    assert_eq!(removed(&m, &a, "f"), [(0, false)]);
}

#[test]
fn another_list_proves_nothing() {
    let (m, a) = analysed("pure f(xs: [int], ys: [int], i: int) -> int {\n @assume(i >= 0 and i < ys.len)\n xs[i]\n}", Checked);
    assert_eq!(bounds_in(&m, &a, "f"), (vec![3], 0));
    assert_eq!(removed(&m, &a, "f"), [(0, false)]);
}

#[test]
fn a_change_ends_it() {
    let src = "pure f(xs: [int], i: int, j: int) -> int {\n var k = i\n @assume(k >= 0 and k < xs.len)\n k = j\n xs[k]\n}";
    assert_eq!(bounds(src, Checked), (vec![5], 0));
    let src = "f(var xs: list[int], i: int) -> int {\n @assume(i >= 0 and i < xs.len)\n xs.pop()\n xs[i]\n}";
    assert_eq!(bounds(src, Checked), (vec![4], 0));
    // The near miss: without the change, the same index is proven.
    let src = "f(var xs: list[int], i: int) -> int {\n @assume(i >= 0 and i < xs.len)\n xs[i]\n}";
    assert_eq!(bounds(src, Checked), (vec![], 1));
}

#[test]
fn it_ends_at_its_block() {
    let src = "pure f(xs: [int], i: int, c: bool) -> int {\n var s = 0\n if c {\n  @assume(i >= 0 and i < xs.len)\n  s = xs[i]\n }\n s + xs[i]\n}";
    for arith in [Checked, Wrapping] {
        assert_eq!(bounds(src, arith), (vec![7], 1), "{arith:?}");
    }
    // With no other way around the block, its end alone leaves it. (Inside, `xs[0]` is proven;
    // after its check, which the plain run learns from, `xs` isn't empty, but nothing says `i` is
    // inside it. A passed `xs[i]` inside would teach that, and soundly: an assumption that removes
    // a bounds check is always evaluated.)
    let src = "pure f(xs: [int], i: int) -> int {\n var s = 0\n arena {\n  @assume(i >= 0 and i < xs.len)\n  s = xs[0]\n }\n s + xs[i]\n}";
    for arith in [Checked, Wrapping] {
        assert_eq!(bounds(src, arith), (vec![7], 1), "{arith:?}");
    }
}

#[test]
fn a_break_leaves_it_behind() {
    let src = "pure f(xs: [int], i: int, c: bool) -> int {\n var s = 0\n while true {\n  @assume(i >= 0 and i < xs.len)\n  if c {\n   break\n  }\n  s = xs[i]\n  break\n }\n s + xs[i]\n}";
    for arith in [Checked, Wrapping] {
        assert_eq!(bounds(src, arith), (vec![11], 1), "{arith:?}");
    }
    // With one `break`, the only way out of the loop.
    let src = "pure f(xs: [int], i: int) -> int {\n var s = 0\n while true {\n  @assume(i >= 0 and i < xs.len)\n  s = xs[0]\n  break\n }\n s + xs[i]\n}";
    for arith in [Checked, Wrapping] {
        assert_eq!(bounds(src, arith), (vec![8], 1), "{arith:?}");
    }
}

#[test]
fn a_continue_leaves_it_behind() {
    let src = "pure f(xs: [int], i: int, n: int) -> int {\n var s = 0\n var k = 0\n while k < n {\n  k += 1\n  if k > 5 {\n   @assume(i >= 0 and i < xs.len)\n   s = s ^ xs[i]\n   continue\n  }\n  s = s ^ xs[i]\n }\n s\n}";
    // The `continue` carries the plain run's state: the next iteration's `xs[i]` keeps its check.
    assert_eq!(bounds(src, Checked), (vec![11], 1));
}

#[test]
fn the_next_iteration_starts_without_it() {
    let src = "pure f(xs: [int], n: int) -> int {\n var s = 0\n if xs.len > 0 {\n  var i = 0\n  while i < n {\n   s = s ^ xs[i]\n   @assume(i + 1 < xs.len)\n   i += 1\n  }\n }\n s\n}";
    for arith in [Checked, Wrapping] {
        let (m, a) = analysed(src, arith);
        assert_eq!(bounds_in(&m, &a, "f"), (vec![6], 0), "{arith:?}");
        assert_eq!(removed(&m, &a, "f"), [(0, false)], "{arith:?}");
    }
}

#[test]
fn facts_from_before_the_block_survive_it() {
    let src = "pure f(xs: [int], n: int, c: bool) -> int {\n var s = 0\n for i from 0 to xs.len {\n  if c {\n   @assume(n >= 0 and n < xs.len)\n   s = s ^ xs[n]\n  }\n  s = s ^ xs[i]\n }\n s\n}";
    for arith in [Checked, Wrapping] {
        assert_eq!(bounds(src, arith), (vec![], 2), "{arith:?}");
    }
}

#[test]
fn a_contradiction_proves_nothing() {
    let src = "pure f(xs: [int], i: int) -> int {\n var s = 0\n if i < 0 {\n  @assume(i > 5)\n  s = xs[i]\n }\n s\n}";
    for arith in [Checked, Wrapping] {
        let (m, a) = analysed(src, arith);
        assert_eq!(bounds_in(&m, &a, "f"), (vec![5], 0), "{arith:?}");
        assert_eq!(removed(&m, &a, "f"), [(0, false)], "{arith:?}: a use that removed nothing (P4)");
    }
}

#[test]
fn an_assumption_removes_an_overflow_check() {
    let src = "pure f(a: int, b: int) -> int {\n @assume(a >= 0 and a <= 1000 and b >= 0 and b <= 1000)\n a * b\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!((proven_of(&m, "f", Overflow), left_of(&m, &a, "f", Overflow).len()), (1, 0));
    assert_eq!(removed(&m, &a, "f"), [(1, false)]);
    let (m, a) = analysed(src, Wrapping);
    assert_eq!(counts(&m, &a, "f"), (0, 0));
    assert_eq!(removed(&m, &a, "f"), [(0, false)]);
}

#[test]
fn a_conditions_own_check_is_left_and_the_block_after_keeps_its_own() {
    // A pin of today's behaviour, not a test of what a condition teaches: Round A learns nothing
    // from a passed overflow check, so `n + 1` after the block keeps its check whatever the plain
    // run does. (The test that a check in a condition teaches the plain run nothing is
    // `a_check_in_a_condition_teaches_the_plain_run_nothing`, with a division, which does teach.)
    // The condition's own `n + 1` is decided outside both runs, as the enclosing code's.
    let src = "pure f(n: int) -> int {\n var r = 0\n arena {\n  @assume(n + 1 > 5)\n  r = n\n }\n let k = n + 1\n r ^ k\n}";
    let (m, a) = analysed(src, Checked);
    let left: Vec<u32> = left_of(&m, &a, "f", Overflow).iter().map(|c| line(c.line)).collect();
    assert_eq!(left, [4, 7]);
}

/// `f` from `src` with its assumption's condition `b > -1` made `7 / b > -1` by hand: a source
/// condition can't divide, so no check in one teaches a fact today, but the analysis mustn't rely
/// on that.
fn with_divided_condition(src: &str, arith: Arith) -> (Module, Analysis) {
    use kestrel_kir::visit_mut::{walk_stmt, VisitMut};
    use kestrel_kir::{BinOp, Expr, ExprKind, Stmt};
    struct Divide;
    impl VisitMut for Divide {
        fn visit_stmt(&mut self, s: &mut Stmt) {
            if let Stmt::Assume { conds, .. } = s {
                if let ExprKind::Binary(_, l, _) = &mut conds[0].1.kind {
                    let (ty, line) = (l.ty, l.line);
                    let b = std::mem::replace(&mut **l, Expr { kind: ExprKind::Int(0), ty, line });
                    let seven = Expr { kind: ExprKind::Int(7), ty, line };
                    **l = Expr { kind: ExprKind::Binary(BinOp::Div, Box::new(seven), Box::new(b)), ty, line };
                }
            }
            walk_stmt(self, s);
        }
    }
    let mut m = optimized(src);
    let i = idx(&m, "f");
    Divide.visit_stmts(&mut m.funcs[i].body);
    let a = kestrel_kir::range::remove_checks(&mut m, arith);
    (m, a)
}

#[test]
fn a_check_in_a_condition_teaches_the_plain_run_nothing() {
    // `b` is 0 to 255: the condition's `7 / b` passing would say it isn't 0. That holds only while
    // the condition runs, so `100 / b` after the block keeps its check.
    let src = "pure f(a: int, c: int) -> int {\n let b = c & 255\n var r = 0\n arena {\n  @assume(b > -1)\n  r = a\n }\n r + 100 / b\n}";
    for arith in [Checked, Wrapping] {
        let (m, a) = with_divided_condition(src, arith);
        let left: Vec<u32> = left_of(&m, &a, "f", Division).iter().map(|c| line(c.line)).collect();
        assert_eq!(left, [5, 8], "{arith:?}");
    }
    // Inside the block, a checked build (which always evaluates the condition) may use it; under
    // `--release`, where the assumption may be trusted and not evaluated, the real run doesn't.
    let src = src.replace("r = a\n", "r = a / b\n");
    let (m, a) = with_divided_condition(&src, Checked);
    assert_eq!(left_of(&m, &a, "f", Division).iter().map(|c| line(c.line)).collect::<Vec<_>>(), [5]);
    let (m, a) = with_divided_condition(&src, Wrapping);
    assert_eq!(left_of(&m, &a, "f", Division).iter().map(|c| line(c.line)).collect::<Vec<_>>(), [5, 6]);
}

#[test]
fn a_condition_that_writes_is_forgotten() {
    // By hand again (a source condition can't call): `next(var k, 1) > -1 and k >= 0 and
    // k < xs.len`. The condition rule forgets what a condition writes once it has refined, so
    // nothing is known of `k` in the block: a trusted copy would never run the write.
    use kestrel_kir::visit_mut::{walk_stmt, VisitMut};
    use kestrel_kir::{BinOp, Expr, ExprKind, KTy, Stmt};
    let src = "next(var x: int, d: int) -> int {\n if d == 0 {\n  x = x + 1\n  return 0\n }\n next(var x, d - 1)\n}\nf(xs: [int], i: int) -> int {\n var k = i\n let w = next(var k, 1)\n @assume(k >= 0 and k < xs.len)\n xs[k] + w\n}";
    struct Write(Option<Expr>);
    impl VisitMut for Write {
        fn visit_stmt(&mut self, s: &mut Stmt) {
            match s {
                Stmt::Let(_, e) if self.0.is_none() && matches!(e.kind, ExprKind::Call(..)) => self.0 = Some(e.clone()),
                Stmt::Assume { conds, .. } => {
                    let call = self.0.clone().expect("the call comes first");
                    let (ty, line) = (conds[0].1.ty, conds[0].1.line);
                    let minus_one = Expr { kind: ExprKind::Int(-1), ty: call.ty, line };
                    let writes = Expr { kind: ExprKind::Binary(BinOp::Gt, Box::new(call), Box::new(minus_one)), ty, line };
                    let old = std::mem::replace(&mut conds[0].1, Expr { kind: ExprKind::Bool(true), ty: KTy::Bool, line });
                    conds[0].1 = Expr { kind: ExprKind::And(Box::new(writes), Box::new(old)), ty, line };
                }
                _ => {}
            }
            walk_stmt(self, s);
        }
    }
    let written = |src: &str, arith: Arith| {
        let mut m = optimized(src);
        let i = idx(&m, "f");
        Write(None).visit_stmts(&mut m.funcs[i].body);
        let a = kestrel_kir::range::remove_checks(&mut m, arith);
        bounds_in(&m, &a, "f")
    };
    for arith in [Checked, Wrapping] {
        assert_eq!(bounds(src, arith), (vec![], 1), "{arith:?}: as written, the assumption proves `xs[k]`");
        assert_eq!(written(src, arith), (vec![12], 0), "{arith:?}");
    }
    // After the block, which the plain run's end leaves: the `if` showed `k` inside `xs`, but the
    // condition's write (run when the assumption is checked) may have moved it, so `xs[k]` on line
    // 17 keeps its check. The plain run must forget the write too, as the condition rule does.
    let after = "next(var x: int, d: int) -> int {\n if d == 0 {\n  x = x + 1\n  return 0\n }\n next(var x, d - 1)\n}\nf(xs: [int], i: int) -> int {\n var k = i\n let w = next(var k, 1)\n var s = w\n if k >= 0 and k < xs.len {\n  arena {\n   @assume(k >= 0 and k < xs.len)\n   s = s + 1\n  }\n  s = s + xs[k]\n }\n s\n}";
    for arith in [Checked, Wrapping] {
        assert_eq!(bounds(after, arith), (vec![], 1), "{arith:?}: as written, the `if` proves `xs[k]`");
        assert_eq!(written(after, arith), (vec![17], 0), "{arith:?}");
    }
}

#[test]
fn an_inlined_assumption_counts_in_its_copy() {
    let src = format!("{GET}\npure h(xs: [int], k: int) -> int {{\n get(xs, k) ^ 1\n}}");
    for arith in [Checked, Wrapping] {
        let (m, a) = analysed(&src, arith);
        assert_eq!(bounds_in(&m, &a, "h"), (vec![], 1), "{arith:?}");
        assert_eq!(removed(&m, &a, "get"), [(1, true)], "{arith:?}");
        assert_eq!(removed(&m, &a, "h"), [(1, true)], "{arith:?}");
        assert_eq!(uses(&m, &a, "h")[0].ids, [0], "the copy carries the source's ids");
    }
}

#[test]
fn an_assumption_around_a_nested_loop() {
    let src = "pure f(xs: [int], n: int, m: int) -> int {\n var s = 0\n for k from 0 to m {\n  @assume(n >= 0 and n <= xs.len)\n  for i from 0 to n {\n   s = s ^ xs[i]\n  }\n }\n s\n}";
    for arith in [Checked, Wrapping] {
        let (m, a) = analysed(src, arith);
        assert_eq!(bounds_in(&m, &a, "f"), (vec![], 1), "{arith:?}");
        assert_eq!(removed(&m, &a, "f"), [(1, true)], "{arith:?}");
    }
    // The near miss: `n <= xs.len + 1` lets `i` reach `xs.len`.
    let src = src.replace("n <= xs.len", "n <= xs.len + 1");
    assert_eq!(bounds(&src, Checked), (vec![6], 0));
}

#[test]
fn a_length_assumption_proves_a_neighbour() {
    let src = "pure f(xs: [u8], i: int) -> u8 {\n @assume(xs.len == 100 and i >= 0 and i < 99)\n xs[i + 1]\n}";
    for arith in [Checked, Wrapping] {
        assert_eq!(bounds(src, arith), (vec![], 1), "{arith:?}");
    }
    assert_eq!(bounds(&src.replace("i < 99", "i < 100"), Checked), (vec![3], 0));
}

#[test]
fn a_field_in_memory_teaches_nothing() {
    let src = "struct Grid {\n n: int\n cells: [int]\n}\npure f(g: Grid, i: int) -> int {\n @assume(i >= 0 and i < g.n and g.n <= g.cells.len)\n g.cells[i]\n}";
    assert_eq!(bounds(src, Checked), (vec![7], 0));
}

#[test]
fn a_division_assumption_proves_a_division() {
    let src = "pure d(a: int, b: int) -> int {\n @assume(b > 0)\n a / b\n}";
    for arith in [Checked, Wrapping] {
        let (m, a) = analysed(src, arith);
        assert_eq!((proven_of(&m, "d", Division), left_of(&m, &a, "d", Division).len()), (1, 0), "{arith:?}");
        assert_eq!(removed(&m, &a, "d"), [(1, false)], "{arith:?}");
    }
    let (m, a) = analysed(&src.replace("b > 0", "b > -1"), Checked);
    assert_eq!(left_of(&m, &a, "d", Division).len(), 1);
}
