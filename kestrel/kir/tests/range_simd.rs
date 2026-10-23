//! Range analysis of `ivec4`: one range for all four lanes (the widest), every interval helper
//! asked about `int`, and a vector check removed only when that range proves it for every lane.

mod range_common;

use kestrel_kir::range::Arith::{Checked, Wrapping};
use kestrel_kir::range::CheckKind::{Bounds, Overflow, Shift};
use range_common::*;

const ID: &str = "id(x: int, d: int) -> int {\n if d == 0 {\n  return x\n }\n id(x, d - 1)\n}\n";

#[test]
fn masked_lanes_lose_their_checks() {
    let (m, a) = analysed("pure f(h: ivec4) -> ivec4 {\n let k = h & 0xFFFF\n k * 3 + ivec4(0, 1, 0, 1)\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (2, 0), "{:#?}", checks(&m, &a, "f").left);
}

#[test]
fn an_unbounded_product_keeps_its_check() {
    let (m, a) = analysed("pure f(a: ivec4, b: ivec4) -> ivec4 {\n a * b\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (0, 1));
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1);
}

#[test]
fn splats_and_makes_carry_ranges() {
    for body in ["ivec4(n & 255) * ivec4(n & 255)", "ivec4(n & 255, 0, 0, 1) * 1000"] {
        let (m, a) = analysed(&format!("pure f(n: int) -> ivec4 {{\n {body}\n}}"), Checked);
        assert_eq!(checks(&m, &a, "f").left.len(), 0, "{body}: {:#?}", checks(&m, &a, "f").left);
    }
    // A scalar product of a lane.
    let (m, a) = analysed("pure f(n: int) -> int {\n ivec4(n & 255).y * 1000\n}", Checked);
    assert_eq!(checks(&m, &a, "f").left.len(), 0, "{:#?}", checks(&m, &a, "f").left);
    // A swizzle and a lane of a computed vector keep the vector's range.
    let (m, a) = analysed("pure f(n: int) -> int {\n let v = ivec4(n & 255, 1, 2, 3).wzyx * 1000\n (v + 1).z * 2\n}", Checked);
    assert_eq!(checks(&m, &a, "f").left.len(), 0, "{:#?}", checks(&m, &a, "f").left);
    // The hull of a make: one wide lane makes every lane wide.
    let (m, a) = analysed("pure f(n: int) -> ivec4 {\n ivec4(n & 255, n, 0, 1) * 1000\n}", Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1);
}

#[test]
fn a_lane_write_widens_the_range() {
    let (m, a) = analysed("pure f(n: int) -> ivec4 {\n var v = ivec4(0)\n v.y = n\n v + 1\n}", Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1);
    let (m, a) = analysed("pure f(n: int) -> ivec4 {\n var v = ivec4(0)\n v.y = 5\n v + 1\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (1, 0));
    // The three other lanes stay: writing a small lane doesn't make a wide vector small.
    let (m, a) = analysed("pure f(n: int) -> ivec4 {\n var v = ivec4(n)\n v.y = 5\n v + 1\n}", Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1);
    // A lane passed as `var` writes the vector too.
    let src = "bump(var x: int) {\n x = x + 1000000\n}\nf(n: int) -> ivec4 {\n var v = ivec4(n & 255)\n bump(var v.y)\n v * 10000\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1, "{:#?}", checks(&m, &a, "f").left);
    // And to a call that isn't inlined.
    let src = "bump(var x: int, d: int) {\n if d == 0 {\n  x = 2147483647\n  return\n }\n bump(var x, d - 1)\n}\nf(n: int) -> ivec4 {\n var v = ivec4(n & 255)\n bump(var v.y, 3)\n v + 1\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1, "{:#?}", checks(&m, &a, "f").left);
}

#[test]
fn a_lane_condition_teaches_nothing() {
    for cond in ["v.x < 10", "v.x >= 0 and v.x < 10"] {
        let (m, a) = analysed(&format!("f(v: ivec4) {{\n if {cond} {{\n  print(v * 1000)\n }}\n}}"), Checked);
        assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1, "{cond}");
    }
    for assume in ["@assume(v.x < 10)", "@assume(v.x >= 0) @assume(v.x < 10)"] {
        let (m, a) = analysed(&format!("f(v: ivec4) {{\n {assume}\n print(v * 1000)\n}}"), Checked);
        assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1, "{assume}");
        assert!(!uses(&m, &a, "f").is_empty() && uses(&m, &a, "f").iter().all(|u| u.removed == 0), "{assume}: {:?}", uses(&m, &a, "f"));
    }
    // A lane of a vector in a local, too.
    let (m, a) = analysed("f(n: int) {\n let v = ivec4(n)\n if v.x >= 0 and v.x < 10 {\n  print(v * 1000)\n }\n}", Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1);
}

#[test]
fn the_shift_amount_decides_the_shift_check() {
    let shifts = |body: &str| {
        let (m, a) = analysed(&format!("pure f(h: ivec4, n: int) -> ivec4 {{\n {body}\n}}"), Checked);
        left_of(&m, &a, "f", Shift).len()
    };
    assert_eq!(shifts("h >> n"), 1);
    assert_eq!(shifts("h >> 13"), 0);
    assert_eq!(shifts("h >> 1"), 0);
    assert_eq!(shifts("h >> (n & 31)"), 0);
    // The width is a lane's, 32, not 64.
    assert_eq!(shifts("h << (n & 63)"), 1);
    // The value shifted is the vector's range: 0 to 255 shifted by up to 7 fits, so the `+ 1` goes.
    let (m, a) = analysed("pure f(h: ivec4, n: int) -> ivec4 {\n ((h & 255) << (n & 7)) + 1\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (2, 0));
    // `>>` is arithmetic: -256 to 255 shifted right stays inside -256 to 255, so `* 8000000` fits.
    let (m, a) = analysed("pure f(h: ivec4, n: int) -> ivec4 {\n let k = (h & 511) - 256\n (k >> (n & 3)) * 8000000\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (3, 0));
}

#[test]
fn bit_not_is_signed_on_lanes() {
    let (m, a) = analysed("pure g(h: ivec4) -> ivec4 {\n let k = h & 7\n ~k - 2147483647\n}", Checked);
    assert_eq!(left_of(&m, &a, "g", Overflow).len(), 1);
    // `~` of 0 to 7 is -8 to -1, so this one fits.
    let (m, a) = analysed("pure g(h: ivec4) -> ivec4 {\n let k = h & 7\n ~k - 2147483640\n}", Checked);
    assert_eq!(counts(&m, &a, "g"), (1, 0));
}

#[test]
fn a_wrapping_lane_result_wraps() {
    let (m, a) = analysed("pure f(h: ivec4) -> ivec4 {\n (h & 255) *% 3 + 1\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (1, 0));
    let (m, a) = analysed("pure f(h: ivec4) -> ivec4 {\n h *% 3 + 1\n}", Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1);
    // A wrapping result that may have wrapped is any `int`, not its exact range.
    let (m, a) = analysed("pure f(h: ivec4) -> ivec4 {\n ((h & 0x7FFFFFFF) +% 0x7FFFFFFF) + 1\n}", Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1);
    // Not its exact range clipped either: 0x8000 * 0x10000 wraps to the lowest `int`.
    let (m, a) = analysed("pure f(h: ivec4) -> ivec4 {\n ((h & 0xFFFF) *% 0x10000) - 1\n}", Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1);
}

#[test]
fn a_converted_float_is_any_int() {
    let (m, a) = analysed("pure f(v: vec4) -> ivec4 {\n ivec4(v) + 1\n}", Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1);
}

#[test]
fn a_loaded_vector_is_any_int() {
    let srcs = [
        "pure f(vs: [ivec4], i: int) -> ivec4 {\n vs[i] + 1\n}",
        "struct P {\n c: ivec4\n}\npure f(p: P) -> ivec4 {\n p.c + 1\n}",
        "f(var v: ivec4) -> ivec4 {\n v.x = 5\n v + 1\n}",
    ];
    for src in srcs {
        let (m, a) = analysed(src, Checked);
        assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1, "{src}");
    }
}

#[test]
fn a_loop_widens_a_vector() {
    // `v` grows by 1 each round: its top end goes to the limit, so `v + 1` keeps its check.
    let src = "pure f(n: int) -> ivec4 {\n var v = ivec4(0)\n for i from 0 to n {\n  v = v + 1\n }\n v\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1);
    // A lane written in a loop: the same.
    let src = "pure f(n: int) -> ivec4 {\n var v = ivec4(0)\n for i from 0 to n {\n  v.z = v.z + 1\n }\n v + 1\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 2, "{:#?}", checks(&m, &a, "f").left);
}

#[test]
fn vectors_leave_scalar_facts_alone() {
    let src = format!("{ID}f(var out: [int]) {{\n for i from 0 to out.len {{\n  let v = ivec4(i)\n  out[i] = (v * id(3, 2)).x\n }}\n}}");
    let (m, a) = analysed(&src, Checked);
    assert_eq!(left_of(&m, &a, "f", Bounds).len(), 0, "{:#?}", checks(&m, &a, "f").left);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1, "{:#?}", checks(&m, &a, "f").left);
}

#[test]
fn a_vector_has_no_linear_form() {
    // Lane y is `n`, not `i`: a linear form carried through the make, the `+ 1` and the lane read
    // would take the index for `i` and remove its bounds check.
    for index in ["ivec4(i, n, 0, 0).y", "(ivec4(i, n, 0, 0) + 1).y - 1", "ivec4(i, n, 0, 0).yxzw.x"] {
        let src = format!("f(var out: [int], n: int) {{\n for i from 0 to out.len {{\n  out[{index}] = 1\n }}\n}}");
        let (m, a) = analysed(&src, Checked);
        assert_eq!(left_of(&m, &a, "f", Bounds).len(), 1, "{index}: {:#?}", checks(&m, &a, "f").left);
    }
}

#[test]
fn a_negated_lane_wraps_under_release() {
    // `v` is the lowest `int` or one above; under `--release` `-v` wraps the lowest to itself, so
    // the index can be 1 and its bounds check (kept in both builds) must stay.
    let src = "f(var out: [int], h: ivec4) {\n if out.len == 1 {\n  let v = (h & 1) - 2147483647 - 1\n  out[(-v).x - 2147483647] = 7\n }\n}";
    let (m, a) = analysed(src, Wrapping);
    assert_eq!(left_of(&m, &a, "f", Bounds).len(), 1, "{:#?}", checks(&m, &a, "f").left);
}

#[test]
fn release_has_no_vector_checks() {
    let srcs = [
        "pure f(h: ivec4) -> ivec4 {\n let k = h & 0xFFFF\n k * 3 + ivec4(0, 1, 0, 1)\n}",
        "pure f(a: ivec4, b: ivec4) -> ivec4 {\n a * b\n}",
        "pure f(h: ivec4, n: int) -> ivec4 {\n (-h >> n) << n\n}",
        "pure f(v: vec4) -> ivec4 {\n ivec4(v) + 1\n}",
        "pure f(n: int) -> ivec4 {\n var v = ivec4(0)\n v.y = n\n v + 1\n}",
    ];
    for src in srcs {
        let (m, a) = analysed(src, Wrapping);
        assert_eq!(counts(&m, &a, "f"), (0, 0), "{src}");
    }
}
