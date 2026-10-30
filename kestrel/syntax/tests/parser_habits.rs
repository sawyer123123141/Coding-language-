//! The messages for other languages' spellings (`list<T>`, `0..n`, `|a| a`, `n++` ...) change
//! nothing that parses: the code next to them still reads as before. Their wording is tested in
//! `tests/errors/newcomer_spellings.kes`.

mod common;
use common::*;

#[test]
fn comparisons_minus_minus_and_pipes_still_parse() {
    assert_eq!(body("let a = 1 - -2"), "{(let a (- 1 (- 2)))}");
    assert_eq!(body("let b = x < y"), "{(let b (< x y))}");
    assert_eq!(body("let c = 3 | 4"), "{(let c (| 3 4))}");
    // An index holding a comparison is not a type list with `<`.
    assert_eq!(body("let d = xs[i < n]"), "{(let d (index xs (< i n)))}");
    // `a -` at the end of a line still goes on to the next one.
    assert_eq!(body("let e = a -\n -b"), "{(let e (- a (- b)))}");
}

#[test]
fn each_spelling_is_one_error_and_the_next_line_still_parses() {
    let next = "\n let ok = 1 +\n}\n";
    for line in ["n++", "n--", "let r = 0..5", "let f = |a| a", "let mut m = 1", "let xs: list<int> = []"] {
        let errors = errs(&format!("main() {{\n {line}{next}"));
        // The second error is the unfinished `1 +` below: the line after was read on its own.
        assert_eq!(errors.len(), 2, "{line}: {errors:?}");
    }
}

#[test]
fn a_greater_than_after_a_comparison_is_still_a_chain() {
    assert_eq!(errs("main() {\n let ok = a < b > c\n}\n"), ["comparisons can't be chained; combine them with `and`"]);
    assert_eq!(errs("main() {\n let ok = list<int>()\n}\n"), ["Kestrel writes generic types with square brackets: `list[int]()`"]);
}
