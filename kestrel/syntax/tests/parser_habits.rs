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
    // `--` and `++` mean a step only at the end of a statement.
    assert_eq!(body("let f = a --b"), "{(let f (- a (- b)))}");
    assert_eq!(body("let g = f(a < b, c > d)"), "{(let g (call f (< a b) (> c d)))}");
    assert_eq!(body("let h = f(a < b, c > (d))"), "{(let h (call f (< a b) (> c d)))}");
    assert_eq!(body("let k = xs[i] < n"), "{(let k (< (index xs i) n))}");
    assert_eq!(body("let l = x\n .len"), "{(let l (. x len))}");
    assert_eq!(body("let m = 1.5 + 0.5"), "{(let m (+ 1.5 0.5))}");
    // A line ending in `>` or `>>` still goes on, unless a field or a declaration starts the next.
    assert_eq!(body("let n = a >\n b"), "{(let n (> a b))}");
    assert_eq!(body("let o = a >>\n b"), "{(let o (>> a b))}");
    // ... and in a map literal, whose key may be a comparison or a shift.
    assert_eq!(body("let p = { lo >\n hi: 1 }"), "{(let p (map ((> lo hi) 1)))}");
    assert_eq!(body("let q = { x >>\n shift: 1 }"), "{(let q (map ((>> x shift) 1)))}");
    // `elif` is still a name.
    assert_eq!(body("var elif = 1\nelif = 2\nelif += 1\nprint(elif)"), "{(var elif 1); (= elif 2); (+= elif 1); (call print elif)}");
    assert_eq!(body("print(elif.x, elif[0], elif(1))"), "{(call print (. elif x) (index elif 0) (call elif 1))}");
}

#[test]
fn a_comparison_or_a_plus_that_isnt_another_spelling_keeps_its_message() {
    assert_eq!(errs("main() {\n let j = 2 ++ 3\n}\n"), ["expected an expression, found `+`"]);
    assert_eq!(errs("main() {\n let j = 2 + +3\n}\n"), ["expected an expression, found `+`"]);
    assert_eq!(errs("main() {\n let y = &&x\n}\n"), ["Kestrel uses `and`, not `&&`"]);
    assert_eq!(errs("main() {\n let z = x < y > (z)\n}\n"), ["comparisons can't be chained; combine them with `and`"]);
    assert_eq!(errs("main() {\n let ok = best[int](a) < b\n}\n"), Vec::<String>::new());
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
