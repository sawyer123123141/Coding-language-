//! Range analysis of loops: fixed points with widening, `break` and `continue` edges, nested loops
//! decided in their final pass, and the limits that make a function give up.

mod range_common;

use kestrel_kir::range::Arith::Checked;
use kestrel_kir::range::CheckKind::{Division, Overflow};
use range_common::*;

#[test]
fn a_counter_below_a_guard_is_proven() {
    let (m, a) = analysed("pure f(n: int) -> int {\n var i = 0\n while i < n {\n  i += 1\n }\n i\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (1, 0));
}

#[test]
fn an_unguarded_counter_keeps_its_check() {
    let src = "pure f() -> int {\n var i = 0\n while true {\n  i += 1\n  if i > 9 {\n   break\n  }\n }\n i\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(counts(&m, &a, "f"), (0, 1));
    let left = &checks(&m, &a, "f").left;
    assert_eq!(left[0].loops.iter().map(|&l| line(l)).collect::<Vec<_>>(), [3], "the loop's line");
    assert_eq!(line(left[0].line), 4);
}

#[test]
fn a_counted_loop_bounds_its_variable() {
    let (m, a) = analysed("pure f() -> int {\n var acc = 0\n for i from 0 to 100 {\n  acc = i * 1000\n }\n acc\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (1, 0));
    // Accumulating grows without a bound the analysis can see.
    let (m, a) = analysed("pure f() -> int {\n var acc = 0\n for i from 0 to 100 {\n  acc += i\n }\n acc\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (0, 1));
}

#[test]
fn an_empty_loop_proves_nothing_wrong() {
    // The body never runs, so its multiply (which would overflow) is unreachable: it keeps its check.
    let (m, a) = analysed("pure f() -> int {\n var x = 0\n for i from 5 to 0 {\n  x = i * 1000000000\n }\n x\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (0, 1));
}

#[test]
fn a_count_down_stays_in_range() {
    let (m, a) = analysed("pure f(n: int) -> int {\n var i = n\n while i > 0 {\n  i -= 1\n }\n i\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (1, 0));
}

#[test]
fn continue_before_the_step_is_joined() {
    // The `continue` path skips `i += 1`; it joins the top with `i` below `n` all the same.
    let src = "pure f(n: int, c: bool) -> int {\n var i = 0\n while i < n {\n  if c {\n   continue\n  }\n  i += 1\n }\n i\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(counts(&m, &a, "f"), (1, 0));
}

/// `range_keep_continue.kes`'s `pick`: the `for` loop's `continue` goes through the step.
pub const PICK: &str = "pure pick(n: int, k: int) -> int {
    var j = 0
    if n > 0 {
        for i from 0 to n {
            if i == k {
                j = 1
                break
            }
            continue
        }
        return 100 / j
    }
    0
}";

#[test]
fn a_for_continue_takes_the_step() {
    // If `continue` skipped the step, the loop would seem never to end normally, `j` would seem
    // always 1 after it, and the division would lose its check.
    let (m, a) = analysed(PICK, Checked);
    assert_eq!(left_of(&m, &a, "pick", Division).len(), 1);
    assert_eq!(proven_of(&m, "pick", Division), 0);
}

#[test]
fn nested_loops_decide_in_the_final_pass() {
    // In the outer loop's first round `k` is 0, which would prove the multiply; only the final
    // pass (with `k` 0 or more) decides it.
    let src = "pure f(n: int) -> int {\n var x = 0\n var k = 0\n while k < n {\n  for i from 0 to 10 {\n   x = k * 100000000\n  }\n  k += 1\n }\n x\n}";
    let (m, a) = analysed(src, Checked);
    let left = left_of(&m, &a, "f", Overflow);
    assert_eq!(left.len(), 1, "{left:#?}");
    assert_eq!(line(left[0].line), 6);
    assert_eq!(left[0].loops.iter().map(|&l| line(l)).collect::<Vec<_>>(), [4, 5], "outermost first");
    assert_eq!(proven_of(&m, "f", Overflow), 1, "`k += 1` under `k < n`");
}

#[test]
fn the_budget_gives_up_cleanly() {
    // 40 nested loops: each level repeats its body a few times, so the visits multiply.
    let depth = 40;
    let mut src = String::from("pure f(n: int) -> int {\n var t = 0\n");
    for d in 0..depth {
        src.push_str(&format!("var c{d} = 0\nwhile c{d} < n {{\n"));
    }
    src.push_str("t = t + 1\n");
    for d in (0..depth).rev() {
        src.push_str(&format!("c{d} += 1\n}}\n"));
    }
    src.push_str("t\n}");
    let (m, a) = analysed(&src, Checked);
    let c = checks(&m, &a, "f");
    assert!(c.gave_up);
    assert_eq!(c.removed, 0);
    assert_eq!(c.left.len(), depth + 1, "every check stays");
    assert!(proven_kinds(&m, "f").is_empty(), "nothing is marked");
}

#[test]
fn deep_nesting_gives_up_without_overflowing_the_stack() {
    let depth = 300;
    let mut src = String::from("pure f(x: int) -> int {\n var y = 0\n");
    for d in 0..depth {
        src.push_str(&format!("if x > {d} {{\n"));
    }
    src.push_str("y = x * 2\n");
    for _ in 0..depth {
        src.push_str("}\n");
    }
    src.push_str("y\n}");
    // Building the module needs a big stack; the analysis runs on a thread with 2 MB, like the
    // Godot add-on's.
    let m = std::thread::Builder::new().stack_size(64 << 20).spawn(move || optimized(&src)).unwrap().join().unwrap();
    let (m, a) = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            let mut m = m;
            let a = kestrel_kir::range::remove_checks(&mut m, Checked);
            (m, a)
        })
        .unwrap()
        .join()
        .unwrap();
    let c = checks(&m, &a, "f");
    assert!(c.gave_up);
    assert_eq!(c.left.len(), 1, "the multiply stays");
    // Dropping the deep module needs a big stack too.
    std::thread::Builder::new().stack_size(64 << 20).spawn(move || drop(m)).unwrap().join().unwrap();
}

#[test]
fn deep_expressions_give_up_without_overflowing_the_stack() {
    // 300 nested operands and 300 nested value `if`s, analysed on a 2 MB stack.
    let depth = 300;
    let sum = format!("{}x{}", "(x + ".repeat(depth), ")".repeat(depth));
    let ifs = format!("{}x{}", "if x > 0 { ".repeat(depth), " } else { 0 }".repeat(depth));
    for body in [sum, ifs] {
        let src = format!("pure f(x: int) -> int {{\n let y = {body}\n y * 2\n}}");
        let m = std::thread::Builder::new().stack_size(64 << 20).spawn(move || optimized(&src)).unwrap().join().unwrap();
        let (m, a) = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || {
                let mut m = m;
                let a = kestrel_kir::range::remove_checks(&mut m, Checked);
                (m, a)
            })
            .unwrap()
            .join()
            .unwrap();
        let c = checks(&m, &a, "f");
        assert!(c.gave_up);
        assert_eq!(c.removed, 0);
        assert!(proven_kinds(&m, "f").is_empty(), "nothing is marked");
        std::thread::Builder::new().stack_size(64 << 20).spawn(move || drop(m)).unwrap().join().unwrap();
    }
}
