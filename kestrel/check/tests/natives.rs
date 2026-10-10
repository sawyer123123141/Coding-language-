use kestrel_check::{check, Tables};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn errors(src: &str) -> Vec<String> {
    run(src).1
}

const BAD_TYPE: &str = "native functions can only take numbers, bools, strings, vectors, and enums or structs of those";

/// Every shape a native may take and return.
const ALLOWED: &str = "enum Dir {
    North
    East
}

struct Rgba {
    r: float
    g: float
    b: float
    a: float = 1
}

struct Mixed {
    n: int
    big: i64
    byte: u8
    f: f64
    flag: bool
    name: string
    pos: vec2
    at: vec3
}

native take_scalars(x: int, y: i64, z: u8, f: float, g: f64, b: bool, s: string)
native take_values(v: vec2, w: vec3, c: Rgba, m: Mixed, d: Dir) -> int
native make_v2() -> vec2
native make_v3() -> vec3
native make_bool() -> bool
native make_u8() -> u8
native make_i64() -> i64
native make_f64() -> f64
native make_float() -> float
pure native shaped(x: float) -> float

main() {
    take_scalars(1, 2, 3, 1.5, 2.5, true, \"s\")
    print(take_values(vec2(1, 2), vec3(1, 2, 3), Rgba(1, 0, 0), Mixed(1, 2, 3, 4, true, \"n\", vec2(0, 0), vec3(0, 0, 0)), Dir.East))
    print(make_v2(), make_v3(), make_bool(), make_u8(), make_i64(), make_f64(), make_float(), shaped(2))
}
";

#[test]
fn every_allowed_native_type_checks() {
    let (t, e) = run(ALLOWED);
    assert!(e.is_empty(), "{e:?}");
    let t = t.unwrap();
    let native: Vec<&str> = t.fns.iter().filter(|f| f.is_native).map(|f| f.name.as_str()).collect();
    assert_eq!(native.len(), 10);
    assert_eq!(native[0], "take_scalars");
    assert!(!t.fns.iter().find(|f| f.name == "main").unwrap().is_native);
}

#[test]
fn natives_keep_their_parameters() {
    let (t, e) = run("native f(x: int, v: vec2) -> float\nmain() {}\n");
    assert!(e.is_empty(), "{e:?}");
    let t = t.unwrap();
    let f = t.fns.iter().find(|f| f.name == "f").unwrap();
    assert_eq!(f.params.len(), 2);
    assert_eq!(f.locals.len(), 2, "parameters are declared even though there is no body");
}

#[test]
fn a_native_that_is_not_pure_does_io() {
    let (t, e) = run("native f()\npure native g()\nmain() {}\n");
    assert!(e.is_empty(), "{e:?}");
    let t = t.unwrap();
    let io = |name: &str| t.fns.iter().find(|f| f.name == name).unwrap().io;
    assert!(io("f"));
    assert!(!io("g"));
}

#[test]
fn bad_parameter_and_return_types_are_errors() {
    assert_eq!(errors("native f(xs: list[int])\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f(xs: [int])\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f(m: map[string, int])\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f(x: int?)\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f(a: Arena)\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f() -> string\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f() -> [int]\nmain() {}\n"), vec![BAD_TYPE]);
}

#[test]
fn struct_and_enum_parameters_must_hold_only_simple_values() {
    // A struct field that is a list, an array or a user struct isn't allowed.
    assert_eq!(errors("struct S {\n xs: list[int]\n}\nnative f(s: S)\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("struct In {\n n: int\n}\nstruct S {\n i: In\n}\nnative f(s: S)\nmain() {}\n"), vec![BAD_TYPE]);
    // Structs declared after the native are still seen in full.
    assert_eq!(errors("native f(s: S)\nstruct S {\n xs: [int]\n}\nmain() {}\n"), vec![BAD_TYPE]);
    // An enum with fields isn't allowed; one without is.
    assert_eq!(errors("enum E {\n A { n: int }\n B\n}\nnative f(e: E)\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("enum E {\n A\n B\n}\nnative f(e: E)\nmain() {}\n"), Vec::<String>::new());
    // Only vec2 and vec3 may be returned; a user struct or enum may not.
    assert_eq!(errors("struct S {\n n: int\n}\nnative f() -> S\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("enum E {\n A\n}\nnative f() -> E\nmain() {}\n"), vec![BAD_TYPE]);
}

#[test]
fn each_bad_type_is_reported_once_at_its_own_place() {
    let e = errors("native f(a: list[int], b: int, c: [int]) -> string\nmain() {}\n");
    assert_eq!(e, vec![BAD_TYPE, BAD_TYPE, BAD_TYPE]);
}

#[test]
fn natives_cannot_take_var_parameters() {
    assert_eq!(errors("native f(var x: int)\nmain() {}\n"), vec!["native functions can't take `var` parameters"]);
}

#[test]
fn unknown_types_are_reported_once() {
    assert_eq!(errors("native f(x: Nope)\nmain() {}\n"), vec!["unknown type `Nope`"]);
}

#[test]
fn a_pure_function_cannot_call_a_native_that_is_not_pure() {
    let e = errors("native f(x: int) -> int\npure g(x: int) -> int {\n f(x)\n}\nmain() {}\n");
    assert_eq!(e, vec!["pure functions can only call pure functions; `f` isn't pure"]);
    // A `pure` native is fine.
    assert_eq!(errors("pure native f(x: int) -> int\npure g(x: int) -> int {\n f(x)\n}\nmain() {}\n"), Vec::<String>::new());
}

#[test]
fn a_par_for_body_cannot_call_a_native_that_is_not_pure() {
    let src = "native f(x: int) -> int\nmain() {\n var xs = [1, 2, 3]\n par for x in xs {\n  x = f(x)\n }\n}\n";
    assert_eq!(errors(src), vec!["`par for` bodies can only call `pure` functions; `f` isn't `pure`"]);
    let ok = "pure native f(x: int) -> int\nmain() {\n var xs = [1, 2, 3]\n par for x in xs {\n  x = f(x)\n }\n}\n";
    assert_eq!(errors(ok), Vec::<String>::new());
}

#[test]
fn calls_to_natives_are_checked_like_any_call() {
    let e = errors("native f(x: int) -> int\nmain() {\n let y: string = f(1)\n f(\"no\")\n}\n");
    assert_eq!(e, vec!["expected string, found int", "expected int, found string"]);
}
