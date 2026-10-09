use kestrel_syntax::ast::*;
use kestrel_syntax::parse;

fn ok(src: &str) -> Program {
    let (p, d) = parse(src);
    let msgs: Vec<&String> = d.iter().map(|d| &d.message).collect();
    assert!(d.is_empty(), "unexpected diagnostics: {msgs:?}");
    p
}

fn errs(src: &str) -> Vec<String> {
    parse(src).1.into_iter().map(|d| d.message).collect()
}

fn fn_decl(p: &Program, i: usize) -> &FnDecl {
    match &p.items[i] {
        Item::Fn(f) => f,
        other => panic!("expected fn, got {other:?}"),
    }
}

/// Parses `src` as the body of `main` and returns the body as an s-expression.
fn body(src: &str) -> String {
    let p = ok(&format!("main() {{\n{src}\n}}"));
    fn_decl(&p, 0).body.to_sexpr()
}

#[test]
fn parses_fib() {
    let p = ok("pure fib(n: int) -> int {\n    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }\n}\n\nmain() {\n    print(fib(10))\n}\n");
    assert_eq!(p.items.len(), 2);
    let fib = fn_decl(&p, 0);
    assert!(fib.is_pure);
    assert_eq!(fib.params.len(), 1);
    assert!(fib.ret.is_some());
    assert_eq!(
        fib.body.to_sexpr(),
        "{(if (< n 2) {n} else {(+ (call fib (- n 1)) (call fib (- n 2)))})}"
    );
}

#[test]
fn precedence() {
    assert_eq!(body("1 + 2 * 3 - 4"), "{(- (+ 1 (* 2 3)) 4)}");
    assert_eq!(body("not a and b or c"), "{(or (and (not a) b) c)}");
    assert_eq!(body("-x * 2 == y"), "{(== (* (- x) 2) y)}");
    assert_eq!(body("(1 + 2) * 3"), "{(* (+ 1 2) 3)}");
}

#[test]
fn calls_with_labels_and_var() {
    assert_eq!(body("f(a, b: 2, var c)"), "{(call f a b: 2 var c)}");
    assert_eq!(body("f()"), "{(call f)}");
}

#[test]
fn else_if_chains_nest() {
    assert_eq!(
        body("if a { 1 } else if b { 2 } else { 3 }"),
        "{(if a {1} else {(if b {2} else {3})})}"
    );
}

#[test]
fn statements() {
    assert_eq!(
        body("let x = 1\nvar y: int = 2\ny += x\nwhile y < 10 { y = y * 2 }\nfor i from 0 to 3 { break }\nreturn"),
        "{(let x 1); (var y 2); (+= y x); (while (< y 10) {(= y (* y 2))}); (for i 0 3 {break}); return}"
    );
    assert_eq!(body("let a = 1; let b = 2"), "{(let a 1); (let b 2)}");
}

#[test]
fn continuation_and_else_on_next_line() {
    assert_eq!(body("let x = 1 +\n    2"), "{(let x (+ 1 2))}");
    assert_eq!(body("if a {\n 1\n}\nelse {\n 2\n}"), "{(if a {1} else {2})}");
}

#[test]
fn types() {
    let p = ok("f(a: [int], b: list[vec3], c: map[string, int]?, var d: float) {}\n");
    let f = fn_decl(&p, 0);
    assert!(matches!(f.params[0].ty.kind, TypeKind::Array(_)));
    match &f.params[1].ty.kind {
        TypeKind::Named { name, args } => {
            assert_eq!(name, "list");
            assert_eq!(args.len(), 1);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(f.params[2].ty.kind, TypeKind::Optional(_)));
    assert!(f.params[3].is_var);
}

#[test]
fn top_level_constant() {
    let p = ok("let SIZE = 32\nmain() {}\n");
    assert!(matches!(&p.items[0], Item::Const(c) if c.bind.name == "SIZE"));
}

#[test]
fn ids_are_dense() {
    let p = ok("main() { let a = 1 + 2 }\n");
    assert_eq!(p.expr_count, 3);
    assert_eq!(p.bind_count, 1);
}

#[test]
fn helpful_errors_and_recovery() {
    assert_eq!(errs("fn main() {}"), vec!["Kestrel functions don't use `fn`: write `name(params) { ... }`"]);
    assert_eq!(
        errs("main() {\n let = 5\n let y =\n}"),
        vec!["expected a variable name, found `=`", "expected an expression, found `}`"]
    );
    assert_eq!(errs("main() { a < b < c }"), vec!["comparisons can't be chained; combine them with `and`"]);
    assert_eq!(errs("main() {\n for x in xs { }\n}"), vec!["for-in loops aren't supported yet"]);
    assert_eq!(errs("struct A {}\nmain() {}"), vec!["`struct` and `enum` aren't supported yet"]);
    assert_eq!(errs("main() { a b }"), vec!["expected end of line, found name `b`"]);
}
