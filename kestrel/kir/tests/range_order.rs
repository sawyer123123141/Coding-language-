//! The spec's statement rule ("One statement at a time"): a fact from a passed check is about the
//! value the check saw, never about one a write in the same statement or condition replaced, and
//! facts never pass between sibling operands or out of an `if` arm.

mod range_common;

use kestrel_kir::range::Arith::{Checked, Wrapping};
use kestrel_kir::range::CheckKind::{Bounds, Narrowing, Overflow};
use range_common::*;

/// `bump(var x, 1)`: triples `x` and returns 1; recursive with an early `return`, so it is never
/// inlined.
const BUMP: &str = "bump(var x: int, d: int) -> int {
    if d == 0 {
        x = x * 3
        return 1
    }
    bump(var x, d - 1)
}";

#[test]
fn a_narrowing_fact_is_about_the_old_value() {
    // `int(x)` passing says the old `x` fits an `int`; the assignment then replaces `x`.
    let src = "pure f(p: i64) -> int {\n var x = p\n x = i64(int(x)) + x\n int(x)\n}";
    let (m, a) = analysed(src, Checked);
    let left = left_of(&m, &a, "f", Narrowing);
    assert!(left.iter().any(|c| line(c.line) == 4), "the second narrowing stays: {left:#?}");
}

#[test]
fn a_condition_forgets_its_own_writes() {
    // `x < 100` holds when it is compared; `bump` then triples `x`, so `x * 10000000` may overflow.
    let forms = [
        "f(p: int) -> int {\n var x = p\n var y = 0\n if x >= 0 and x < 100 and bump(var x, 1) > 0 {\n  y = x * 10000000\n }\n y\n}",
        "f(p: int) -> int {\n var x = p\n var y = 0\n while x >= 0 and x < 100 and bump(var x, 1) > 0 {\n  y = x * 10000000\n  x = p\n }\n y\n}",
        "f(p: int) -> int {\n var x = p\n let y = if x >= 0 and x < 100 and bump(var x, 1) > 0 {\n  x * 10000000\n } else {\n  0\n }\n y\n}",
    ];
    for src in forms {
        let (m, a) = analysed(&format!("{BUMP}\n{src}"), Checked);
        let left = left_of(&m, &a, "f", Overflow);
        assert!(left.iter().any(|c| line(c.line) >= 5 && c.text.contains("10000000")), "{src}\n{left:#?}");
        assert_eq!(proven_of(&m, "f", Overflow), 0, "{src}");
    }
}

#[test]
fn a_conditions_passed_check_is_forgotten_after_its_write() {
    // `int(x)` passing in the condition says the old `x` fits an `int`; `bump` then triples `x`
    // before the arm runs, so the arm's `int(x)` keeps its check.
    let src = "bump(var x: i64, d: int) -> int {
    if d == 0 {
        x = x * 3
        return 1
    }
    bump(var x, d - 1)
}
f(p: i64) -> int {
    var x = p
    var y = 0
    if int(x) > 0 and bump(var x, 1) > 0 {
        y = int(x)
    }
    y
}";
    let (m, a) = analysed(src, Checked);
    let left = left_of(&m, &a, "f", Narrowing);
    assert!(left.iter().any(|c| line(c.line) == 12), "{left:#?}");
    assert_eq!(proven_of(&m, "f", Narrowing), 0);
}

#[test]
fn a_later_part_of_a_condition_sees_an_earlier_parts_write() {
    // Within one condition, `x < 100` is refined, then `bump` changes `x`, then `x` is multiplied:
    // the multiply must not use the old range.
    let src = format!(
        "{BUMP}\nf(p: int) -> int {{\n var x = p\n var y = 0\n if x >= 0 and x < 100 and bump(var x, 1) > 0 and x * 10000000 > 5 {{\n  y = 1\n }}\n y\n}}"
    );
    let (m, a) = analysed(&src, Checked);
    let left = left_of(&m, &a, "f", Overflow);
    assert_eq!(left.len(), 1, "{left:#?}");
    assert_eq!(proven_of(&m, "f", Overflow), 0);
}

/// `half(x)` (inlined, narrowing `x`) and `bump(var x, 1)` (not inlined) as sibling operands,
/// with `{statement}` on line 13 and `int(x)` after it on line 14.
fn sibling(statement: &str) -> String {
    format!(
        "pure half(x: i64) -> int {{
    int(x) / 2
}}
bump(var x: i64, d: int) -> i64 {{
    if d == 0 {{
        x = x * 3
        return 1
    }}
    bump(var x, d - 1)
}}
f(p: i64) -> int {{
    var x = p
    {statement}
    int(x) + t
}}"
    )
}

#[test]
fn a_sibling_write_wins() {
    // The inlined `half` narrows its own copy of `x` (in its `Seq`'s value), which equals `x`
    // inside the `Seq` (a fact between values) but not after its sibling triples `x`: kept as a
    // regression test of facts leaving an operand. The second form narrows `x` itself, in an
    // operand that runs before its sibling triples it.
    for statement in ["let t = half(x) + int(bump(var x, 1))", "let t = half(x) + int(x) + int(bump(var x, 1))"] {
        let (m, a) = analysed(&sibling(statement), Checked);
        let left = left_of(&m, &a, "f", Narrowing);
        assert!(left.iter().any(|c| line(c.line) == 14), "{statement}: `int(x)` after the call stays: {left:#?}");
    }
}

#[test]
fn each_operand_sees_its_siblings_writes() {
    // `int(x)` on line 10 teaches that `x` fits an `int`; in the next statement `bump` may run
    // before the sibling `int(x)` reads `x`, so that `int(x)` keeps its check.
    let src = "bump(var x: i64, d: int) -> i64 {
    if d == 0 {
        x = x * 3
        return 1
    }
    bump(var x, d - 1)
}
f(p: i64) -> int {
    var x = p
    let a = int(x)
    let t = int(bump(var x, 1)) + int(x)
    a + t
}";
    let (m, a) = analysed(src, Checked);
    let left = left_of(&m, &a, "f", Narrowing);
    assert!(left.iter().any(|c| line(c.line) == 11 && c.text.starts_with("`int(x)`")), "{left:#?}");
    assert_eq!(proven_of(&m, "f", Narrowing), 0);
}

#[test]
fn an_if_arm_teaches_nothing_after_it() {
    let src = "pure f(x: i64, c: bool) -> int {\n var y = 0\n if c {\n  y = int(x)\n }\n int(x) + y\n}";
    let (m, a) = analysed(src, Checked);
    let left = left_of(&m, &a, "f", Narrowing);
    assert_eq!(left.len(), 2, "{left:#?}");
    assert!(left.iter().any(|c| line(c.line) == 6));
}

#[test]
fn a_statement_teaches_the_next_one() {
    // The near miss of the rules above: `int(x)` passing, with nothing writing `x`, proves the
    // next statement's `int(x)`.
    let src = "pure f(x: i64) -> int {\n let a = int(x)\n let b = int(x)\n a ^ b\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(left_of(&m, &a, "f", Narrowing).len(), 1);
    assert_eq!(proven_of(&m, "f", Narrowing), 1);
}

#[test]
fn the_same_statement_does_not_teach_itself() {
    // Two checks in one statement: neither relies on the other.
    let src = "pure f(x: i64) -> int {\n int(x) ^ int(x)\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(left_of(&m, &a, "f", Narrowing).len(), 2);
}

#[test]
fn a_write_inside_a_seq_value_wins() {
    // The inlined `h` sets `x`, then its value reads `x` beside a call that triples it, which may
    // run first (codegen runs it first): the multiply keeps its check.
    let src = format!("{BUMP}\nh(p: int) -> int {{\n var x = 5\n bump(var x, 1) + x * 200000000\n}}\nf(p: int) -> int {{\n h(p)\n}}");
    let (m, a) = analysed(&src, Checked);
    let left = left_of(&m, &a, "f", Overflow);
    assert!(left.iter().any(|c| line(c.line) == 10 && c.text.starts_with("`x * 200000000`")), "{left:#?}");
    assert_eq!(proven_of(&m, "f", Overflow), 0);
}

#[test]
fn a_write_inside_a_comparison_wins() {
    // `x < 100` holds before the comparison; inside it, `bump` may triple `x` before the multiply
    // reads it.
    let src = format!(
        "{BUMP}\nf(p: int) -> int {{\n var x = p\n var y = 0\n if x >= 0 and x < 100 and bump(var x, 1) + x * 10000000 > 5 {{\n  y = 1\n }}\n y\n}}"
    );
    let (m, a) = analysed(&src, Checked);
    let left = left_of(&m, &a, "f", Overflow);
    assert!(left.iter().any(|c| line(c.line) == 11 && c.text.starts_with("`x * 10000000`")), "{left:#?}");
    assert_eq!(proven_of(&m, "f", Overflow), 0);
}

/// `drop0(var xs)` pops and returns 0, `grow0(var xs)` adds and returns 0, `lenof(xs)` is
/// `xs.len` through a local: all small, so inlined, each a `Seq` in the expression.
const CALLS: &str = "drop0(var xs: list[int]) -> int {
    xs.pop()
    0
}
grow0(var xs: list[int]) -> int {
    xs.add(5)
    0
}
lenof(xs: list[int]) -> int {
    let n = xs.len
    n
}";

#[test]
fn two_reads_of_a_length_around_a_pop_are_two_values() {
    // `xs.len` read before the inlined pop, the index checked after it: not `xs.len - 1 < xs.len`.
    let src = format!("{CALLS}\nf(var xs: list[int]) -> int {{\n xs[int(u8(xs.len - 1)) + drop0(var xs)]\n}}");
    let (m, a) = analysed(&src, Checked);
    assert_eq!(left_of(&m, &a, "f", Bounds).len(), 1);
    assert_eq!(proven_of(&m, "f", Bounds), 0);
}

#[test]
fn a_comparison_of_two_reads_around_a_pop_can_hold() {
    // `xs.len + 0 > xs.len` holds when the right read comes after the pop: the arm runs.
    for left in ["xs.len", "lenof(xs)"] {
        let src = format!(
            "{CALLS}\nf(var xs: list[int]) -> int {{\n let ys: list[int] = [7, 8, 9]\n var k = 0\n if {left} + drop0(var xs) > xs.len {{\n  k = 100000\n }}\n ys[k]\n}}"
        );
        for arith in [Checked, Wrapping] {
            let mut m = optimized(&src);
            let a = kestrel_kir::range::remove_checks(&mut m, arith);
            let lines: Vec<u32> = left_of(&m, &a, "f", Bounds).iter().map(|c| line(c.line)).collect();
            assert!(lines.contains(&19), "{left} {arith:?}: {lines:?}");
            assert_eq!(proven_of(&m, "f", Bounds), 0, "{left} {arith:?}");
        }
    }
}

#[test]
fn a_passed_check_over_two_reads_teaches_nothing_wrong() {
    // `xs[xs.len + grow0(var xs)]` passes (the length grew in between); it must not make the rest
    // of the arm unreachable.
    let src = format!(
        "{CALLS}\nf(var xs: list[int], c: bool) -> int {{\n let ys: list[int] = [7, 8, 9]\n var k = 0\n if c {{\n  k = xs[xs.len + grow0(var xs)]\n  k = 100000\n }}\n ys[k]\n}}"
    );
    for arith in [Checked, Wrapping] {
        let mut m = optimized(&src);
        let a = kestrel_kir::range::remove_checks(&mut m, arith);
        let lines: Vec<u32> = left_of(&m, &a, "f", Bounds).iter().map(|c| line(c.line)).collect();
        assert!(lines.contains(&20), "{arith:?}: {lines:?}");
        assert_eq!(proven_of(&m, "f", Bounds), 0, "{arith:?}");
    }
}
