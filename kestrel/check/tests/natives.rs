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

const BAD_TYPE: &str = "native functions can only take numbers, bools, strings, vectors, arrays of plain values, and enums or structs of those";

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
    assert_eq!(errors("native f(xs: [string])\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f(m: map[string, int])\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f(x: int?)\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f(a: Arena)\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f() -> [int]\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f() -> map[string, int]\nmain() {}\n"), vec![BAD_TYPE]);
}

#[test]
fn struct_and_enum_parameters_must_hold_only_simple_values() {
    // A struct field that is an array or list of strings, or a struct holding a string array, isn't allowed.
    assert_eq!(errors("struct S {\n xs: list[string]\n}\nnative f(s: S)\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("struct In {\n xs: [string]\n}\nstruct S {\n i: In\n}\nnative f(s: S)\nmain() {}\n"), vec![BAD_TYPE]);
    // Structs declared after the native are still seen in full.
    assert_eq!(errors("native f(s: S)\nstruct S {\n xs: [string]\n}\nmain() {}\n"), vec![BAD_TYPE]);
    // An enum with fields isn't allowed; one without is.
    assert_eq!(errors("enum E {\n A { n: int }\n B\n}\nnative f(e: E)\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("enum E {\n A\n B\n}\nnative f(e: E)\nmain() {}\n"), Vec::<String>::new());
    // An enum can't be returned, or be a struct's field (a field holding an array or list of plain values is fine).
    assert_eq!(errors("enum E {\n A\n}\nnative f() -> E\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("enum E {\n A\n}\nstruct S {\n e: E\n}\nnative f() -> S\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("struct S {\n xs: list[int]\n ys: [float]\n}\nnative f(s: S)\nmain() {}\n"), Vec::<String>::new());
}

#[test]
fn each_bad_type_is_reported_once_at_its_own_place() {
    let e = errors("native f(a: list[int], b: int, c: [string]) -> [int]\nmain() {}\n");
    assert_eq!(e, vec![BAD_TYPE, BAD_TYPE, BAD_TYPE]);
}

const ARRAYS_AND_STRUCTS: &str = r#"struct Rgba {
    r: float
    g: float
    b: float
    a: float = 1
}
struct Vertex {
    pos: vec3
    tint: Rgba
}
struct Data {
    xs: list[int] = []
    ps: [vec2] = []
    vs: list[Vertex] = []
    name: string = ""
}
struct Pair {
    n: int
    at: vec3
    ok: bool
    tint: Rgba
}
native sum(vs: [vec3]) -> float
native colors(cs: [Rgba], bytes: [u8], flags: [bool])
native verts(vs: [Vertex]) -> int
native count(d: Data) -> int
native pair(n: int) -> Pair
main() {
    var grown = list[vec3]()
    grown.add(vec3(1, 2, 3))
    print(sum([vec3(1, 2, 3)]), sum(grown))
    colors([Rgba(1, 0, 0)], [1, 2], [true])
    print(verts([Vertex(pos: vec3(0, 0, 0), tint: Rgba(1, 1, 1))]), count(Data()), pair(1))
}
"#;

#[test]
fn arrays_of_plain_values_and_struct_results_check() {
    assert_eq!(errors(ARRAYS_AND_STRUCTS), Vec::<String>::new());
}

#[test]
fn arrays_of_other_values_are_refused() {
    let data = "struct Data {\n xs: list[int] = []\n ps: [vec2] = []\n name: string = \"\"\n}\n";
    let cases = [
        // Strings aren't plain, and neither is an array of arrays.
        "native a(xs: [string])\n".to_string(),
        "native b(xs: [[int]])\n".to_string(),
        // A direct list parameter isn't allowed; only a struct field may be a list.
        "native c(xs: list[int])\n".to_string(),
        "enum Dir {\n North\n}\nnative d(xs: [Dir])\n".to_string(),
        "struct Holder {\n names: [string]\n}\nnative e(h: Holder)\n".to_string(),
        // A struct result can't hold an array or list, or a string.
        format!("{data}native f() -> Data\n"),
        "struct Named {\n s: string\n}\nnative g() -> Named\n".to_string(),
    ];
    for case in cases {
        assert_eq!(errors(&format!("{case}main() {{}}\n")), vec![BAD_TYPE], "{case}");
    }
}

#[test]
fn a_list_is_passed_to_a_natives_array_but_not_to_a_function() {
    let program = |decls: &str, call: &str| format!("{decls}main() {{\n var grown = list[vec3]()\n grown.add(vec3(1, 2, 3))\n var ints = list[int]()\n ints.add(1)\n print({call})\n}}\n");
    let native = "native sum(vs: [vec3]) -> float\n";
    assert_eq!(errors(&program(native, "sum(grown)")), Vec::<String>::new());
    // A Kestrel function's `[T]` parameter still refuses a list.
    let function = "total(vs: [vec3]) -> float { 0.0 }\n";
    assert_eq!(errors(&program(function, "total(grown)")), ["expected [vec3], found list[vec3]"]);
    // The element type has to match.
    assert_eq!(errors(&program(native, "sum(ints)")), ["expected [vec3], found list[int]"]);
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

const SCRATCH: &str = "native show(s: string, n: int)
native peek(n: int) -> int
tag(n: int) -> string { return \"tag {n}\" }
";

#[test]
fn text_built_for_a_native_call_is_freed_at_the_call_so_it_cannot_be_kept() {
    // `show` has an interpolated argument, so everything its arguments make is freed when it returns.
    let src = format!("{SCRATCH}main() {{\n var kept = \"\"\n let n = 1\n show(\"a{{n}}\", if n > 0 {{ kept = tag(n); 1 }} else {{ 2 }})\n print(kept)\n}}\n");
    assert_eq!(errors(&src), ["this memory is freed when the call to `show` returns, but `kept` lives longer"]);
}

#[test]
fn a_native_call_with_no_interpolated_argument_frees_nothing() {
    // The same stores are fine when no argument is an interpolated string: no scratch arena exists.
    let src = format!(
        "{SCRATCH}main() {{\n var kept = \"\"\n let n = 1\n show(\"plain\", if n > 0 {{ kept = tag(n); 1 }} else {{ 2 }})\n show(kept, peek(n))\n}}\n"
    );
    assert_eq!(errors(&src), Vec::<String>::new());
}

#[test]
fn memory_declared_inside_the_arguments_may_be_used_there() {
    let src = format!("{SCRATCH}main() {{\n let n = 1\n show(\"a{{n}}\", if n > 0 {{ let t = tag(n); peek(n) }} else {{ 2 }})\n}}\n");
    assert_eq!(errors(&src), Vec::<String>::new());
}

#[test]
fn leaving_a_function_from_a_native_argument_cannot_return_memory_made_there() {
    let src = format!("{SCRATCH}f(n: int) -> string {{\n show(\"a{{n}}\", if n > 0 {{ return tag(n) }} else {{ 2 }})\n return \"no\"\n}}\nmain() {{}}\n");
    assert_eq!(errors(&src), ["can't return memory from inside a call to `show`: it's freed when the call returns"]);
}

#[test]
fn a_struct_that_contains_itself_is_reported_once_not_looped_over() {
    // The plain-value rule looks inside structs; it must stop at one that holds itself.
    let src = "struct A {\n a: A\n}\nstruct B {\n c: C\n}\nstruct C {\n b: B\n}\nnative f(xs: [A], d: B) -> A\nmain() {}\n";
    assert_eq!(errors(src), ["struct `A` contains itself", "struct `B` contains itself", "struct `C` contains itself"]);
}

#[test]
fn natives_refuse_vec4() {
    assert_eq!(errors("native f(v: vec4)\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f() -> vec4\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("struct S {\n v: vec4\n}\nnative f(s: S)\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f(vs: [vec4])\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f(v: ivec4)\nmain() {}\n"), vec![BAD_TYPE]);
    assert_eq!(errors("native f() -> ivec4\nmain() {}\n"), vec![BAD_TYPE]);
    // The same in a built-in module, whose natives Kestrel's own code declares.
    let src = "native f(v: vec4)\nmain() {}\n";
    let (mut p, d) = parse(src);
    assert!(d.is_empty());
    p.builtin_ranges = std::iter::once(0..src.len() as u32).collect();
    let errs: Vec<String> = check(&p).1.into_iter().map(|d| d.message).collect();
    assert_eq!(errs, vec![BAD_TYPE]);
}

#[test]
fn a_native_may_return_a_string_a_list_or_an_optional() {
    let src = "struct Game {
 x: float
}
native a() -> string
native b() -> list[string]
native c() -> list[u8]?
native d() -> string?
native e() -> list[string]?
native f() -> list[vec2]
native g() -> Game?
native h() -> int?
main() {}
";
    assert_eq!(errors(src), Vec::<String>::new());
}

#[test]
fn a_native_result_list_must_hold_plain_items_or_strings() {
    assert_eq!(errors("native f() -> list[list[int]]
main() {}
"), vec![BAD_TYPE]);
    assert_eq!(errors("native f() -> list[[int]]
main() {}
"), vec![BAD_TYPE]);
    assert_eq!(errors("native f() -> list[int?]
main() {}
"), vec![BAD_TYPE]);
    assert_eq!(errors("struct Game {
 a: Arena
}
native f() -> Game?
main() {}
"), vec![BAD_TYPE]);
    assert_eq!(errors("struct Named {
 s: string
}
native f() -> Named?
main() {}
"), vec![BAD_TYPE]);
    assert_eq!(errors("enum E {
 A
}
native f() -> E?
main() {}
"), vec![BAD_TYPE]);
}

#[test]
fn a_native_that_returns_memory_never_frees_its_arguments_at_the_call() {
    // `load` gives memory the caller keeps, so its call never runs in a scratch arena: memory made
    // in its arguments may outlive the call, even next to an interpolated argument.
    let src = "native load(path: string, n: int) -> list[u8]?
native show(path: string, n: int)
tag(n: int) -> string { return \"tag {n}\" }
main() {
 var kept = \"\"
 let n = 1
 let bytes = load(\"save{n}.bin\", if n > 0 { kept = tag(n); 1 } else { 2 })
 print(kept)
}
";
    assert_eq!(errors(src), Vec::<String>::new());
    // The same arguments to a native with no result are freed at the call.
    let freed = src.replace("let bytes = load(", "show(");
    assert_eq!(errors(&freed), ["this memory is freed when the call to `show` returns, but `kept` lives longer"]);
}
