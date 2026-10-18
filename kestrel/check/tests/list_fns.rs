//! `filter`, `map`, `any` and `all`: the types they give, which literals get a stack
//! environment, the exclusivity rule, purity, and the discarded and owning cases.

use kestrel_check::{check, ListMethod, Res, Ty, Tables};
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
fn the_four_read_their_receiver() {
    for m in [ListMethod::Filter, ListMethod::Map, ListMethod::Any, ListMethod::All] {
        assert!(!m.changes_receiver(), "{m:?}");
        assert!(m.takes_fn(), "{m:?}");
    }
    assert!(ListMethod::Pop.changes_receiver());
    assert!(!ListMethod::Pop.takes_fn());
}

#[test]
fn they_resolve_and_give_lists_and_bools() {
    let t = ok("main() {
    let xs = [1, 2, 3]
    var ys: list[int] = [4, 5]
    let a = xs.filter((e) { e > 1 })
    let b = ys.map((e) { e * 2.5 })
    let c = ys.any((e) { e > 4 })
    let d = xs.all((e) { e > 0 })
    print(a, b, c, d)
}
");
    let of = |m: ListMethod| -> Ty {
        let (id, _) = t.res.iter().find(|(_, r)| **r == Res::List(m)).expect("the call");
        t.ty(*id)
    };
    // `filter` gives a list of the receiver's items (an array's too), `map` a list of the results.
    assert_eq!(t.ty_name(of(ListMethod::Filter)), "list[int]");
    assert_eq!(t.ty_name(of(ListMethod::Map)), "list[float]");
    assert_eq!(of(ListMethod::Any), Ty::Bool);
    assert_eq!(of(ListMethod::All), Ty::Bool);
}

#[test]
fn a_literal_argument_gets_a_stack_environment() {
    let t = ok("main() {
    let xs = [1, 2, 3]
    let k = 2
    let stored = (e: int) -> bool { e > k }
    print(xs.filter((e) { e > k }).len, xs.any(stored), xs.map((e) { e + k }).len)
}
");
    let mut ids: Vec<_> = t.closures.keys().copied().collect();
    ids.sort();
    let stack: Vec<bool> = ids.iter().map(|id| t.stack_closures.contains(id)).collect();
    // `filter`'s and `map`'s literals are direct arguments; `stored` is not.
    assert_eq!(stack, vec![false, true, true]);
}

#[test]
fn the_call_can_change_nothing_the_literal_reads() {
    let errs = errors(
        "make(var n: int) -> list[int] {
    n += 1
    var out: list[int] = []
    out.add(n)
    out
}

main() {
    var n = 0
    let xs = [1, 2, 3]
    print(make(var n).filter((e) { e > n }).len)
    print(xs.filter((e) { e > n }).len)
}
",
    );
    assert_eq!(errs, vec!["`n` is changed by this call, so the closure passed to it can't read it"]);
}

#[test]
fn a_literal_may_read_the_list_it_reads() {
    ok("main() {
    var xs: list[int] = [1, 2, 3]
    print(xs.filter((e) { e < xs.len }).len)
    print(xs.any((e) { xs.has(e + 1) }))
}
");
}

#[test]
fn map_takes_its_result_from_the_function() {
    let t = ok("name_of(s: string) -> string { s }

main() {
    let xs = [\"a\", \"b\"]
    let lens = xs.map((s) { s.len })
    let same = xs.map(name_of)
    let pairs = xs.map((s) { [s, s] })
    let nested = xs.map((s) { (y: int) -> int { y + s.len } })
    print(lens, same, pairs.len, nested.len)
}
");
    let elems: Vec<String> = t.elems.iter().map(|e| t.ty_name(*e)).collect();
    for want in ["int", "string", "[string]", "pure (int) -> int"] {
        assert!(elems.iter().any(|e| e == want), "no item type {want}: {elems:?}");
    }
}

#[test]
fn map_needs_a_value() {
    let errs = errors(
        "show(n: int) { print(n) }

main() {
    let xs = [1, 2]
    print(xs.map((e) { print(e) }).len)
    print(xs.map(show).len)
    print(xs.map(3).len)
}
",
    );
    assert_eq!(
        errs,
        vec![
            "`map` needs a closure that gives a value",
            "`map` needs a function that gives a value",
            "`map` needs a function that takes `int` and gives a value, found int"
        ]
    );
}

#[test]
fn filter_refuses_owning_items_but_the_others_read_them() {
    let errs = errors(
        "struct Pool {
    mem: Arena = Arena()
    id: int
}

main() {
    let pools: list[Pool] = []
    print(pools.filter((p) { p.id > 1 }).len)
    print(pools.any((p) { p.id > 1 }), pools.all((p) { p.id > 1 }))
    print(pools.map((p) { p.id }).len)
    print([1].map((i) { Pool(id: i) }).len)
}
",
    );
    assert_eq!(errs, vec!["`filter` would copy the items of `pools`, which own an Arena; use a `for` loop with `remove`"]);
}

#[test]
fn a_function_value_in_a_pure_function_must_be_pure() {
    let errs = errors(
        "noisy(n: int) -> bool { print(n)\n true }
pure fine(n: int) -> bool { n > 1 }
main() { }

pure a(xs: list[int], f: (int) -> bool, g: pure (int) -> bool) -> int {
    xs.filter(g).len + xs.filter(fine).len + xs.filter((e) { e > 1 }).len + xs.filter(f).len + xs.filter(noisy).len
}
",
    );
    assert_eq!(
        errs,
        vec![
            "`f` isn't a pure function type, so a `pure` function can't call it; declare the parameter as `pure (int) -> bool`",
            "`noisy` isn't `pure`, so a `pure` function can't call it"
        ]
    );
}

#[test]
fn an_impure_literal_makes_the_closure_around_it_impure() {
    let errs = errors(
        "need(f: pure (int) -> bool) -> bool { f(1) }

main() {
    let xs = [1, 2]
    need((x) { xs.any((e) { e > x }) })
    need((x) {
        xs.any((e) {
            print(e)
            e > x
        })
    })
}
",
    );
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].starts_with("this closure isn't pure"), "{errs:?}");
}

#[test]
fn a_discarded_call_does_nothing() {
    let errs = errors(
        "main() {
    let xs = [1, 2]
    xs.any((e) { e > 1 })
    xs.map((e) { e })
}
",
    );
    assert_eq!(errs, vec!["this only reads the list, so the line does nothing"; 2]);
}

#[test]
fn the_wrong_arguments_are_reported() {
    let errs = errors(
        "main() {
    let xs = [1, 2]
    let f = (n: int) -> bool { n > 1 }
    print(xs.filter())
    print(xs.filter(1, 2))
    print(xs.filter(f, 3))
}
",
    );
    assert_eq!(errs, vec!["`filter` takes one function"; 3]);
}
