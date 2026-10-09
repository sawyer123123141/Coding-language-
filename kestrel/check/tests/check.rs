use kestrel_check::{check, Tables, Ty};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn ok(src: &str) -> Tables {
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "unexpected errors: {errs:?}");
    t.expect("tables")
}

fn errors(src: &str) -> Vec<String> {
    run(src).1
}

fn local_ty(t: &Tables, name: &str) -> Ty {
    t.locals.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no local {name}")).ty
}

#[test]
fn statements_and_inference() {
    let t = ok("main() {\n var x = 1\n x += 2\n let y: float = x\n while x < 10 { x = x * 2 }\n for i from 0 to 3 { x -= i }\n let z = if x > 3 { 1 } else { 2 }\n}\n");
    assert_eq!(local_ty(&t, "x"), Ty::Int);
    assert_eq!(local_ty(&t, "y"), Ty::Float);
    assert_eq!(local_ty(&t, "z"), Ty::Int);
    assert_eq!(local_ty(&t, "i"), Ty::Int);
    assert!(t.coerce.values().any(|&ty| ty == Ty::Float), "x should be coerced to float");
}

#[test]
fn literals_take_the_expected_type() {
    let t = ok("main() {\n let a: i64 = 5\n let b: u8 = 255\n let c: f64 = 1.5\n let d: float = 3\n}\n");
    assert_eq!(local_ty(&t, "a"), Ty::I64);
    assert_eq!(local_ty(&t, "b"), Ty::U8);
    assert_eq!(local_ty(&t, "c"), Ty::F64);
    assert_eq!(local_ty(&t, "d"), Ty::Float);
}

#[test]
fn constants() {
    let t = ok("let SIZE = 32\nlet AREA = SIZE * SIZE\nmain() {\n let x = AREA\n}\n");
    assert_eq!(t.consts[1].ty, Ty::Int);
    assert_eq!(local_ty(&t, "x"), Ty::Int);
}

#[test]
fn diverging_branches_satisfy_return_type() {
    ok("f(x: int) -> int {\n if x > 0 { return 1 } else { return 2 }\n}\nmain() {}\n");
    ok("f(x: int) -> int {\n if x > 0 { 1 } else { 2 }\n}\nmain() {}\n");
}

#[test]
fn errors_are_reported() {
    let cases: &[(&str, &str)] = &[
        ("f() {}\n", "program needs a `main()` function"),
        ("main() {\n let x = 1\n if true { let x = 2 }\n}\n", "`x` is already defined; Kestrel doesn't allow shadowing"),
        ("main() {\n let x = 1\n x = 2\n}\n", "can't change `x`: it was declared with `let`"),
        ("main() {\n let x: int = true\n}\n", "expected int, found bool"),
        ("main() {\n let x: int = 1.5\n}\n", "expected int, found float"),
        ("main() {\n let x = 1 + true\n}\n", "`+` needs numbers, found int and bool"),
        ("main() {\n while 1 { }\n}\n", "expected bool, found int"),
        ("main() {\n let x = y\n}\n", "unknown name `y`"),
        ("main() {\n let x = 3000000000\n}\n", "`3000000000` doesn't fit in int"),
        ("main() {\n let b: u8 = 256\n}\n", "`256` doesn't fit in u8"),
        ("main() {\n break\n}\n", "`break` outside a loop"),
        ("main() {\n var total = 1\n total = total\n - 2\n}\n", "this line has no effect; did you mean to continue the line above?"),
        ("main() {\n let x = if true { 1 }\n}\n", "this `if` needs an `else` to produce a value"),
        ("f() -> int { }\nmain() {}\n", "`f` must return int, but its body can finish without a value"),
        ("let A = B\nlet B = 1\nmain() {}\n", "unknown name `B`"),
        ("f(x: vec9) {}\nmain() {}\n", "unknown type `vec9`"),
        ("main() {\n let s = \"hi\"\n}\n", "string values aren't supported yet (only as `print` arguments)"),
        ("pure f(var x: int) {}\nmain() {}\n", "pure functions can't take `var` parameters"),
        ("f(x: int) {\n x = 1\n}\nmain() {}\n", "can't change parameter `x`"),
        ("main() {\n let u: u8 = 1\n let n = -u\n}\n", "can't negate a u8"),
        ("main() {}\nmain() {}\n", "`main` is defined twice"),
        ("print() {}\nmain() {}\n", "`print` is a built-in name; pick another"),
    ];
    for (src, expected) in cases {
        let errs = errors(src);
        assert_eq!(errs, vec![expected.to_string()], "source:\n{src}");
    }
}

#[test]
fn assigning_var_param_marks_function() {
    let t = ok("bump(var x: int) {\n x += 1\n}\nmain() {}\n");
    assert!(t.fns[0].mutates_params);
}
