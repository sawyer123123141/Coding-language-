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
    // Fills, copies, conversions and mixed makes, for each kind of target.
    let src = "main() {\n let a = vec3(0.5)\n let b = ivec2(7)\n let c = vec3(a)\n let d = vec3(ivec3(1, 2, 3))\n let f = ivec3(vec3(1, 2, 3))\n let g = vec3(vec2(1, 2), 3)\n let h = ivec3(1, ivec2(2, 3))\n let i = ivec2(ivec2(1, 2))\n let j = ivec2(vec2(1.5, 2))\n print(a, b, c, d, f, g, h, i, j)\n}\n";
    let (t, ty) = local_types(src);
    for (name, want) in [("a", "vec3"), ("b", "ivec2"), ("c", "vec3"), ("d", "vec3"), ("f", "ivec3"), ("g", "vec3"), ("h", "ivec3"), ("i", "ivec2"), ("j", "ivec2")] {
        assert_eq!(ty(&t, name), want, "{name}");
    }
    // Those nine; the six makes with one number per component inside them are constructions.
    assert_eq!(t.vec_makes.len(), 9);
    assert_eq!(t.ctor_args.len(), 6);
}

#[test]
fn vec4_types() {
    let src = "main() {\n let a = vec4(1).xy\n let b = vec3(1, 2, 3).xyzz\n let c = vec4(1).w\n let d = vec4(1, 2, 3, 4).wzyx\n let e = vec4(w: 4, z: 3, y: 2, x: 1) * 2 + 1\n let f = vec4(1) == vec4(2)\n print(a, b, c, d, e, f)\n}\n";
    let (t, ty) = local_types(src);
    assert_eq!(ty(&t, "a"), "vec2");
    assert_eq!(ty(&t, "b"), "vec4");
    assert_eq!(ty(&t, "c"), "float");
    assert_eq!(ty(&t, "d"), "vec4");
    assert_eq!(ty(&t, "e"), "vec4");
    assert_eq!(ty(&t, "f"), "bool");
    assert_eq!(t.lanes.values().copied().collect::<Vec<_>>(), vec![3]);
}

#[test]
fn ivec4_types() {
    let src = "main() {\n let a = ivec4(1).wzyx\n let b = ivec4(1).xy\n let c = ivec4(1).w\n let d = ivec4(1, 2, 3, 4) * 2 >> 1\n let e = ivec4(vec4(1.5))\n let f = vec4(ivec4(1))\n let g = ivec4(1) *% 0x9E3779B1\n let h = ~ivec4(1) == ivec4(x: 1, y: 2, z: 3, w: 4)\n print(a, b, c, d, e, f, g, h)\n}\n";
    let (t, ty) = local_types(src);
    assert_eq!(ty(&t, "a"), "ivec4");
    assert_eq!(ty(&t, "b"), "ivec2");
    assert_eq!(ty(&t, "c"), "int");
    assert_eq!(ty(&t, "d"), "ivec4");
    assert_eq!(ty(&t, "e"), "ivec4");
    assert_eq!(ty(&t, "f"), "vec4");
    assert_eq!(ty(&t, "g"), "ivec4");
    assert_eq!(ty(&t, "h"), "bool");
    // `.w` is a lane; `.wzyx` and `.xy` are swizzles.
    assert_eq!(t.lanes.values().copied().collect::<Vec<_>>(), vec![3]);
}

#[test]
fn vec4_lanes_are_places() {
    let src = "struct P {\n pos: vec4\n}\nmain() {\n var v = vec4()\n v.x = 1\n v.w += 2\n var p = P(pos: v)\n p.pos.y = 3\n print(v, p)\n}\n";
    let (t, ty) = local_types(src);
    assert_eq!(ty(&t, "v"), "vec4");
    let errs = run("main() {\n let v = vec4()\n v.x = 1\n}\n").1;
    assert_eq!(errs, vec!["can't change `v`: it was declared with `let`"]);
    let errs = run("main() {\n var v = vec4()\n v.xy = vec2()\n}\n").1;
    assert_eq!(errs, vec!["only variables, fields and elements can be assigned to"]);
}
