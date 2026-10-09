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
    assert_eq!(errs("use foo\nmain() {}"), vec!["`use` isn't supported yet"]);
    assert_eq!(errs("main() { a b }"), vec!["expected end of line, found name `b`"]);
}

fn struct_decl(p: &Program, i: usize) -> &StructDecl {
    match &p.items[i] {
        Item::Struct(s) => s,
        other => panic!("expected struct, got {other:?}"),
    }
}

#[test]
fn struct_with_fields_defaults_and_methods() {
    let p = ok("struct Chunk {\n    x: int\n    z: int = 0\n\n    pure area(self) -> int { self.x * self.z }\n    grow(var self, by: int) {\n        self.x += by\n    }\n}\nmain() {}\n");
    let s = struct_decl(&p, 0);
    assert_eq!(s.name, "Chunk");
    assert_eq!(s.fields.len(), 2);
    assert!(s.fields[0].default.is_none());
    assert_eq!(s.fields[1].default.as_ref().unwrap().to_sexpr(), "0");
    assert_eq!(s.methods.len(), 2);
    let area = &s.methods[0];
    assert!(area.is_pure);
    assert!(area.params[0].is_self && !area.params[0].is_var);
    assert_eq!(area.params[0].bind.name, "self");
    assert!(matches!(&area.params[0].ty.kind, TypeKind::Named { name, .. } if name == "Chunk"));
    assert_eq!(area.body.to_sexpr(), "{(* (. self x) (. self z))}");
    let grow = &s.methods[1];
    assert!(grow.params[0].is_self && grow.params[0].is_var);
    assert_eq!(grow.body.to_sexpr(), "{(+= (. self x) by)}");
}

#[test]
fn field_chains_and_method_calls() {
    assert_eq!(body("a.b.c"), "{(. (. a b) c)}");
    assert_eq!(body("p.move(1, dy: 2).x"), "{(. (.call p move 1 dy: 2) x)}");
    assert_eq!(body("-p.x * 2"), "{(* (- (. p x)) 2)}");
    assert_eq!(body("p.x += 1"), "{(+= (. p x) 1)}");
    assert_eq!(body("Point(1, y: 2).x"), "{(. (call Point 1 y: 2) x)}");
    assert_eq!(body("let n = list\n    .size"), "{(let n (. list size))}");
}

#[test]
fn parenthesised_expressions_span_their_parens() {
    let p = ok("main() {\n(1 + 2) * 3\n}\n");
    let Stmt::Expr(e) = &fn_decl(&p, 0).body.stmts[0] else { panic!() };
    let ExprKind::Binary { lhs, .. } = &e.kind else { panic!() };
    assert_eq!((lhs.span.start, lhs.span.end), (9, 16));
}

#[test]
fn struct_errors() {
    assert_eq!(errs("f(self) {}\nmain() {}"), vec!["`self` is only allowed as the first parameter of a method"]);
    assert_eq!(
        errs("struct A {\n    x: int\n    f(a: int, self) {}\n}\nmain() {}"),
        vec!["`self` is only allowed as the first parameter of a method"]
    );
    assert_eq!(errs("struct A {\n    x int\n}\nmain() {}"), vec!["expected `:` and a type, or `(` for a method, found name `int`"]);
}

#[test]
fn enum_with_variants_and_methods() {
    let p = ok("enum Block {\n    Air\n    Dirt\n    Stone { hardness: u8, depth: int }\n    Ore {\n        kind: int\n        value: int\n    }\n\n    pure solid(self) -> bool {\n        match self {\n            Air { false }\n            else { true }\n        }\n    }\n}\nmain() {}\n");
    let Item::Enum(e) = &p.items[0] else { panic!("expected enum") };
    assert_eq!(e.name, "Block");
    let names: Vec<&str> = e.variants.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["Air", "Dirt", "Stone", "Ore"]);
    assert_eq!(e.variants[2].fields.len(), 2);
    assert_eq!(e.variants[3].fields.len(), 2);
    assert_eq!(e.methods.len(), 1);
    assert_eq!(e.methods[0].body.to_sexpr(), "{(match self [Air {false}] [else {true}])}");
}

#[test]
fn match_arms_with_bindings() {
    assert_eq!(
        body("match e {\n    Zombie z { z.hp -= 1 }\n    Skeleton s { s.hp -= 2 }\n}"),
        "{(match e [Zombie z {(-= (. z hp) 1)}] [Skeleton s {(-= (. s hp) 2)}])}"
    );
    assert_eq!(body("let n = match b { Air { 0 } else { 1 } }"), "{(let n (match b [Air {0}] [else {1}]))}");
    assert_eq!(body("let b = Block.Stone(hardness: 3)"), "{(let b (.call Block Stone hardness: 3))}");
}

#[test]
fn enum_errors() {
    assert_eq!(errs("enum A {\n    X { a int }\n}\nmain() {}"), vec!["expected `:` and a type, found name `int`"]);
    assert_eq!(errs("main() {\n match x { 1 { } }\n}"), vec!["expected a variant name or `else`, found number `1`"]);
}

#[test]
fn interpolated_strings() {
    assert_eq!(body("print(\"hp = {hp + 1}!\")"), "{(call print (interp \"hp = \" (+ hp 1) \"!\"))}");
    assert_eq!(body("print(\"{p.x}{p.y}\")"), "{(call print (interp (. p x) (. p y)))}");
    // Spans inside the string point at the source text.
    let (_, d) = parse("main() {\n print(\"x {1 +}\")\n}");
    assert_eq!(d.iter().map(|d| d.message.as_str()).collect::<Vec<_>>(), ["expected an expression, found `}`"]);
    assert_eq!(d[0].span.start, 23, "points at the closing brace");
}
