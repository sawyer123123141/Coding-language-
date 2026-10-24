use kestrel_check::{check, Tables};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn ok(src: &str) -> Tables {
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "{errs:?}");
    t.expect("no errors")
}

fn local_ty(t: &Tables, name: &str) -> String {
    t.ty_name(t.locals.iter().find(|l| l.name == name).unwrap().ty)
}

#[test]
fn wrapping_operators_give_the_type_plain_ones_do() {
    let t = ok("main() {
    let a = 3
    let b: i64 = 4
    let c: u8 = 5
    let r1 = a *% a
    let r2 = a *% b
    let r3 = c +% c
    let r4 = c -% 1
    let r5 = 7 *% c
    let r6 = a +% 1 -% 2
    print(r1, r2, r3, r4, r5, r6)
}
");
    assert_eq!(local_ty(&t, "r1"), "int");
    assert_eq!(local_ty(&t, "r2"), "i64");
    assert_eq!(local_ty(&t, "r3"), "u8");
    assert_eq!(local_ty(&t, "r4"), "u8");
    // A literal on the left meets the other side as it does for `*`: at `int`.
    assert_eq!(local_ty(&t, "r5"), "int");
    assert_eq!(local_ty(&t, "r6"), "int");
}

#[test]
fn compound_wrapping_assignments_check() {
    ok("main() {
    var h = 1
    h *%= 3
    h +%= 4
    h -%= 5
    var b: u8 = 7
    b *%= 3
    var w: i64 = 1
    w +%= 9
    print(h, b, w)
}
");
}

#[test]
fn a_hex_literal_takes_its_types_bit_pattern() {
    let t = ok("main() {\n    let x = 0x9E3779B1\n    print(x)\n}\n");
    assert_eq!(local_ty(&t, "x"), "int");
    assert_eq!(t.wrapped_literals.values().copied().collect::<Vec<_>>(), vec![-1640531535]);

    let t = ok("main() {\n    let x: i64 = 0xFFFFFFFF\n    print(x)\n}\n");
    assert_eq!(local_ty(&t, "x"), "i64");
    // Within i64's positive range it is an ordinary 4294967295.
    assert!(t.wrapped_literals.is_empty());

    let t = ok("main() {\n    let x: i64 = 3\n    print(x *% 0xFFFFFFFFFFFFFFFF)\n}\n");
    assert_eq!(t.wrapped_literals.values().copied().collect::<Vec<_>>(), vec![-1]);

    // Inside its positive range a hex literal is an ordinary one.
    let t = ok("main() {\n    let x = 0x7FFFFFFF\n    let y: u8 = 0xFF\n    print(x, y)\n}\n");
    assert!(t.wrapped_literals.is_empty());
}

#[test]
fn literals_too_wide_for_their_type_still_fail() {
    let cases: &[(&str, &str)] = &[
        ("main() {\n    let x = 0x1FFFFFFFF\n}\n", "`8589934591` doesn't fit in int"),
        ("main() {\n    let x = 2654435761\n}\n", "`2654435761` doesn't fit in int"),
        ("main() {\n    let x: u8 = 0x1FF\n}\n", "`511` doesn't fit in u8"),
    ];
    for (src, expected) in cases {
        assert_eq!(run(src).1, vec![expected.to_string()], "source:\n{src}");
    }
}

#[test]
fn wrapping_operators_refuse_floats_and_vectors() {
    let w1 = |op: &str, plain: &str| format!("`{op}` wraps integers; for floats use `{plain}`");
    let cases: Vec<(&str, String)> = vec![
        ("main() {\n    let x = 1.5 *% 2.0\n}\n", w1("*%", "*")),
        ("main() {\n    let x = 2 +% 2.0\n}\n", w1("+%", "+")),
        ("main() {\n    var f = 1.0\n    f +%= 2.0\n}\n", w1("+%=", "+=")),
        ("main() {\n    var f = 1.0\n    f *%= 2\n}\n", w1("*%=", "*=")),
        ("main() {\n    var f = 1.0\n    f -%= 2\n}\n", w1("-%=", "-=")),
        ("main() {\n    let v = vec2(1, 2) -% vec2(1, 1)\n}\n", w1("-%", "-")),
        ("main() {\n    var v = vec3(1, 2, 3)\n    v *%= 2\n}\n", w1("*%=", "*=")),
    ];
    for (src, expected) in &cases {
        assert_eq!(run(src).1, vec![expected.clone()], "source:\n{src}");
    }
}

#[test]
fn wrapping_operators_refuse_other_types() {
    let (_, errs) = run("main() {\n    let x = true +% false\n}\n");
    assert_eq!(errs, vec!["`+%` needs integers, found bool".to_string()]);
}

#[test]
fn a_hex_literal_meeting_an_i64_is_read_as_an_i64_on_either_side() {
    for expr in ["0xFFFFFFFF + w", "w + 0xFFFFFFFF", "0xFFFFFFFF *% w", "0xFFFFFFFF < w"] {
        let t = ok(&format!("main() {{\n    let w: i64 = 3\n    print({expr})\n}}\n"));
        assert!(t.wrapped_literals.is_empty(), "{expr}");
    }
    // Alone it stays an `int`, and so does the variable that holds it.
    let t = ok("main() {\n    let x = 0xFFFFFFFF\n    print(x * 1.5)\n}\n");
    assert_eq!(t.wrapped_literals.len(), 1);
    // Next to a float it is the float it spells, on either side.
    let t = ok("main() {\n    let d: f64 = 1.5\n    print(0xFFFFFFFF * d, d * 0xFFFFFFFF, 0xFFFFFFFF * 1.5, 1.5 * 0xFFFFFFFF)\n}\n");
    assert!(t.wrapped_literals.is_empty());
}

#[test]
fn the_smallest_integer_written_in_hex_cannot_be_negated() {
    let (_, errs) = run("main() {\n    let a = -0x80000000\n}\n");
    assert_eq!(errs, vec!["`-0x80000000` negates -2147483648, which overflows; write `0x80000000`".to_string()]);
    ok("main() {\n    let a: i64 = -0x80000000\n    let b = -0xFFFFFFFF\n    print(a, b)\n}\n");
}

#[test]
fn a_hex_expression_never_gives_two_values_by_order() {
    // Each pair is the same operation written both ways round. The right one reads the literal at
    // the wider type; the left one read it as a wrapped `int` before meeting that type, which
    // would be another value, so it is an error that says what to write.
    let pairs = [
        ("-0xFFFFFFFF + w", "w + -0xFFFFFFFF"),
        ("(0xFFFFFFFF >> 4) + z", "z + (0xFFFFFFFF >> 4)"),
        ("0xFFFFFFFF * 2 + d", "d + 0xFFFFFFFF * 2"),
        ("0xFFFFFFFF * 2 + w", "w + 0xFFFFFFFF * 2"),
        ("-0xFFFFFFFF * 1.5", "1.5 * -0xFFFFFFFF"),
        ("max(0xFFFFFFFF * 2, z)", "max(z, 0xFFFFFFFF * 2)"),
        ("(if w > 0 { 0xFFFFFFFF } else { 0 }) + w", "w + (if w > 0 { 0xFFFFFFFF } else { 0 })"),
    ];
    let src = |e: &str| format!("main() {{\n    let w: i64 = 10\n    let z: i64 = 0\n    let d = 0.5\n    print({e})\n}}\n");
    for (left, right) in pairs {
        let (_, errs) = run(&src(left));
        assert_eq!(errs.len(), 1, "{left}: {errs:?}");
        assert!(errs[0].starts_with("`0xFFFFFFFF` is read as the `int` -1 here, so this changes when it is widened to"), "{left}: {errs:?}");
        assert!(errs[0].contains("write `i64(0xFFFFFFFF)` to mean 4294967295, or `int(0xFFFFFFFF)` to keep -1"), "{left}: {errs:?}");
        let t = ok(&src(right));
        assert!(t.wrapped_literals.is_empty(), "{right}");
    }
    // A bare literal agrees in every position of a built-in.
    for e in ["max(0xFFFFFFFF, z)", "max(z, 0xFFFFFFFF)", "clamp(0xFFFFFFFF, z, w)", "sqrt(0xFFFFFFFF)", "min(d, 0xFFFFFFFF)"] {
        let t = ok(&src(e));
        assert!(t.wrapped_literals.is_empty(), "{e}");
    }
    // Explicit conversions, variables and `int`-only expressions keep the wrapped `int`.
    for e in ["int(0xFFFFFFFF) * 2 + w", "i64(0xFFFFFFFF) * 2 + w", "0xFFFFFFFF * 2 + 1", "abs(-0xFFFFFFFF)"] {
        ok(&src(e));
    }
    ok("main() {\n    let x = 0xFFFFFFFF * 2\n    let w: i64 = 1\n    print(x + w, x * 1.5)\n}\n");
}

#[test]
fn an_int_vector_and_a_float_get_advice_that_works() {
    let (_, errs) = run("main() {\n    let h = ivec4(1)\n    let z = h +% 1.5\n}\n");
    let want = "`+%` can't mix ivec4 and float; convert the float with `int(...)` to wrap, or the ivec4 with `vec4(...)` and use `+`";
    assert_eq!(errs, vec![want.to_string()]);
    // Both ways the advice gives compile.
    ok("main() {\n    let h = ivec4(1)\n    print(h +% int(1.5), vec4(h) + 1.5)\n}\n");
}
