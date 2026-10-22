//! A list a function returns keeps its length in a caller that inlines the function, with a passing
//! `@prove` on the function or an `@assume` around its tail as without them (the final review's
//! I1): neither puts the returned local into a temporary whose length range analysis can't follow.

mod range_common;

use kestrel_kir::range::Arith::{Checked, Wrapping};
use kestrel_kir::range::CheckKind::Bounds;
use range_common::*;

/// `make`, which returns a list of 100, with `before` in front of it and `inside` after its first
/// statement; `use_it` reads every item of what it returns.
fn src(before: &str, inside: &str, ending: &str) -> String {
    format!(
        "{before}pure make(n: int) -> [u8] {{\n var out = array[u8](100)\n{inside} out[n & 63] = 1\n {ending}\n}}\npure use_it(n: int) -> int {{\n let s = make(n)\n var t = 0\n for i from 0 to 100 {{\n  t += int(s[i])\n }}\n t\n}}"
    )
}

/// The lines of the bounds checks left in `use_it`.
fn left(src: &str, arith: kestrel_kir::range::Arith) -> Vec<u32> {
    let (m, a) = analysed(src, arith);
    left_of(&m, &a, "use_it", Bounds).iter().map(|c| line(c.line)).collect()
}

#[test]
fn the_callers_index_is_proven_as_without_annotations() {
    for ending in ["out", "return out"] {
        for arith in [Checked, Wrapping] {
            assert_eq!(left(&src("", "", ending), arith), Vec::<u32>::new(), "{ending}, {arith:?}: no annotation");
            assert_eq!(left(&src("@prove(no_checks)\n", "", ending), arith), Vec::<u32>::new(), "{ending}, {arith:?}: a passing `@prove`");
            assert_eq!(left(&src("", " @assume(n >= 0)\n", ending), arith), Vec::<u32>::new(), "{ending}, {arith:?}: an `@assume` around the tail");
            let both = src("@prove(no_checks)\n", " @assume(n >= 0)\n", ending);
            assert_eq!(left(&both, arith), Vec::<u32>::new(), "{ending}, {arith:?}: both");
        }
    }
}

#[test]
fn the_near_miss_keeps_its_check() {
    // Reading one past the end: the length is known, so the last read keeps its check.
    let near = src("@prove(no_checks)\n", " @assume(n >= 0)\n", "out").replace("from 0 to 100", "from 0 to 101");
    for arith in [Checked, Wrapping] {
        assert_eq!(left(&near, arith), [12], "{arith:?}");
    }
}
