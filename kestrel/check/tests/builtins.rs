use kestrel_check::{check, Builtin, Res, Tables, Ty};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn local_ty(t: &Tables, name: &str) -> Ty {
    t.locals.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no local {name}")).ty
}

#[test]
fn float_functions_take_the_common_float_type() {
    let src = "main() {\n let w: f64 = 2\n let a = sqrt(16)\n let b = pow(2, w)\n let c = atan2(1.5, 2)\n let d = floor(w)\n let e = PI\n let f: f64 = TAU\n let g = sin(u8(3))\n let h = PI * w\n print(a, b, c, d, e, f, g, h)\n}\n";
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "{errs:?}");
    let t = t.unwrap();
    let expected = [
        ("a", Ty::Float),
        ("b", Ty::F64),
        ("c", Ty::Float),
        ("d", Ty::F64),
        ("e", Ty::Float),
        ("f", Ty::F64),
        ("g", Ty::Float),
        ("h", Ty::F64),
    ];
    for (name, ty) in expected {
        assert_eq!(local_ty(&t, name), ty, "type of `{name}`");
    }
    let builtins: Vec<Builtin> = t.res.values().filter_map(|r| if let Res::Builtin(b) = r { Some(*b) } else { None }).collect();
    for b in [Builtin::Sqrt, Builtin::Pow, Builtin::Atan2, Builtin::Floor, Builtin::Sin, Builtin::Pi, Builtin::Tau] {
        assert!(builtins.contains(&b), "{b:?} should be resolved");
    }
}

#[test]
fn the_other_argument_meets_an_f64_one() {
    // `pow(2, w)`: the integer first goes to float, then both meet at f64.
    let (t, errs) = run("main() {\n let w: f64 = 2\n print(pow(2, w))\n}\n");
    assert!(errs.is_empty(), "{errs:?}");
    let t = t.unwrap();
    assert!(t.coerce.values().any(|&ty| ty == Ty::F64), "the literal 2 should end up as f64");
    assert!(!t.coerce.values().any(|&ty| ty == Ty::Float), "no stop at float on the way");
}

#[test]
fn builtins_are_pure_and_constants_may_use_them() {
    let src = "let ROOT2 = sqrt(2.0)\nlet HALF_PI = PI / 2\npure hyp(a: float, b: float) -> float {\n sqrt(a * a + b * b)\n}\nmain() {\n print(ROOT2, HALF_PI, hyp(3, 4))\n}\n";
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "{errs:?}");
    let t = t.unwrap();
    assert_eq!(t.consts[0].ty, Ty::Float);
    assert_eq!(t.consts[1].ty, Ty::Float);
}

#[test]
fn builtin_tables_agree() {
    for &name in Builtin::GLOBAL_NAMES {
        let b = Builtin::global(name).unwrap_or_else(|| panic!("`{name}` is listed but not found"));
        assert_eq!(b.name(), name);
        assert!(Builtin::vector_method(name).is_none(), "`{name}` is both a global and a vector method");
    }
    for name in ["length", "normalized", "dot", "distance_to"] {
        let b = Builtin::vector_method(name).unwrap();
        assert_eq!(b.name(), name);
        assert!(Builtin::global(name).is_none(), "vector method `{name}` must not be a global name");
    }
    assert_eq!(Builtin::global("PI"), Some(Builtin::Pi));
    assert!(Builtin::global("sqrt").unwrap().is_pure());
    assert!(!Builtin::global("randf").unwrap().is_pure());
    assert!(!Builtin::global("rand_seed").unwrap().is_pure());
    assert!(Builtin::global("main").is_none());
}

#[test]
fn builtin_errors() {
    let cases: &[(&str, &str)] = &[
        ("main() {\n print(sqrt(1, 2))\n}\n", "`sqrt` takes one value"),
        ("main() {\n print(sqrt())\n}\n", "`sqrt` takes one value"),
        ("main() {\n print(atan2(1))\n}\n", "`atan2` takes two values"),
        ("main() {\n print(pow(1, 2, 3))\n}\n", "`pow` takes two values"),
        ("main() {\n print(sqrt(\"x\"))\n}\n", "`sqrt` needs a number here, found string"),
        ("main() {\n print(sqrt(true))\n}\n", "`sqrt` needs a number here, found bool"),
        ("main() {\n print(atan2(1, vec2(1, 2)))\n}\n", "`atan2` needs a number here, found vec2"),
        ("main() {\n print(sqrt(x: 4))\n}\n", "built-in functions take plain values, not labels or `var`"),
        ("main() {\n var x = 4.0\n print(sqrt(var x))\n}\n", "built-in functions take plain values, not labels or `var`"),
        ("main() {\n sqrt(4)\n}\n", "the result of `sqrt` is unused, so this line does nothing"),
        ("main() {\n let PI = 3\n}\n", "`PI` is a built-in name; pick another"),
        ("main() {\n let min = 3\n}\n", "`min` is a built-in name; pick another"),
        ("round(x: int) -> int { x }\nmain() {}\n", "`round` is a built-in name; pick another"),
        ("log(x: int) {\n print(x)\n}\nmain() {}\n", "`log` is a built-in name; pick another"),
        ("main() {\n for round from 0 to 3 {\n print(round)\n }\n}\n", "`round` is a built-in name; pick another"),
        ("main() {\n PI = 3\n}\n", "can't change constant `PI`"),
        ("main() {\n let f = sqrt\n}\n", "`sqrt` is a function; call it with `sqrt(...)`"),
        ("main() {\n let x = PI(1)\n}\n", "`PI` is a constant; write it without `(...)`"),
        ("let X = randf()\nmain() {}\n", "constants can only use literals, operators and earlier constants"),
        // Typed by a later task.
        ("main() {\n print(abs(1))\n}\n", "`abs` isn't available yet"),
        ("main() {\n print(randf())\n}\n", "`randf` isn't available yet"),
    ];
    for (src, expected) in cases {
        assert_eq!(run(src).1, vec![expected.to_string()], "source:\n{src}");
    }
}

#[test]
fn errors_inside_arguments_are_still_found() {
    let (_, errs) = run("main() {\n print(sqrt(nope, 2))\n}\n");
    assert_eq!(errs, vec!["`sqrt` takes one value".to_string(), "unknown name `nope`".to_string()]);
}
