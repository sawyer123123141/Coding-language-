//! What `--release` trusts: an assumption statement stays evaluated (`Check`) when a bounds check
//! rests on it, and is `Trust`ed otherwise; a checked build evaluates every one. Also the budget
//! and a function the analysis gives up on.

mod range_common;

use kestrel_kir::range::Arith::{Checked, Wrapping};
use kestrel_kir::range::CheckKind::{Bounds, Division};
use kestrel_kir::range::{mark_assumptions, remove_checks};
use kestrel_kir::AssumeCheck::{Check, Trust};
use range_common::*;

const GET: &str = "pure get(xs: [int], i: int) -> int {\n @assume(i >= 0 and i < xs.len)\n xs[i]\n}";
const D: &str = "pure d(a: int, b: int) -> int {\n @assume(b > 0)\n a / b\n}";

#[test]
fn release_trusts_an_arithmetic_only_assumption() {
    let (m, _) = analysed(D, Wrapping);
    assert_eq!(proven_of(&m, "d", Division), 1);
    assert_eq!(assume_checks(&m, "d"), [Trust]);
    let (m, _) = analysed(D, Checked);
    assert_eq!(assume_checks(&m, "d"), [Check]);
}

#[test]
fn checked_resets_trust() {
    let (mut m, a) = analysed(&format!("{D}\n{GET}"), Wrapping);
    assert_eq!(assume_checks(&m, "d"), [Trust]);
    mark_assumptions(&mut m, &a, Checked);
    assert_eq!(assume_checks(&m, "d"), [Check]);
    assert_eq!(assume_checks(&m, "get"), [Check]);
}

#[test]
fn release_keeps_a_bounds_assumption() {
    let (m, _) = analysed(GET, Wrapping);
    assert_eq!(assume_checks(&m, "get"), [Check]);
}

#[test]
fn release_keeps_the_outer_of_a_redundant_pair() {
    let src = "pure f(xs: [int], i: int) -> int {\n @assume(i >= 0 and i < xs.len)\n let t = 0\n @assume(i >= 0 and i < xs.len)\n xs[i] + t\n}";
    let (m, a) = analysed(src, Wrapping);
    assert_eq!(proven_of(&m, "f", Bounds), 1);
    assert_eq!(assume_checks(&m, "f"), [Check, Trust]);
    let found: Vec<(u32, bool)> = uses(&m, &a, "f").iter().map(|u| (u.removed, u.bounds)).collect();
    assert_eq!(found.len(), 2);
    assert!(found.contains(&(1, true)) && found.contains(&(0, false)), "{found:?}");
}

#[test]
fn release_keeps_both_of_a_needed_pair() {
    let src = "pure f(xs: [int], i: int) -> int {\n @assume(i >= 0)\n let t = 0\n @assume(i < xs.len)\n xs[i] + t\n}";
    let (m, _) = analysed(src, Wrapping);
    assert_eq!(proven_of(&m, "f", Bounds), 1);
    assert_eq!(assume_checks(&m, "f"), [Check, Check]);
}

/// `get` inlined twice into `both`: in a loop that proves its index without the assumption
/// (copy A), and with an index from `id` (copy B).
pub const COPIES: &str = "id(x: int, d: int) -> int {\n if d == 0 {\n  return x\n }\n id(x, d - 1)\n}
pure get(xs: [int], i: int) -> int {\n @assume(i >= 0 and i < xs.len)\n xs[i]\n}
both(xs: [int]) -> int {\n var s = 0\n for k from 0 to xs.len {\n  s += get(xs, k)\n }\n let j = id(7, 1)\n s + get(xs, j)\n}";

#[test]
fn release_trusts_each_copy_on_its_own() {
    let (m, a) = analysed(COPIES, Wrapping);
    assert_eq!(proven_of(&m, "both", Bounds), 2);
    assert_eq!(assume_checks(&m, "both"), [Trust, Check], "copy A is trusted, copy B stays evaluated");
    let found = uses(&m, &a, "both");
    assert_eq!(found.len(), 2);
    assert_ne!(found[0].at, found[1].at);
    assert_eq!(found[0].ids, found[1].ids, "both copies carry the source's ids");
    assert_eq!([found[0].bounds, found[1].bounds], [false, true]);
    // A checked build evaluates both.
    let (m, _) = analysed(COPIES, Checked);
    assert_eq!(assume_checks(&m, "both"), [Check, Check]);
}

#[test]
fn the_plain_run_keeps_a_division_fact() {
    // `b` is 0 to 255, so the zero check of `a / b` that passes teaches the plain run that `b` is
    // 1 to 255; the trusted assumption removes that check, and a plain divide still traps on 0.
    let src = "pure f(a: int, c: int) -> int {\n let b = c & 255\n var r = 0\n arena {\n  @assume(b > 0)\n  r = a / b\n }\n r + 100 / b\n}";
    for (arith, check) in [(Wrapping, Trust), (Checked, Check)] {
        let (m, a) = analysed(src, arith);
        assert_eq!((proven_of(&m, "f", Division), left_of(&m, &a, "f", Division).len()), (2, 0), "{arith:?}");
        assert_eq!(assume_checks(&m, "f"), [check], "{arith:?}");
    }
    // The near miss: with `b` any `int`, a passed zero check teaches nothing (N2).
    let (m, a) = analysed(&src.replace("c & 255", "c"), Wrapping);
    assert_eq!(left_of(&m, &a, "f", Division).len(), 1);
}

/// `f` with two nests of `DEPTH` `while` loops, one after the other, each with `k` statements
/// inside; with `assume`, an assumption before the second nest (around half the body).
fn two_nests(k: usize, assume: bool) -> String {
    const DEPTH: usize = 4;
    let mut src = String::from("pure f(n: int) -> int {\n var t = 0\n");
    for nest in 0..2 {
        if nest == 1 && assume {
            src.push_str("@assume(n > 0)\n");
        }
        for d in 0..DEPTH {
            src.push_str(&format!("var c{nest}{d} = 0\nwhile c{nest}{d} < n {{\n"));
        }
        src.push_str(&"t = t ^ 1\n".repeat(k));
        for d in (0..DEPTH).rev() {
            src.push_str(&format!("c{nest}{d} += 1\n}}\n"));
        }
    }
    src.push_str("t\n}");
    src
}

fn gives_up(src: &str) -> bool {
    let (m, a) = analysed(src, Checked);
    checks(&m, &a, "f").gave_up
}

#[test]
fn an_assumption_does_not_spend_the_function_budget() {
    // K statements per nest come to about 90% of the budget (824 give up); the same with an
    // assumption around the second nest, whose block then runs twice in the deciding pass, still
    // finishes.
    const K: usize = 741;
    assert!(!gives_up(&two_nests(K, false)), "{K} statements fit the budget");
    assert!(gives_up(&two_nests(K * 6 / 5, false)), "a fifth more don't");
    let src = two_nests(K, true);
    let (m, a) = analysed(&src, Checked);
    assert!(!checks(&m, &a, "f").gave_up);
    assert_eq!(uses(&m, &a, "f").len(), 1);
}

#[test]
fn a_function_given_up_on_keeps_its_assumption() {
    let depth = 300;
    let mut src = String::from("pure f(x: int) -> int {\n var y = 0\n");
    for d in 0..depth {
        src.push_str(&format!("if x > {d} {{\n"));
    }
    src.push_str("@assume(x > 0)\ny = 100 / x\n");
    for _ in 0..depth {
        src.push_str("}\n");
    }
    src.push_str("y\n}");
    let m = std::thread::Builder::new().stack_size(64 << 20).spawn(move || optimized(&src)).unwrap().join().unwrap();
    let (m, a) = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            let mut m = m;
            let a = remove_checks(&mut m, Wrapping);
            (m, a)
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(checks(&m, &a, "f").gave_up);
    assert!(uses(&m, &a, "f").is_empty());
    let found = std::thread::Builder::new().stack_size(64 << 20).spawn(move || (assume_checks(&m, "f"), m)).unwrap().join().unwrap();
    assert_eq!(found.0, [Check]);
    std::thread::Builder::new().stack_size(64 << 20).spawn(move || drop(found.1)).unwrap().join().unwrap();
}

