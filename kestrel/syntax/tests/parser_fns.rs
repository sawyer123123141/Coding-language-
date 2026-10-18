//! Parsing function types, calls of values and closure literals.

use kestrel_syntax::ast::*;

mod common;
use common::*;

/// The first parameter's type of `f`, as written back.
fn param_type(ty: &str) -> String {
    let p = ok(&format!("f(g: {ty}) {{}}\nmain() {{}}\n"));
    fn_decl(&p, 0).params[0].ty.to_sexpr()
}

#[test]
fn function_types_in_type_position() {
    assert_eq!(param_type("(int, var vec2) -> bool"), "(int, var vec2) -> bool");
    assert_eq!(param_type("pure (int) -> int"), "pure (int) -> int");
    assert_eq!(param_type("()"), "()");
    assert_eq!(param_type("(int)"), "(int)");
    assert_eq!(param_type("list[(int)]"), "list[(int)]");
    assert_eq!(param_type("[(int) -> int]"), "[(int) -> int]");
    // A function that gives a function, and one that takes one.
    assert_eq!(param_type("(int) -> (int) -> int"), "(int) -> (int) -> int");
    assert_eq!(param_type("((int) -> bool) -> int"), "((int) -> bool) -> int");
    // `?` binds to the result of an arrow type, and to the whole of a type with no arrow.
    assert_eq!(param_type("(int) -> int?"), "(int) -> int?");
    assert_eq!(param_type("(int)?"), "(int)?");
}

#[test]
fn function_types_in_the_other_type_places() {
    let p = ok("struct S {\n    cb: (int) -> int\n    on_hit: (int)\n}\nmake() -> (int) -> int { f }\nmain() {\n    let g: pure (int) -> int = f\n    let xs = list[(int) -> int]()\n}\n");
    let Item::Struct(s) = &p.items[0] else { panic!("a struct") };
    assert_eq!(s.fields[0].ty.to_sexpr(), "(int) -> int");
    assert_eq!(s.fields[1].ty.to_sexpr(), "(int)");
    assert_eq!(fn_decl(&p, 1).ret.as_ref().unwrap().to_sexpr(), "(int) -> int");
    assert_eq!(fn_decl(&p, 2).body.to_sexpr(), "{(let g f); (let xs (make list[(int) -> int]))}");
}

#[test]
fn function_type_errors() {
    assert_eq!(errs("f(g: (int) ->) {}\nmain() {}\n"), vec!["expected a type, found `)`"]);
}

#[test]
fn a_postfix_call_is_a_call_of_a_value() {
    assert_eq!(body("handlers[i](3)"), "{(callv (index handlers i) 3)}");
    assert_eq!(body("handlers[i](3).len"), "{(. (callv (index handlers i) 3) len)}");
    assert_eq!(body("make_adder(2)(5)"), "{(callv (call make_adder 2) 5)}");
    assert_eq!(body("(f)(var p, 0.5)"), "{(callv f var p 0.5)}");
    assert_eq!(body("b.on_click()"), "{(.call b on_click)}");
    assert_eq!(body("a.b(1)(2)"), "{(callv (.call a b 1) 2)}");
    assert_eq!(body("f(1)(2)(3)"), "{(callv (callv (call f 1) 2) 3)}");
    // The call has to start on the same line as its callee.
    assert_eq!(body("f\n(1)"), "{f; 1}");
}

#[test]
fn closure_literals() {
    assert_eq!(body("(x: int) { x }"), "{(closure (x: int) {x})}");
    assert_eq!(body("let f = (x: int, var y) -> int { x }"), "{(let f (closure (x: int, var y) -> int {x}))}");
    assert_eq!(body("let f = () { }"), "{(let f (closure () {}))}");
    assert_eq!(body("xs.filter((e) { e.hp > 0 })"), "{(.call xs filter (closure (e) {(> (. e hp) 0)}))}");
    assert_eq!(body("let f = () -> (int) -> int { g }"), "{(let f (closure () -> (int) -> int {g}))}");
    assert_eq!(body("let f = (g: (int) -> int) { g(1) }"), "{(let f (closure (g: (int) -> int) {(call g 1)}))}");
    // A closure followed by `(3)` on the same line is called.
    assert_eq!(body("(x: int) { x }(3)"), "{(callv (closure (x: int) {x}) 3)}");
    assert_eq!(body("let t = [(x) { x }, (y) { y }]"), "{(let t [(closure (x) {x}) (closure (y) {y})])}");
    // A closure inside a closure.
    assert_eq!(body("let f = () { () { 1 } }"), "{(let f (closure () {(closure () {1})}))}");
}

#[test]
fn a_closure_body_of_several_lines_inside_call_parentheses() {
    let src = "apply((x) {\n    let y = x + 1\n    y * 2\n}, 4)";
    assert_eq!(body(src), "{(call apply (closure (x) {(let y (+ x 1)); (* y 2)}) 4)}");
    // A body of one line that returns.
    assert_eq!(body("f((x: int) -> int { if x < 0 { return 0 }; x })"), "{(call f (closure (x: int) -> int {(if (< x 0) {(return 0)}); x}))}");
}

#[test]
fn parentheses_still_group() {
    assert_eq!(body("(a + b) * c"), "{(* (+ a b) c)}");
    assert_eq!(body("(f)(var p, 0.5)"), "{(callv f var p 0.5)}");
    assert_eq!(body("((1))"), "{1}");
}

#[test]
fn a_parenthesised_head_is_not_a_closure() {
    assert_eq!(body("if (ready) { go() }"), "{(if ready {(call go)})}");
    assert_eq!(body("while (n > 0) { n -= 1 }"), "{(while (> n 0) {(-= n 1)})}");
    assert_eq!(body("for v in (xs) { }"), "{(for-in v xs {})}");
    assert_eq!(body("par for v in (xs) { }"), "{(par-for v xs {})}");
    assert_eq!(body("for i from (a) to (b) { }"), "{(for i a b {})}");
    assert_eq!(body("in (w.mem) { }"), "{(in (. w mem) {})}");
    assert_eq!(body("match (e) { A { } else { } }"), "{(match e [A {}] [else {}])}");
    assert_eq!(body("if (a) and (b) { }"), "{(if (and a b) {})}");
    assert_eq!(body("if a == (b) { }"), "{(if (== a b) {})}");
    assert_eq!(body("let v = if (c) { 1 } else { 2 }"), "{(let v (if c {1} else {2}))}");
    assert_eq!(body("if x { } else if (y) { }"), "{(if x {} else {(if y {})})}");
}

#[test]
fn a_closure_in_a_head_goes_inside_brackets() {
    assert_eq!(
        body("if xs.any((e) { e.dead }) { alarm() }"),
        "{(if (.call xs any (closure (e) {(. e dead)})) {(call alarm)})}"
    );
    assert_eq!(body("while f((x) { x }) { }"), "{(while (call f (closure (x) {x})) {})}");
    assert_eq!(body("for v in make([(x) { x }]) { }"), "{(for-in v (call make [(closure (x) {x})]) {})}");
    assert_eq!(body("for v in m[(x) { x }(1)] { }"), "{(for-in v (index m (callv (closure (x) {x}) 1)) {})}");
    // The head's rule is back inside the arguments' own blocks and the body of a closure.
    assert_eq!(
        body("if f((x) { if (x) { 1 } }) { }"),
        "{(if (call f (closure (x) {(if x {1})})) {})}"
    );
    assert_eq!(
        body("if f((x) { (y) { y } }) { }"),
        "{(if (call f (closure (x) {(closure (y) {y})})) {})}"
    );
}

#[test]
fn closure_parse_errors() {
    assert_eq!(errs("main() { let f = (x: ) { } }"), vec!["expected a type, found `)`"]);
    assert_eq!(errs("main() { let f = (x -> int) { } }"), vec!["expected `)` or `,`, found `->`"]);
    assert_eq!(errs("main() { let f = (self) { } }").len(), 1);
    assert_eq!(errs("main() { let f = (x) -> { } }"), vec!["expected a type, found `{`"]);
}

#[test]
fn closure_parameters_and_types_in_the_tree() {
    let p = ok("main() { let f = (a, var b: int) -> bool { true } }");
    let Stmt::Let { value, .. } = &fn_decl(&p, 0).body.stmts[0] else { panic!("a let") };
    let ExprKind::Closure(c) = &value.kind else { panic!("a closure") };
    assert_eq!(c.params.len(), 2);
    assert!(!c.params[0].is_var && c.params[0].ty.is_none());
    assert!(c.params[1].is_var && c.params[1].ty.is_some());
    assert_eq!(c.ret.as_ref().unwrap().to_sexpr(), "bool");
}

#[test]
fn many_nested_parentheses_are_scanned_once() {
    // Levels of `(` ... `)` (the debug build's 2 MiB test stack allows about 80), none of them a closure.
    let src = format!("{}1{}", "(".repeat(50), ")".repeat(50));
    assert_eq!(body(&src), "{1}");
}
