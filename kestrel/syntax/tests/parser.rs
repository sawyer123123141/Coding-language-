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
fn par_for() {
    assert_eq!(body("par for c in world {\n c.x = 1\n}"), "{(par-for c world {(= (. c x) 1)})}");
    assert_eq!(body("par for i, c in world {}"), "{(par-for i c world {})}");
    assert_eq!(errs("main() {\n par for i from 0 to 3 {}\n}\n")[0], "`par for` needs items to work on; loop over an array or list");
    assert_eq!(errs("main() {\n par x\n}\n")[0], "expected `for` after `par`, found name `x`");
}

#[test]
fn map_literals() {
    assert_eq!(body("let m = {}"), "{(let m (map))}");
    assert_eq!(body("let m = {\"a\": 1, \"b\": x + 1}"), "{(let m (map (\"a\" 1) (\"b\" (+ x 1))))}");
    assert_eq!(body("let m = {\n    \"a\": 1,\n    \"b\": 2,\n}\nf(m)"), "{(let m (map (\"a\" 1) (\"b\" 2))); (call f m)}");
    assert_eq!(body("let m = {\n}"), "{(let m (map))}");
    assert_eq!(errs("main() {\n let m = {1 2}\n}\n")[0], "expected `:`, found number `2`");
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
    assert_eq!(errs("pub use foo\nmain() {}"), vec!["`use` can't be `pub`"]);
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

#[test]
fn arrays_lists_and_indexing() {
    assert_eq!(body("let a = [1, 2,\n 3]"), "{(let a [1 2 3])}");
    assert_eq!(body("let e: [int] = []"), "{(let e [])}");
    assert_eq!(body("xs[i + 1].hp -= 1"), "{(-= (. (index xs (+ i 1)) hp) 1)}");
    assert_eq!(body("let g = grid[y][x]"), "{(let g (index (index grid y) x))}");
    assert_eq!(body("let h = array[int](n * 2)"), "{(let h (make array[int] (* n 2)))}");
    assert_eq!(body("var w = list[Chunk]()"), "{(var w (make list[Chunk]))}");
    assert_eq!(body("let m = map[string, [int]]()"), "{(let m (make map[string, [int]]))}");
    assert_eq!(body("world.add(Chunk(x: 1))"), "{(.call world add (call Chunk x: 1))}");
}

#[test]
fn for_in_loops() {
    assert_eq!(body("for v in xs { print(v) }"), "{(for-in v xs {(call print v)})}");
    assert_eq!(body("for i, v in xs { }"), "{(for-in i v xs {})}");
    assert_eq!(body("for i from 0 to 3 { }"), "{(for i 0 3 {})}");
}

#[test]
fn collection_errors() {
    assert_eq!(errs("main() {\n let a = array[int]\n}"), vec!["expected `(` to create the array, like `array[...]()`"]);
    assert_eq!(errs("main() {\n let a = [1, 2\n}"), vec!["expected `]` or `,`, found `}`"]);
}

#[test]
fn arena_and_in_blocks() {
    assert_eq!(body("arena {\n var t = list[vec3]()\n}"), "{(arena {(var t (make list[vec3]))})}");
    assert_eq!(body("in self.mem { self.h = [] }"), "{(in (. self mem) {(= (. self h) [])})}");
}

#[test]
fn optionals() {
    assert_eq!(body("let t = load(1) else { return }"), "{(let t (call load 1) else {return})}");
    assert_eq!(body("var x: int? = none"), "{(var x none)}");
    let p = ok("f(a: Chunk?) {}\n");
    assert!(matches!(fn_decl(&p, 0).params[0].ty.kind, TypeKind::Optional(_)));
}

#[test]
fn use_declarations() {
    let p = ok("use math\nuse { noise2, lerp from math\n    vectors, colors\n}\nmain() {}\n");
    let Item::Use(u) = &p.items[0] else { panic!() };
    assert_eq!((u.clauses[0].module.as_str(), u.clauses[0].names.len()), ("math", 0));
    let Item::Use(u) = &p.items[1] else { panic!() };
    let names: Vec<&str> = u.clauses[0].names.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!((u.clauses[0].module.as_str(), names), ("math", vec!["noise2", "lerp"]));
    let modules: Vec<&str> = u.clauses[1..].iter().map(|c| c.module.as_str()).collect();
    assert_eq!(modules, ["vectors", "colors"]);
}

#[test]
fn native_functions_have_no_body() {
    let p = ok("pub native f(x: int) -> int\npure native g()\nnative h(a: string, b: vec2)\nmain() {}\n");
    let f = fn_decl(&p, 0);
    assert!(f.is_native && f.is_pub && !f.is_pure);
    assert!(f.ret.is_some() && f.params.len() == 1);
    assert!(f.body.stmts.is_empty());
    let g = fn_decl(&p, 1);
    assert!(g.is_native && g.is_pure && !g.is_pub);
    let h = fn_decl(&p, 2);
    assert!(h.is_native && !h.is_pure && h.params.len() == 2 && h.ret.is_none());
    assert!(!fn_decl(&p, 3).is_native, "ordinary functions are not native");
    // The empty body sits at the end of the signature, inside the declaration's span.
    assert_eq!((f.body.span.start, f.body.span.end), (f.span.end, f.span.end));
    assert_eq!(f.span.end as usize, "pub native f(x: int) -> int".len());
}

#[test]
fn native_errors() {
    assert_eq!(errs("struct A {\n    x: int\n    native f()\n}\nmain() {}"), vec!["native functions can't be methods"]);
    assert_eq!(errs("struct A {\n    pub pure native f(self)\n}\nmain() {}"), vec!["native functions can't be methods"]);
    assert_eq!(errs("enum E {\n    X\n    native f()\n}\nmain() {}"), vec!["native functions can't be methods"]);
    assert_eq!(errs("native h() { }\nmain() {}"), vec!["native functions have no body; remove the `{ }`"]);
    assert_eq!(errs("native h() -> int {\n    1\n}\nmain() {}"), vec!["native functions have no body; remove the `{ }`"]);
    assert_eq!(errs("native\nmain() {}"), vec!["expected a function name, found end of line"]);
}

/// The message and help of each parse error.
fn errs_help(src: &str) -> Vec<(String, Option<String>)> {
    parse(src).1.into_iter().map(|d| (d.message, d.help)).collect()
}

#[test]
fn a_value_in_the_type_place_says_where_it_goes() {
    let msg = "a type goes after `:`, and the value after `=`".to_string();
    let with = |help: &str| vec![(msg.clone(), Some(help.to_string()))];
    assert_eq!(errs_help("main() {\n    let velocity: vec3()\n}\n"), with("write `let velocity: vec3 = vec3()`"));
    assert_eq!(errs_help("main() {\n    var xs: list[int]()\n}\n"), with("write `var xs: list[int] = list[int]()`"));
    assert_eq!(errs_help("struct Ball {\n    velocity: vec3()\n}\nmain() {}\n"), with("write `velocity: vec3 = vec3()`"));
    assert_eq!(errs_help("let ORIGIN: vec3()\nmain() {}\n"), with("write `let ORIGIN: vec3 = vec3()`"));
    // A built-in scalar type has no such value, and a type that isn't a name can't be called.
    for src in ["main() {\n    let x: int(5)\n}\n", "main() {\n    let s: string()\n}\n", "let N: float(1)\nmain() {}\n", "main() {\n    let a: [int](3)\n}\n"] {
        assert_eq!(errs_help(src), vec![(msg.clone(), None)], "{src}");
    }
}

#[test]
fn nesting_too_deep_is_one_error_and_the_rest_of_the_file_is_skipped() {
    let too_deep = "this expression nests more than 1,000 levels deep; split it into smaller parts with `let`";
    let sum = |terms: usize| format!("main() {{\n    let a = )\n    print({})\n    let b = )\n}}\n", vec!["1"; terms].join(" + "));
    // The error before it stays; the one after it isn't reported.
    assert_eq!(errs(&sum(1001)), vec!["expected an expression, found `)`", too_deep]);
    assert_eq!(errs(&sum(1000)), vec!["expected an expression, found `)`", "expected an expression, found `)`"]);
    let ty = |levels: usize| format!("f(x: {}int{}) {{}}\nmain() {{}}\n", "[".repeat(levels), "]".repeat(levels));
    assert_eq!(errs(&ty(1001)), vec!["this type nests more than 1,000 levels deep; simplify it"]);
    assert_eq!(errs(&ty(1000)), Vec::<String>::new());
}
