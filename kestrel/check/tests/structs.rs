use kestrel_check::{check, Res, Tables};
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

const POINTS: &str = "struct Point {
    x: int
    y: int = 0

    pure len2(self) -> int { self.x * self.x + self.y * self.y }
    shift(var self, dx: int) {
        self.x += dx
    }
}

struct Line {
    a: Point
    b: Point
}

pure mid(l: Line) -> Point {
    Point((l.a.x + l.b.x) / 2, y: (l.a.y + l.b.y) / 2)
}

nudge(var n: int) {
    n += 1
}

main() {
    var p = Point(3, 4)
    let q = Point(y: 1, x: 2)
    let r = Point(5)
    p.shift(1)
    p.shift(p.x)
    p.x = p.x + q.y
    var line = Line(p, r)
    line.a.y += 10
    nudge(var line.b.x)
    let m = mid(line)
    print(p, m.len2(), line)
}
";

#[test]
fn structs_methods_and_fields_check() {
    let t = ok(POINTS);
    let p = t.locals.iter().find(|l| l.name == "p").unwrap();
    assert_eq!(t.ty_name(p.ty), "Point");
    let mut ctor_orders: Vec<Vec<Option<usize>>> = t.ctor_args.values().cloned().collect();
    ctor_orders.sort();
    assert!(ctor_orders.contains(&vec![Some(0), None]), "Point(5) fills y from its default: {ctor_orders:?}");
    assert!(ctor_orders.contains(&vec![Some(1), Some(0)]), "Point(y: 1, x: 2): {ctor_orders:?}");
    let shift = t.fns.iter().position(|f| f.name == "Point.shift").expect("method registered");
    assert!(t.res.values().any(|r| *r == Res::Fn(kestrel_check::FnId(shift as u32))));
    assert!(t.fns[shift].params[0].is_var);
    assert!(!t.fns[t.fns.iter().position(|f| f.name == "Point.len2").unwrap()].params[0].is_var);
    assert!(t.field_index.len() >= 10);
}

#[test]
fn struct_errors() {
    let p = "struct P {\n x: int\n g(var self) {\n self.x = 1\n }\n}\n";
    let cases: Vec<(String, &str)> = vec![
        ("struct P {\n x: int\n x: int\n}\nmain() {}\n".into(), "`P` has two fields named `x`"),
        ("struct A {\n a: A\n}\nmain() {}\n".into(), "struct `A` contains itself"),
        ("pure f() -> int { 1 }\nstruct P {\n x: int = f()\n}\nmain() {}\n".into(), "field defaults can only use literals, operators and constants"),
        ("struct P {\n x: int = true\n}\nmain() {}\n".into(), "expected int, found bool"),
        (format!("{p}main() {{\n let a = P()\n}}\n"), "missing field `x` for `P`"),
        (format!("{p}main() {{\n let a = P(1, 2)\n}}\n"), "`P` takes 1 argument(s), got 2"),
        (format!("{p}main() {{\n let a = P(z: 1)\n}}\n"), "`P` has no field named `z`"),
        (format!("{p}main() {{\n let a = P(1)\n let y = a.z\n}}\n"), "`P` has no field `z`"),
        ("main() {\n let a = 1\n let b = a.x\n}\n".into(), "int has no field `x`"),
        (format!("{p}main() {{\n let a = P(1)\n a.x = 2\n}}\n"), "can't change `a`: it was declared with `let`"),
        ("struct P {\n x: int\n f(self) {\n self.x = 1\n }\n}\nmain() {}\n".into(), "can't change `self`: the method takes `self`, not `var self`"),
        (format!("{p}main() {{\n let a = P(1)\n a.g()\n}}\n"), "can't call `g` on `a`: it changes `a`, which was declared with `let`"),
        (format!("{p}main() {{\n P(1).g()\n}}\n"), "`g` changes its receiver, so it needs a variable, not a temporary"),
        (format!("{p}main() {{\n var a = P(1)\n a.h()\n}}\n"), "`P` has no method `h`"),
        (format!("{p}pure f(a: P) -> int {{\n var b = a\n b.g()\n b.x\n}}\nmain() {{}}\n"), "pure functions can only call pure functions; `P.g` isn't pure"),
        (format!("{p}set(var n: int, a: P) {{\n n = a.x\n}}\nmain() {{\n var a = P(1)\n set(var a.x, a)\n}}\n"), "`a` is passed as `var` and also used in another argument of the same call"),
        (format!("{p}keep(a: P) {{}}\nmain() {{\n var a = P(1)\n a.g()\n}}\nput(var a: P, b: P) {{}}\nh() {{\n var a = P(1)\n put(var a, a)\n}}\n"), "`a` is passed as `var` and also used in another argument of the same call"),
        (format!("{p}main() {{\n let a = P\n}}\n"), "`P` is a struct; build one with `P(...)`"),
        ("struct P {\n x: int\n x(self) {}\n}\nmain() {}\n".into(), "`P` already has a field named `x`"),
        (format!("{p}main() {{\n P(1)\n}}\n"), "this line has no effect; did you mean to continue the line above?"),
        (format!("{p}main() {{\n let a = P(1)\n let b = a == a\n}}\n"), "`==` needs numbers, found P and P"),
    ];
    for (src, expected) in cases {
        let (_, errs) = run(&src);
        assert_eq!(errs, vec![expected.to_string()], "source:\n{src}");
    }
}

#[test]
fn scalar_reads_alongside_var_are_allowed() {
    ok("struct P {\n x: int\n add(var self, n: int) {\n self.x += n\n }\n}\nbump(var a: int, b: int) {\n a += b\n}\nmain() {\n var p = P(1)\n p.add(p.x)\n var n = 1\n bump(var n, n)\n}\n");
}

const COLOR: &str = "struct Color {
    r: float
    g: float
    b: float
    a: float = 1
}
";

#[test]
fn constants_can_hold_struct_values() {
    ok(&format!(
        "{COLOR}let RED = Color(1, 0, 0)
let TINT = Color(1, 2, sqrt(2), a: 0.5)
let SHADE = Color(PI, 0, 0)
let UP = vec2(0, 1)
let FLOOR = vec3(0, -1, 0)
let SPAWN = vec2(0, 2) * 3
let ALIAS = RED
main() {{
    print(RED, TINT, SHADE, UP, FLOOR, SPAWN, ALIAS)
}}
"
    ));
}

#[test]
fn field_defaults_can_construct_structs() {
    ok(&format!("{COLOR}struct Sprite {{\n tint: Color = Color(1, 1, 1)\n at: vec2 = vec2(0, 0)\n}}\nmain() {{\n print(Sprite())\n}}\n"));
}

#[test]
fn struct_constants_need_constant_arguments() {
    let msg = "constants can only use literals, operators and earlier constants";
    let cases = [
        format!("{COLOR}f() -> float {{ 1 }}\nlet X = Color(1, 2, f())\nmain() {{}}\n"),
        format!("{COLOR}let X = Color(1, 2, randf())\nmain() {{}}\n"),
        format!("{COLOR}let X = Color(1, 2, Color(1, 2, 3).r)\nmain() {{}}\n"),
        "let X = vec2(1, randf())\nmain() {}\n".to_string(),
    ];
    for src in cases {
        assert_eq!(run(&src).1, vec![msg.to_string()], "source:\n{src}");
    }
    // A field default is held to the same rule, with its own message.
    let src = format!("{COLOR}f() -> float {{ 1 }}\nstruct S {{\n c: Color = Color(1, 2, f())\n}}\nmain() {{}}\n");
    assert_eq!(run(&src).1, vec!["field defaults can only use literals, operators and constants".to_string()]);
}

#[test]
fn struct_constants_are_type_checked() {
    let cases = [
        (format!("{COLOR}let X = Color(1, 2)\nmain() {{}}\n"), "missing field `b` for `Color`"),
        (format!("{COLOR}let X = Color(1, 2, true)\nmain() {{}}\n"), "expected float, found bool"),
        (format!("{COLOR}let X: vec2 = Color(1, 2, 3)\nmain() {{}}\n"), "expected vec2, found Color"),
    ];
    for (src, expected) in cases {
        assert_eq!(run(&src).1, vec![expected.to_string()], "source:\n{src}");
    }
}
