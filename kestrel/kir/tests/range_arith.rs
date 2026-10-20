//! Range analysis of arithmetic: overflow, shift, division and narrowing checks, each proof with a
//! near miss that must keep its check.

mod range_common;

use kestrel_kir::range::Arith::{Checked, Wrapping};
use kestrel_kir::range::CheckKind::{Division, Narrowing, Overflow, Shift};
use range_common::*;

/// The voxel world's `hash`, copied from `bench/voxels3d/voxels3d.kes`.
const HASH: &str = "pure hash(x: int, y: int, z: int, seed: int) -> int {
    var h = i64(x) * 73856093 ^ i64(y) * 19349663 ^ i64(z) * 83492791 ^ i64(seed) * 1610612741
    h = h & 0x7FFFFFFF
    h = (h ^ (h >> 13)) * 0x5BD1E995 & 0x7FFFFFFF
    int(h ^ (h >> 15))
}";

#[test]
fn the_voxel_hash_has_no_checks_left() {
    let (m, a) = analysed(HASH, Checked);
    let (proven, left) = counts(&m, &a, "hash");
    assert_eq!(left, 0, "{:#?}", checks(&m, &a, "hash").left);
    assert!(proven >= 8, "the four products, two shifts, the last product and the narrowing: {proven}");
}

/// `(h & 0x7FFFFFFF) * k` for a 64-bit `h`.
fn masked_times(k: &str) -> String {
    format!("pure f(h: i64) -> i64 {{ (h & 0x7FFFFFFF) * {k} }}")
}

#[test]
fn a_mask_times_a_31_bit_constant_fits() {
    let (m, a) = analysed(&masked_times("0x5BD1E995"), Checked);
    assert_eq!(counts(&m, &a, "f"), (1, 0));
}

#[test]
fn times_a_33_bit_constant_does_not() {
    let (m, a) = analysed(&masked_times("0x15BD1E995"), Checked);
    assert_eq!(counts(&m, &a, "f"), (0, 1));
}

#[test]
fn a_constant_shift_is_proven() {
    let (m, a) = analysed("pure f(x: int) -> int {\n let a = x >> 3\n let b = 1 << 5\n a ^ b\n}", Checked);
    assert_eq!(counts(&m, &a, "f"), (2, 0));
}

#[test]
fn a_computed_shift_is_not() {
    let (m, a) = analysed("pure f(n: int) -> int { 1 << n }", Checked);
    assert_eq!(counts(&m, &a, "f"), (0, 1));
}

#[test]
fn a_shift_by_minus_one_to_three_is_not() {
    let src = "pure f(x: int, n: int) -> int {\n if n >= -1 and n <= 3 {\n  return x >> n\n }\n 0\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(left_of(&m, &a, "f", Shift).len(), 1);
    assert_eq!(proven_of(&m, "f", Shift), 0);
}

#[test]
fn narrowing_a_masked_value_is_proven() {
    let (m, a) = analysed("pure f(h: i64) -> int { int(h & 0x7FFFFFFF) }", Checked);
    assert_eq!(counts(&m, &a, "f"), (1, 0));
}

#[test]
fn narrowing_any_i64_is_not() {
    let (m, a) = analysed("pure f(h: i64) -> int { int(h) }", Checked);
    assert_eq!(counts(&m, &a, "f"), (0, 1));
}

#[test]
fn u8_of_minus_one_to_ten_is_not() {
    let src = "pure f(x: int) -> u8 {\n if x >= -1 and x <= 10 {\n  return u8(x)\n }\n 0\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(left_of(&m, &a, "f", Narrowing).len(), 1);
    assert_eq!(proven_of(&m, "f", Narrowing), 0);
}

#[test]
fn a_small_u8_add_is_proven() {
    let src = "pure f(a: u8, b: u8) -> u8 {\n if a <= 127 and b <= 127 {\n  return a + b\n }\n 0\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(proven_of(&m, "f", Overflow), 1);
    assert!(left_of(&m, &a, "f", Overflow).is_empty());
}

#[test]
fn u8_200_plus_100_is_not() {
    let src = "pure f(a: u8, b: u8) -> u8 {\n if a == 200 and b == 100 {\n  return a + b\n }\n 0\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(left_of(&m, &a, "f", Overflow).len(), 1);
    assert_eq!(proven_of(&m, "f", Overflow), 0);
}

#[test]
fn a_guarded_divisor_is_proven() {
    let (m, a) = analysed("pure f(a: int, b: int) -> int {\n if b > 0 {\n  return a / b\n }\n 0\n}", Checked);
    assert_eq!(proven_of(&m, "f", Division), 1);
    assert!(left_of(&m, &a, "f", Division).is_empty());
}

#[test]
fn an_unguarded_one_is_not() {
    let (m, a) = analysed("pure f(a: int, b: int) -> int { a / b }", Checked);
    assert_eq!(counts(&m, &a, "f"), (0, 1));
    assert_eq!(left_of(&m, &a, "f", Division).len(), 1);
}

#[test]
fn a_divisor_of_minus_one_with_a_nonnegative_dividend_is_proven() {
    let (m, a) = analysed("pure f(a: int) -> int {\n if a >= 0 {\n  return a / -1\n }\n 0\n}", Checked);
    assert_eq!(proven_of(&m, "f", Division), 1);
    assert!(left_of(&m, &a, "f", Division).is_empty());
    // Any dividend can be the minimum: the division by -1 keeps its check.
    let (m, a) = analysed("pure f(a: int) -> int { a / -1 }", Checked);
    assert_eq!(left_of(&m, &a, "f", Division).len(), 1);
}

#[test]
fn a_u8_literal_divisor_that_wraps_to_zero_is_a_site() {
    // The checker refuses `x / 256` on a `u8`, so the literal is put in KIR by hand: codegen
    // writes it wrapped to the type, as 0, and keeps the zero check.
    let mut m = optimized("pure f(x: u8) -> u8 { x / 7 }");
    let f = idx(&m, "f");
    kestrel_kir::visit_mut::VisitMut::visit_stmts(&mut Literal(256), &mut m.funcs[f].body);
    let a = kestrel_kir::range::remove_checks(&mut m, Checked);
    assert_eq!(left_of(&m, &a, "f", Division).len(), 1, "{:#?}", m.funcs[f].body);
    // 7 itself is no site.
    let (m, a) = analysed("pure f(x: u8) -> u8 { x / 7 }", Checked);
    assert_eq!(counts(&m, &a, "f"), (0, 0));
}

/// Replaces every integer literal 7 with this one.
struct Literal(i64);

impl kestrel_kir::visit_mut::VisitMut for Literal {
    fn visit_expr(&mut self, e: &mut kestrel_kir::Expr) {
        kestrel_kir::visit_mut::walk_expr(self, e);
        if let kestrel_kir::ExprKind::Int(7) = e.kind {
            e.kind = kestrel_kir::ExprKind::Int(self.0);
        }
    }
}

#[test]
fn an_inlined_constant_argument_proves_its_shift() {
    let src = "pure octave(x: int, shift: int) -> int {\n let mask = (1 << shift) - 1\n let cx = x >> shift\n cx + (x & mask)\n}
pure f(x: int) -> int { octave(x, 5) }";
    let (m, a) = analysed(src, Checked);
    assert!(left_of(&m, &a, "f", Shift).is_empty(), "{:#?}", checks(&m, &a, "f").left);
    assert_eq!(proven_of(&m, "f", Shift), 2);
    // `(1 << 5) - 1` and `cx + (x & 31)` can't overflow either.
    assert_eq!(counts(&m, &a, "f").1, 0, "{:#?}", checks(&m, &a, "f").left);
    // Its own copy knows nothing about `shift`: the first shift keeps its check, and the second
    // is proven by what the first one's check taught (`shift` is 0 to 31 once it passed).
    assert_eq!(left_of(&m, &a, "octave", Shift).len(), 1);
    assert_eq!(proven_of(&m, "octave", Shift), 1);
}

#[test]
fn release_forgets_a_wrapped_value() {
    let src = "pure f(x: int, z: int) -> int {\n if x >= 0 {\n  let y = x + 1\n  return z / y\n }\n 0\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(proven_of(&m, "f", Division), 1, "checked: `y` is 1 or more");
    assert!(left_of(&m, &a, "f", Division).is_empty());
    let (m, a) = analysed(src, Wrapping);
    assert_eq!(proven_of(&m, "f", Division), 0, "release: `x + 1` may wrap to the minimum");
    assert_eq!(left_of(&m, &a, "f", Division).len(), 1);
}

#[test]
fn release_shifts_teach_nothing() {
    let src = "pure f(n: int, z: int) -> int {\n if n >= 0 {\n  let y = 1 << n\n  return z / (32 - n) + y\n }\n 0\n}";
    let (m, a) = analysed(src, Checked);
    assert_eq!(proven_of(&m, "f", Division), 1, "checked: the shift's check taught `n <= 31`");
    let (m, a2) = analysed(src, Wrapping);
    assert_eq!(proven_of(&m, "f", Division), 0, "release: no shift check ran, so `32 - n` can be 0");
    assert_eq!(left_of(&m, &a2, "f", Division).len(), 1);
    assert!(left_of(&m, &a, "f", Division).is_empty());
}

#[test]
fn a_reference_local_sees_writes_through_its_list() {
    let src = "f(var xs: list[int]) -> int {
 var s = 0
 for x in xs {
  if x >= 0 and x < 100 {
   xs[0] = 1000000000
   s = x * 100
  }
 }
 s
}";
    let (m, a) = analysed(src, Checked);
    let left = left_of(&m, &a, "f", Overflow);
    assert!(left.iter().any(|c| line(c.line) == 6), "`x * 100` keeps its check: {left:#?}");
}

#[test]
fn the_left_text_names_the_ranges() {
    let (m, a) = analysed("pure f(a: int, b: int) -> int { a * b }", Checked);
    let left = &checks(&m, &a, "f").left;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].kind, Overflow);
    assert_eq!(left[0].text, "`a * b` may exceed `int` (`a` can be any `int`; `b` can be any `int`)");
    assert!(left[0].loops.is_empty());
}

#[test]
fn literal_divisors_are_not_sites() {
    let (m, a) = analysed("pure f(x: int) -> int { x / 4 }", Checked);
    assert_eq!(counts(&m, &a, "f"), (0, 0));
}

#[test]
fn release_keeps_no_overflow_sites() {
    // With `--release` an add has no check, so there is nothing to prove or list.
    let (m, a) = analysed("pure f(a: int, b: int) -> int { a + b }", Wrapping);
    assert_eq!(counts(&m, &a, "f"), (0, 0));
}


#[test]
fn a_module_analysed_twice_proves_the_same() {
    // The second run sees the first one's `Proven` nodes: they are no sites, and nothing else
    // changes.
    let (mut m, a) = analysed(HASH, Checked);
    let first = proven_kinds(&m, "hash");
    let again = kestrel_kir::range::remove_checks(&mut m, Checked);
    assert_eq!(proven_kinds(&m, "hash"), first);
    assert_eq!(checks(&m, &again, "hash").left, checks(&m, &a, "hash").left);
    assert_eq!(checks(&m, &again, "hash").removed, 0, "nothing new to remove");
}
