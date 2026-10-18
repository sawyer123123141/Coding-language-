//! `sort` and `sort_by`: they change their receiver's order (arrays included), take a pure key
//! that gives a number, string or bool, and obey the exclusivity rule and the loop rule.

use kestrel_check::{check, ListMethod, Res, Tables};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().filter(|d| d.is_error()).map(|d| d.message).collect())
}

fn ok(src: &str) -> Tables {
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "unexpected errors: {errs:?}");
    t.expect("tables")
}

fn errors(src: &str) -> Vec<String> {
    run(src).1
}

#[test]
fn the_sorts_change_the_order_not_the_length() {
    for m in [ListMethod::Sort, ListMethod::SortBy] {
        assert!(m.changes_receiver(), "{m:?}");
        assert!(m.reorders(), "{m:?}");
    }
    assert!(ListMethod::SortBy.takes_fn());
    assert!(!ListMethod::Sort.takes_fn());
    // The others that change a receiver change its length.
    assert!(!ListMethod::Clear.reorders());
}

#[test]
fn lists_and_arrays_sort_and_resolve() {
    let t = ok("main() {
    var xs: list[int] = [3, 1]
    var arr = array[float](3)
    xs.sort()
    arr.sort()
    xs.sort_by((e) { -e })
    arr.sort_by((e) { e > 1.0 })
}
");
    let count = |m: ListMethod| t.res.values().filter(|r| **r == Res::List(m)).count();
    assert_eq!((count(ListMethod::Sort), count(ListMethod::SortBy)), (2, 2));
}

#[test]
fn a_literal_key_gets_a_stack_environment_and_names_may_be_functions() {
    let t = ok("struct E {
    hp: int
}

pure hp(e: E) -> int { e.hp }

main() {
    var es: list[E] = []
    let k = 2
    let stored = (e: E) -> int { e.hp + k }
    es.sort_by((e) { e.hp + k })
    es.sort_by(hp)
    es.sort_by(stored)
}
");
    let mut ids: Vec<_> = t.closures.keys().copied().collect();
    ids.sort();
    let stack: Vec<bool> = ids.iter().map(|id| t.stack_closures.contains(id)).collect();
    // `stored` comes first in the source, so it is not a stack closure; the literal key is.
    assert_eq!(stack, vec![false, true]);
}

#[test]
fn the_receiver_must_be_something_that_can_change() {
    let e = errors("main() {\n let ys = [2, 1]\n ys.sort()\n}\n");
    assert_eq!(e, ["can't call `sort` on `ys`: it changes `ys`, which was declared with `let`"]);
    let e = errors("f(zs: list[int]) {\n zs.sort_by((e) { e })\n}\nmain() {}\n");
    assert_eq!(e, ["can't call `sort_by` on parameter `zs`: it changes `zs`, which isn't `var`"]);
    let e = errors("main() {\n [2, 1].sort()\n}\n");
    assert_eq!(e, ["`sort` changes its receiver, so it needs a variable, not a temporary"]);
    // A `var` parameter is fine.
    ok("f(var zs: list[int]) {\n zs.sort_by((e) { e })\n}\nmain() {}\n");
}

#[test]
fn a_loop_over_the_list_or_one_holding_it_refuses_the_sort() {
    let e = errors("main() {\n var xs: list[int] = [2, 1]\n for x in xs {\n xs.sort()\n }\n}\n");
    assert_eq!(e, ["can't sort `xs` while looping over it; sort it after the loop"]);
    // A list inside an element of the looped list is a different list.
    ok("main() {\n var rows: list[list[int]] = [[2, 1]]\n for r in rows {\n rows[0].sort()\n }\n}\n");
    let e = errors("main() {\n var rows: list[list[int]] = [[2, 1]]\n for r in rows {\n rows.sort_by((q) { q.len })\n }\n}\n");
    assert_eq!(e, ["can't sort `rows` while looping over it; sort it after the loop"]);
}

#[test]
fn only_numbers_strings_and_bools_sort() {
    let e = errors("main() {\n var vs: list[vec2] = []\n vs.sort()\n}\n");
    assert_eq!(e, ["`sort` needs numbers, strings or bools; `list[vec2]` holds `vec2`; use `sort_by` with a key"]);
    let e = errors("main() {\n var vs: list[int] = []\n vs.sort_by((e) { vec2(e, e) })\n}\n");
    assert_eq!(e, ["`sort_by` needs a key that is a number, string or bool; this one gives `vec2`"]);
    ok("main() {
    var a: list[string] = []
    var b: list[bool] = []
    var c: list[u8] = []
    var d: list[f64] = []
    a.sort()
    b.sort()
    c.sort()
    d.sort()
    a.sort_by((s) { s.len })
    b.sort_by((s) { s })
}
");
}

#[test]
fn the_key_must_be_pure() {
    let e = errors("main() {\n var xs: list[int] = []\n xs.sort_by((e) { print(e)\n e })\n}\n");
    assert_eq!(e, ["this closure isn't pure (it calls `print`), but `sort_by` needs a pure one"]);
    let e = errors("noisy(n: int) -> int {\n print(n)\n n\n}\nmain() {\n var xs: list[int] = []\n xs.sort_by(noisy)\n}\n");
    assert_eq!(e, ["`noisy` isn't `pure`, but `sort_by` needs a pure function"]);
    let e = errors("f(var xs: list[int], g: (int) -> int) {\n xs.sort_by(g)\n}\nmain() {}\n");
    assert_eq!(e, ["`g` isn't a pure function type, but `sort_by` needs a pure one"]);
    // The same holds in a `pure` function and in a `par for` body.
    ok("pure f(var xs: list[int], g: pure (int) -> int) {\n xs.sort_by(g)\n xs.sort_by((e) { g(e) })\n}\nmain() {}\n");
}

#[test]
fn the_key_cant_read_what_the_sort_changes() {
    let e = errors("main() {\n var xs: list[int] = [2, 1]\n xs.sort_by((e) { xs.len + e })\n}\n");
    assert_eq!(e, ["`xs` is changed by this call, so the closure passed to it can't read it"]);
    // A different field of the same struct is fine.
    ok("struct S {
    items: list[int]
    center: int

    arrange(var self) {
        self.items.sort_by((e) { e + self.center })
    }
}

main() {}
");
    // A key stored in the list it sorts is read from memory the sort moves.
    let e = errors("struct H {\n key: pure (H) -> int\n}\nmain() {\n var hs: list[H] = []\n hs.sort_by(hs[0].key)\n}\n");
    assert_eq!(e, ["can't call `hs[0].key` while this call changes `hs`, which may hold its memory"]);
    // ... but a key stored beside it, or in another struct's field, is not.
    ok("struct H {
    key: pure (int) -> int
    items: list[int]
}

main() {
    var h = H(key: (e) { e }, items: [2, 1])
    h.items.sort_by(h.key)
}
");
}

#[test]
fn wrong_arguments_say_how_the_method_is_used() {
    assert_eq!(errors("main() {\n var xs: list[int] = []\n xs.sort(1)\n}\n"), ["`sort` takes no arguments"]);
    assert_eq!(errors("main() {\n var xs: list[int] = []\n xs.sort_by()\n}\n"), ["`sort_by` takes one function"]);
    assert_eq!(
        errors("main() {\n var xs: list[int] = []\n xs.sort_by((e: float) { e })\n}\n"),
        ["`sort_by` needs a function that takes `int`, found `pure (float) -> float`"]
    );
}
