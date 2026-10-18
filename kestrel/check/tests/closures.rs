//! Closure literals in the checker: types, frames, purity, and who owns a closure's locals.

use kestrel_check::{check, ClosureInfo, Tables};
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

fn local_ty(t: &Tables, name: &str) -> String {
    t.ty_name(t.locals.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no local {name}")).ty)
}

/// The closures, in the order of their ids (their order in the source).
fn closures(t: &Tables) -> Vec<&ClosureInfo> {
    let mut ids: Vec<_> = t.closures.keys().copied().collect();
    ids.sort();
    ids.iter().map(|id| &t.closures[id]).collect()
}

#[test]
fn a_closure_has_a_function_type_and_is_pure_when_its_body_is() {
    let t = ok("main() {\n    let sq = (x: int) -> int { x * x }\n    let hurt = (n: int) { print(n) }\n    let both = (a: int, var b: float) { b += 1.0 }\n}\n");
    assert_eq!(local_ty(&t, "sq"), "pure (int) -> int");
    assert_eq!(local_ty(&t, "hurt"), "(int)");
    assert_eq!(local_ty(&t, "both"), "pure (int, var float)");
    let cs = closures(&t);
    assert_eq!(cs.iter().map(|c| c.is_pure).collect::<Vec<_>>(), vec![true, false, true]);
    assert!(cs.iter().all(|c| c.captures.is_empty()));
    // Changing its own `var` parameter is the closure's, not the function's.
    assert_eq!(cs.iter().map(|c| c.mutates_params).collect::<Vec<_>>(), vec![false, false, true]);
}

#[test]
fn the_result_is_inferred_from_the_tail_and_the_returns() {
    let t = ok("main() {\n    let a = (x: int) { x + 1 }\n    let b = (x: float) {\n        if x < 0.0 { return 0.0 }\n        x\n    }\n    let c = (x: int) { return x }\n    let d = () { }\n}\n");
    assert_eq!(local_ty(&t, "a"), "pure (int) -> int");
    assert_eq!(local_ty(&t, "b"), "pure (float) -> float");
    assert_eq!(local_ty(&t, "c"), "pure (int) -> int");
    assert_eq!(local_ty(&t, "d"), "pure ()");
    let errs = errors("main() {\n    let a = (x: int) {\n        if x > 0 { return 1 }\n        2.5\n    }\n}\n");
    assert_eq!(errs, vec!["expected int, found float"]);
}

#[test]
fn an_if_statement_at_the_end_gives_no_value() {
    ok("main() {\n    let a = (x: int) { if x > 0 { print(x) } }\n    let b = (x: int) { if x > 0 { print(x) } else { print(0) } }\n}\n");
}

#[test]
fn an_expected_function_type_gives_the_parameters_and_the_result() {
    let t = ok("apply(f: (int) -> float, x: int) -> float { f(x) }\nmain() {\n    let r = apply((n) { n * 2 }, 4)\n}\n");
    let cs = closures(&t);
    // The `int` tail converts to the expected `float`; the closure is typed by what it was given.
    assert_eq!(t.sig_name(cs[0].sig), "pure (int) -> float");
}

#[test]
fn locals_belong_to_the_closure_and_not_to_the_function() {
    let t = ok("main() {\n    let before = 1\n    let f = (a: int, b: int) -> int {\n        let sum = a + b\n        sum\n    }\n    let after = f(1, 2)\n}\n");
    let c = closures(&t)[0];
    let names: Vec<&str> = c.locals.iter().map(|&l| t.local(l).name.as_str()).collect();
    assert_eq!(names, vec!["a", "b", "sum"]);
    assert!(c.locals.iter().all(|&l| t.local(l).closure.is_some() && t.local(l).owner == c.encl_fn));
    let main = t.func(t.main.unwrap());
    let main_names: Vec<&str> = main.locals.iter().map(|&l| t.local(l).name.as_str()).collect();
    assert_eq!(main_names, vec!["before", "f", "after"]);
    assert!(main.locals.iter().all(|&l| t.local(l).closure.is_none()));
    assert!(c.locals[..2].iter().all(|&l| t.local(l).is_param));
}

#[test]
fn a_field_default_and_a_constant_own_their_closures_locals() {
    let t = ok("struct Button {\n    on_click: (int) = (n) { let m = n + 1 }\n}\nlet DOUBLE = (x: int) -> int { x * 2 }\nmain() { }\n");
    for c in closures(&t) {
        assert_eq!(c.encl_fn, None);
        assert!(c.locals.iter().all(|&l| t.local(l).owner.is_none() && t.local(l).closure.is_some()));
    }
    assert_eq!(closures(&t).len(), 2);
}

#[test]
fn print_and_impure_calls_make_the_closure_impure_and_not_the_function() {
    let t = ok("shout(n: int) { print(n) }\nmain() {\n    let a = (n: int) { print(n) }\n    let b = (n: int) { shout(n) }\n    let c = (n: int) -> int { randi_range(0, n) }\n}\n");
    let cs = closures(&t);
    assert_eq!(cs.iter().map(|c| c.is_pure).collect::<Vec<_>>(), vec![false, false, false]);
    // `main` itself prints nothing and calls nothing impure outside the closures.
    let main = t.func(t.main.unwrap());
    assert!(!main.io && !main.mutates_params);
    // The `print` inside `shout` still marks `shout`.
    assert!(t.func(t.fn_by_name["shout"]).io);
}

#[test]
fn purity_is_required_by_pure_functions_par_bodies_and_pure_function_types() {
    let errs = errors("pure f() -> int {\n    let g = () { print(1) }\n    1\n}\nmain() { }\n");
    assert_eq!(errs, vec!["this closure calls `print`, which a `pure` function can't do"]);
    let errs = errors("pure f() -> int {\n    let g = () { randf() }\n    1\n}\nmain() { }\n");
    assert_eq!(errs, vec!["this closure uses random numbers, which a `pure` function can't do"]);
    let errs = errors("run(f: pure (int) -> int) -> int { f(1) }\nmain() {\n    let r = run((x) {\n        print(x)\n        x\n    })\n}\n");
    assert_eq!(errs, vec!["this closure isn't pure (it calls `print`), but `f` needs a pure one"]);
    // A pure closure goes where a non-pure type is expected.
    ok("run(f: (int) -> int) -> int { f(1) }\nmain() {\n    let r = run((x) { x })\n}\n");
}

#[test]
fn a_closure_calling_a_function_value_depends_on_its_type() {
    let consts = "let PURE_F: pure (int) -> int = (x) { x }\nlet PLAIN: (int) -> int = (x) { x }\n";
    let t = ok(&format!("{consts}main() {{\n    let a = (n: int) -> int {{ PURE_F(n) }}\n    let b = (n: int) -> int {{ PLAIN(n) }}\n}}\n"));
    assert!(local_ty(&t, "a").starts_with("pure"));
    assert!(!local_ty(&t, "b").starts_with("pure"));
    let errs = errors(&format!("{consts}pure f() -> int {{\n    let b = (n: int) -> int {{ PLAIN(n) }}\n    1\n}}\nmain() {{ }}\n"));
    assert_eq!(
        errs,
        vec!["this closure calls `PLAIN`, which isn't a pure function type; a `pure` function can only call `pure` functions"]
    );
}

#[test]
fn parameter_types_must_be_given_where_nothing_expects_a_function() {
    assert_eq!(
        errors("main() {\n    let f = (x, y: int) { y }\n}\n"),
        vec!["give `x` a type: `(x: int) { ... }`; Kestrel can only leave it out where a function type is expected"]
    );
    assert_eq!(
        errors("main() {\n    let f: (int) -> int = (a, b) { a }\n}\n"),
        vec!["this closure takes 2 parameters, but `(int) -> int` takes 1 parameter"]
    );
    assert_eq!(
        errors("main() {\n    let f: (int) -> int = () { 1 }\n}\n"),
        vec!["this closure takes no parameters, but `(int) -> int` takes 1 parameter"]
    );
}

#[test]
fn naming_a_local_from_outside_is_refused_for_now() {
    assert_eq!(
        errors("main() {\n    let n = 1\n    let f = () { n + 1 }\n}\n"),
        vec!["closures can't capture `n` yet"]
    );
    // A closure's own locals and parameters, and the module's functions and constants, are not captures.
    ok("let K = 3\nsq(x: int) -> int { x * x }\nmain() {\n    let f = (a: int) -> int {\n        let b = a + K\n        sq(b)\n    }\n}\n");
    // The refusal reaches into closures inside closures.
    assert_eq!(
        errors("main() {\n    let n = 1\n    let f = () { () { n } }\n}\n"),
        vec!["closures can't capture `n` yet"]
    );
}

#[test]
fn jumps_cannot_leave_a_closure() {
    let errs = errors("main() {\n    while true {\n        let f = () { break }\n        let g = () { continue }\n    }\n}\n");
    assert_eq!(
        errs,
        vec![
            "`break` can't leave this closure; it is a function of its own",
            "`continue` can't leave this closure; it is a function of its own"
        ]
    );
    // A loop of its own inside the closure is fine, and `break` outside any loop reads as before.
    ok("main() {\n    while true {\n        let f = () { while true { break } }\n        break\n    }\n}\n");
    assert_eq!(errors("main() {\n    let f = () { break }\n}\n"), vec!["`break` outside a loop"]);
}

#[test]
fn return_leaves_the_closure_even_in_a_par_for() {
    ok("main() {\n    var xs: list[int] = [1, 2]\n    par for x in xs {\n        let f = (n: int) -> int { return n }\n        x = f(x)\n    }\n}\n");
    assert_eq!(
        errors("main() {\n    var xs: list[int] = [1, 2]\n    par for x in xs {\n        return\n    }\n}\n"),
        vec!["`return` can't leave a `par for` body"]
    );
}

#[test]
fn a_closure_that_captures_nothing_points_into_no_arena() {
    ok("make() -> (int) -> int {\n    arena {\n        return (x) { x + 1 }\n    }\n}\nmain() { }\n");
}

#[test]
fn a_closure_statement_does_nothing() {
    assert_eq!(
        errors("main() {\n    (x: int) { x }\n}\n"),
        vec!["this line has no effect; did you mean to continue the line above?"]
    );
}

#[test]
fn constants_are_checked_after_the_constants_a_closure_body_names() {
    ok("let F = (x: int) -> int { G + x }\nlet G = 5\nmain() { }\n");
}
