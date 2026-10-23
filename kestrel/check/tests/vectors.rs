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
        ("main() {\n let v = vec2(1, 2)\n let b = v < v\n}\n", "`<` doesn't compare vectors; compare components, like `a.x < b.x`"),
        ("main() {\n var v = vec2(1, 2)\n v.yx = vec2(0, 0)\n}\n", "only variables, fields and elements can be assigned to"),
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

/// The checked tables of `src`, which must check without errors, and the type name of a local.
fn local_types(src: &str) -> (Tables, impl Fn(&Tables, &str) -> String) {
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "{errs:?}");
    (t.unwrap(), |t: &Tables, name: &str| t.ty_name(t.locals.iter().find(|l| l.name == name).unwrap().ty))
}

#[test]
fn ivec_types_and_swizzles() {
    let src = "main() {\n let a = ivec3(1, 2, 3).zx\n let b = ivec3(1, 2, 3).x\n let c = vec2(1, 2).xyx\n let d = ivec2(1, 2).yyx\n let e = ivec3(4, 5, 6) / 2 + a.xyx\n print(a, b, c, d, e)\n}\n";
    let (t, ty) = local_types(src);
    assert_eq!(ty(&t, "a"), "ivec2");
    assert_eq!(ty(&t, "b"), "int");
    assert_eq!(ty(&t, "c"), "vec3");
    assert_eq!(ty(&t, "d"), "ivec3");
    assert_eq!(ty(&t, "e"), "ivec3");
}

#[test]
fn makes_resolve() {
    // A fill, a copy, two conversions and two mixed makes, for each kind of target.
    let src = "main() {\n let a = vec3(0.5)\n let b = ivec2(7)\n let c = vec3(a)\n let d = vec3(ivec3(1, 2, 3))\n let f = ivec3(vec3(1, 2, 3))\n let g = vec3(vec2(1, 2), 3)\n let h = ivec3(1, ivec2(2, 3))\n print(a, b, c, d, f, g, h)\n}\n";
    let (t, ty) = local_types(src);
    for (name, want) in [("a", "vec3"), ("b", "ivec2"), ("c", "vec3"), ("d", "vec3"), ("f", "ivec3"), ("g", "vec3"), ("h", "ivec3")] {
        assert_eq!(ty(&t, name), want, "{name}");
    }
    // Those seven; the four makes with one number per component inside them are constructions.
    assert_eq!(t.vec_makes.len(), 7);
    assert_eq!(t.ctor_args.len(), 4);
}
