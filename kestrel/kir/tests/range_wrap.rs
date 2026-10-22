//! Range analysis of the wrapping operators `*% +% -%`: they have no check in any build, and
//! their results are the type's range unless the exact result fits.

mod range_common;

use kestrel_kir::range::Arith::{Checked, Wrapping};
use kestrel_kir::range::CheckKind::{Bounds, Overflow};
use range_common::*;

#[test]
fn a_wrapping_op_has_no_check() {
    for arith in [Checked, Wrapping] {
        for op in ["*%", "+%", "-%"] {
            let (m, a) = analysed(&format!("pure f(a: int, b: int) -> int {{ a {op} b }}"), arith);
            assert_eq!(counts(&m, &a, "f"), (0, 0), "{op}");
        }
        let (m, a) = analysed("pure f(a: u8, b: u8) -> u8 { a *% b }", arith);
        assert_eq!(counts(&m, &a, "f"), (0, 0));
        let (m, a) = analysed("pure f(var h: int, k: int) -> int {\n h *%= k\n h +%= 1\n h -%= k\n h\n}", arith);
        assert_eq!(counts(&m, &a, "f"), (0, 0));
    }
}

#[test]
fn its_result_has_the_wrapped_range() {
    // 0 to 255, times 3: 0 to 765 fits, so the `+ 1` is proven.
    let (m, a) = analysed("pure f(h: int) -> int { (h & 255) *% 3 + 1 }", Checked);
    assert_eq!(counts(&m, &a, "f"), (1, 0));
    // An unknown `h` times 3 may have wrapped: any `int`, so the `+ 1` keeps its check.
    let (m, a) = analysed("pure g(h: int) -> int { h *% 3 + 1 }", Checked);
    assert_eq!(counts(&m, &a, "g"), (0, 1));
    assert_eq!(left_of(&m, &a, "g", Overflow).len(), 1);
}

#[test]
fn a_result_that_may_have_wrapped_is_not_its_exact_range() {
    // 0 to 0x7FFFFFFF plus 0x7FFFFFFF reaches past `int`'s top, so it may have wrapped.
    let src = "pure f(h: int) -> int { ((h & 0x7FFFFFFF) +% 0x7FFFFFFF) + 1 }";
    for arith in [Checked, Wrapping] {
        let (m, a) = analysed(src, arith);
        let overflow = left_of(&m, &a, "f", Overflow).len();
        assert_eq!(overflow, usize::from(arith.checked()), "the `+ 1` keeps its check in a checked build only");
    }
}

#[test]
fn a_wrapped_hex_literal_is_negative() {
    // 0xFFFFFFFF is -1 in an `int`: the product is -1 to 0.
    let (m, a) = analysed("pure f(h: int) -> int { (h & 1) * 0xFFFFFFFF }", Checked);
    assert_eq!(counts(&m, &a, "f"), (1, 0));
}

#[test]
fn a_wrapping_add_gives_no_linear_form_when_it_may_wrap() {
    // `i` is `h + 1` unless `h` is the largest `int`, when `i > h` is false. A linear form
    // would make the `else` branch dead and remove the check on `h * h` in it.
    let src = "pure f(h: int) -> int {\n let i = h +% 1\n if i > h {\n  return 0\n }\n h * h\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1, "{:#?}", checks(&m, &a, "f").left);
    let (m, a) = analysed(src, Wrapping);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 0);
}

#[test]
fn a_wrapping_add_that_cannot_wrap_keeps_its_linear_form() {
    // `n` is 0 to 255, so `n +% 1` is exactly `n + 1`: below `xs.len` makes the index in range.
    let src = "pure f(xs: list[int], h: int) -> int {
 let n = h & 255
 if n +% 1 < xs.len {
  return xs[n +% 1]
 }
 0
}";
    for arith in [Checked, Wrapping] {
        let (m, a) = analysed(src, arith);
        assert_eq!(left_of(&m, &a, "f", Bounds).len(), 0, "{:#?}", checks(&m, &a, "f").left);
    }
}

#[test]
fn a_wrapping_add_that_may_wrap_keeps_its_bounds_check() {
    // `n +% 1` wraps to the smallest `int` when `n` is the largest: below `xs.len`, but not an index.
    let src = "pure f(xs: list[int], h: int) -> int {
 let n = h & 0x7FFFFFFF
 if n +% 1 < xs.len {
  return xs[n +% 1]
 }
 0
}";
    for arith in [Checked, Wrapping] {
        let (m, a) = analysed(src, arith);
        assert_eq!(left_of(&m, &a, "f", Bounds).len(), 1, "{:#?}", checks(&m, &a, "f").left);
    }
}

#[test]
fn the_report_lists_nothing_for_a_wrapping_op() {
    for arith in [Checked, Wrapping] {
        let (_, r) = reported("pure f(a: int, b: int) -> int { a *% b +% a -% 1 }", arith);
        assert!(func_report(&r, "f").items.is_empty(), "{:#?}", func_report(&r, "f").items);
        assert_eq!(func_report(&r, "f").checks_left(), 0);
    }
}
