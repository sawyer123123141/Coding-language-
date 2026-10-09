use kestrel_check::{check, Tables};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

#[test]
fn vector_math_and_swizzles_check() {
    let (t, errs) = run("main() {\n var p = vec3(1, 2, 3)\n let v = vec3(0.5, 0, -1)\n p += v * 2\n let q = p.xz\n let r = -p + 1.0\n let d = p.x * q.y\n let w = 2 * q - q / 2.0\n print(p, q, r, d, w)\n}\n");
    assert!(errs.is_empty(), "{errs:?}");
    let t = t.unwrap();
    let ty = |name: &str| t.ty_name(t.locals.iter().find(|l| l.name == name).unwrap().ty);
    assert_eq!(ty("p"), "vec3");
    assert_eq!(ty("q"), "vec2");
    assert_eq!(ty("r"), "vec3");
    assert_eq!(ty("d"), "float");
    assert_eq!(ty("w"), "vec2");
    assert_eq!(t.swizzles.values().next().unwrap(), &vec![0, 2]);
}

#[test]
fn vector_errors() {
    let cases: &[(&str, &str)] = &[
        ("main() {\n let v = vec2(1, 2)\n let b = v < v\n}\n", "`<` needs numbers, found vec2 and vec2"),
        ("main() {\n var v = vec2(1, 2)\n v.yx = vec2(0, 0)\n}\n", "only variables and their fields can be assigned to"),
        ("main() {\n let v = vec2(1, 2)\n let w = v.xz\n}\n", "`vec2` has no field `xz`"),
        ("main() {\n let v = vec2(1, 2) % 2.0\n}\n", "`%` doesn't work on vectors"),
        ("main() {\n let v = vec2(1, 2) + vec3(1, 2, 3)\n}\n", "`+` can't mix vec2 and vec3"),
        ("main() {\n let v = vec2(1, 2) * true\n}\n", "`*` needs a vector and a number, found vec2 and bool"),
        ("struct vec2 {\n x: int\n}\nmain() {}\n", "`vec2` is a built-in name; pick another"),
    ];
    for (src, expected) in cases {
        assert_eq!(run(src).1, vec![expected.to_string()], "source:\n{src}");
    }
}
