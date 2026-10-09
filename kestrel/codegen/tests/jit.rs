use kestrel_codegen::{JitProgram, Options};
use kestrel_syntax::{parse, LineIndex};

fn jit_with(src: &str, opts: Options) -> JitProgram {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let m = kestrel_kir::lower(&p, &t.unwrap(), &LineIndex::new(src));
    JitProgram::compile(&m, opts).expect("codegen")
}

fn jit(src: &str) -> JitProgram {
    jit_with(&format!("{src}\nmain() {{}}\n"), Options::default())
}

fn call0<R>(p: &JitProgram, name: &str) -> R {
    let f: extern "C" fn() -> R = unsafe { std::mem::transmute(p.fn_ptr(name).expect(name)) };
    f()
}

fn call1(p: &JitProgram, name: &str, x: i32) -> i32 {
    let f: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(p.fn_ptr(name).expect(name)) };
    f(x)
}

#[test]
fn recursion() {
    let p = jit("pure fib(n: int) -> int {\n if n < 2 { n } else { fib(n - 1) + fib(n - 2) }\n}");
    assert_eq!(call1(&p, "fib", 20), 6765);
}

#[test]
fn loops_break_continue() {
    let p = jit("sum_to(n: int) -> int {\n var t = 0\n for i from 0 to n { t += i }\n t\n}\nodd_sum() -> int {\n var i = 0\n var s = 0\n while true {\n  i += 1\n  if i > 10 { break }\n  if i % 2 == 0 { continue }\n  s += i\n }\n s\n}");
    assert_eq!(call1(&p, "sum_to", 100), 4950);
    assert_eq!(call0::<i32>(&p, "odd_sum"), 25);
}

#[test]
fn var_parameters() {
    let p = jit("bump(var x: int, by: int) {\n x += by\n}\nf() -> int {\n var a = 5\n bump(var a, by: 3)\n bump(var a, 2)\n a\n}");
    assert_eq!(call0::<i32>(&p, "f"), 10);
}

#[test]
fn floats_and_conversions() {
    let p = jit("f() -> int {\n let x: float = 7\n let y = x / 2.0\n int(y * 10.0)\n}\ng() -> i64 {\n let a = 3\n let b: i64 = a\n b * 1000000000\n}\nh() -> int {\n int(7.5 % 2.0 * 10.0)\n}\nsat() -> int {\n int(f64(2147483647) * 2.0)\n}\nu() -> int {\n let a: u8 = 200\n let b: u8 = 55\n int(a + b)\n}\nd() -> int {\n -7 / 2\n}\nr() -> int {\n -7 % 2\n}");
    assert_eq!(call0::<i32>(&p, "f"), 35);
    assert_eq!(call0::<i64>(&p, "g"), 3_000_000_000);
    assert_eq!(call0::<i32>(&p, "h"), 15);
    assert_eq!(call0::<i32>(&p, "sat"), i32::MAX);
    assert_eq!(call0::<i32>(&p, "u"), 255);
    assert_eq!(call0::<i32>(&p, "d"), -3);
    assert_eq!(call0::<i32>(&p, "r"), -1);
}

#[test]
fn short_circuit() {
    let p = jit("side(var n: int) -> bool {\n n += 1\n true\n}\nsc() -> int {\n var n = 0\n let a = false and side(var n)\n let b = true or side(var n)\n let c = true and side(var n)\n n\n}");
    assert_eq!(call0::<i32>(&p, "sc"), 1);
}

#[test]
fn if_expression_with_diverging_branch() {
    let p = jit("pick(x: int) -> int {\n let y = if x > 0 { x * 2 } else { return -1 }\n y + 1\n}");
    assert_eq!(call1(&p, "pick", 5), 11);
    assert_eq!(call1(&p, "pick", -3), -1);
}

#[test]
fn unchecked_arithmetic_wraps() {
    let src = "w() -> int {\n let a = 2147483647\n a + 1\n}\nmain() {}\n";
    let p = jit_with(src, Options { checked_arith: false });
    assert_eq!(call0::<i32>(&p, "w"), i32::MIN);
}

#[test]
fn main_runs_through_runtime() {
    let p = jit_with("main() {\n print(1, 2.5, true, \"hi\")\n}\n", Options::default());
    assert_eq!(p.run_main(), 0);
}

fn call2(p: &JitProgram, name: &str, x: i32, y: i32) -> i32 {
    let f: extern "C" fn(i32, i32) -> i32 = unsafe { std::mem::transmute(p.fn_ptr(name).expect(name)) };
    f(x, y)
}

#[test]
fn continue_in_for_runs_the_step() {
    let p = jit("odds() -> int {\n var s = 0\n for i from 0 to 10 {\n  if i % 2 == 0 { continue }\n  s += i\n }\n s\n}");
    assert_eq!(call0::<i32>(&p, "odds"), 25);
}

#[test]
fn var_parameter_forwarded_to_another_call() {
    let p = jit("bump(var x: int, by: int) {\n x += by\n}\ntwice(var x: int) {\n bump(var x, by: 1)\n bump(var x, by: 2)\n}\nf() -> int {\n var a = 5\n twice(var a)\n a\n}");
    assert_eq!(call0::<i32>(&p, "f"), 8);
}

#[test]
fn unchecked_min_div_neg_one_wraps() {
    let src = "d(a: int, b: int) -> int {\n a / b\n}\nr(a: int, b: int) -> int {\n a % b\n}\nmain() {}\n";
    let p = jit_with(src, Options { checked_arith: false });
    assert_eq!(call2(&p, "d", i32::MIN, -1), i32::MIN);
    assert_eq!(call2(&p, "r", i32::MIN, -1), 0);
}

#[test]
fn u8_compares_unsigned() {
    let p = jit("gt() -> int {\n let a: u8 = 200\n let b: u8 = 100\n if a > b { 1 } else { 0 }\n}\nlt() -> int {\n let a: u8 = 200\n let b: u8 = 100\n if a < b { 1 } else { 0 }\n}");
    assert_eq!(call0::<i32>(&p, "gt"), 1);
    assert_eq!(call0::<i32>(&p, "lt"), 0);
}

#[test]
fn float_to_u8_saturates() {
    let p = jit("hi() -> int {\n let x = 300.0\n int(u8(x))\n}\nlo() -> int {\n let x = -5.0\n int(u8(x))\n}");
    assert_eq!(call0::<i32>(&p, "hi"), 255);
    assert_eq!(call0::<i32>(&p, "lo"), 0);
}

/// Dividends that straddle every power-of-two boundary and the type limits.
const I32_XS: &[i32] =
    &[0, 1, -1, 2, -2, 3, -3, 7, -7, 31, -31, 32, -33, 1000, -1000, i32::MAX, i32::MIN, i32::MIN + 1];
const I64_XS: &[i64] = &[
    0, 1, -1, 2, -2, 3, -3, 63, 64, -64, -65, 1000, -1000, i32::MAX as i64, i32::MIN as i64, i64::MAX,
    i64::MIN, i64::MIN + 1,
];
const U8_XS: &[u8] = &[0, 1, 2, 15, 16, 17, 127, 128, 255];

/// Kestrel function-name suffix for a divisor (`-2` -> `m2`).
fn tag(n: i64) -> String {
    if n < 0 { format!("m{}", -n) } else { n.to_string() }
}

/// Source for `d_N(x) = x / N` and `r_N(x) = x % N` over every divisor, at type `ty`.
fn div_source(ty: &str, divisors: &[i64]) -> String {
    let mut src = String::new();
    for &n in divisors {
        let t = tag(n);
        src += &format!("d_{t}(x: {ty}) -> {ty} {{\n x / {n}\n}}\nr_{t}(x: {ty}) -> {ty} {{\n x % {n}\n}}\n");
    }
    src + "main() {}\n"
}

fn call_as<A, R>(p: &JitProgram, name: &str, x: A) -> R {
    let f: extern "C" fn(A) -> R = unsafe { std::mem::transmute(p.fn_ptr(name).expect(name)) };
    f(x)
}

#[test]
fn division_by_constant_matches_rust_i32() {
    const DIVISORS: &[i64] = &[1, 2, 4, 8, 32, 1024, 7, 100, -2, -8];
    for checked_arith in [true, false] {
        let p = jit_with(&div_source("int", DIVISORS), Options { checked_arith });
        for &n in DIVISORS {
            for &x in I32_XS {
                let n32 = n as i32;
                let (d, r) = (format!("d_{}", tag(n)), format!("r_{}", tag(n)));
                assert_eq!(call1(&p, &d, x), x.wrapping_div(n32), "{x} / {n} (checked={checked_arith})");
                assert_eq!(call1(&p, &r, x), x.wrapping_rem(n32), "{x} % {n} (checked={checked_arith})");
            }
        }
    }
}

#[test]
fn division_by_constant_matches_rust_i64() {
    const DIVISORS: &[i64] = &[2, 64, 3];
    for checked_arith in [true, false] {
        let p = jit_with(&div_source("i64", DIVISORS), Options { checked_arith });
        for &n in DIVISORS {
            for &x in I64_XS {
                let (d, r) = (format!("d_{}", tag(n)), format!("r_{}", tag(n)));
                assert_eq!(call_as::<i64, i64>(&p, &d, x), x.wrapping_div(n), "{x} / {n} (checked={checked_arith})");
                assert_eq!(call_as::<i64, i64>(&p, &r, x), x.wrapping_rem(n), "{x} % {n} (checked={checked_arith})");
            }
        }
    }
}

#[test]
fn division_by_constant_matches_rust_u8() {
    const DIVISORS: &[i64] = &[1, 2, 16, 3];
    for checked_arith in [true, false] {
        let p = jit_with(&div_source("u8", DIVISORS), Options { checked_arith });
        for &n in DIVISORS {
            for &x in U8_XS {
                let (d, r) = (format!("d_{}", tag(n)), format!("r_{}", tag(n)));
                assert_eq!(call_as::<u8, u8>(&p, &d, x), x / n as u8, "{x} / {n} (checked={checked_arith})");
                assert_eq!(call_as::<u8, u8>(&p, &r, x), x % n as u8, "{x} % {n} (checked={checked_arith})");
            }
        }
    }
}

// Output and panics can't be observed in-process (the runtime writes to the
// real stdout and `kestrel_rt_panic` exits), so these cases run `main` in a
// child copy of this test binary and inspect its stdout and exit code.
const CHILD_CASES: &[(&str, &str)] = &[
    (
        "print_order",
        "noisy() -> int {\n print(\"in noisy\")\n 5\n}\nmain() {\n print(\"result\", noisy())\n}\n",
    ),
    ("print_panic", "main() {\n var z = 0\n z += 0\n print(\"x =\", 10 / z)\n}\n"),
    (
        "print_left_to_right",
        "tick(var n: int) -> int {\n n += 1\n n\n}\nmain() {\n var a = 0\n var b = 10\n print(tick(var a), tick(var b), tick(var a))\n}\n",
    ),
    ("div_by_variable_zero", "main() {
 var d = 1
 d -= 1
 print(\"q =\", 10 / d)
}
"),
    ("rem_by_variable_zero", "main() {
 var d = 1
 d -= 1
 print(\"r =\", 10 % d)
}
"),
    ("div_by_literal_zero", "main() {
 print(\"q =\", 10 / 0)
}
"),
    (
        "min_div_variable_neg_one",
        "main() {
 var v = 0
 v -= 1
 let m = -2147483647 - 1
 print(\"q =\", m / v)
}
",
    ),
    (
        "labelled_arg_order",
        "noisy(n: int) -> int {\n print(\"eval\", n)\n n\n}\nsub(a: int, b: int) -> int {\n a - b\n}\nmain() {\n let r = sub(b: noisy(1), a: noisy(2))\n print(r)\n}\n",
    ),
];

/// Entry point for the child process; does nothing in a normal test run.
#[test]
fn child_entry() {
    let Ok(case) = std::env::var("KESTREL_CHILD_CASE") else { return };
    let src = CHILD_CASES.iter().find(|(n, _)| *n == case).expect("known case").1;
    jit_with(src, Options::default()).run_main();
}

/// Runs a `CHILD_CASES` entry; returns (exit code, stdout, stderr).
fn run_child(case: &str) -> (Option<i32>, String, String) {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["child_entry", "--exact", "--nocapture", "--test-threads=1"])
        .env("KESTREL_CHILD_CASE", case)
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn print_evaluates_arguments_before_any_output() {
    let (code, stdout, _) = run_child("print_order");
    assert_eq!(code, Some(0), "stdout: {stdout}");
    assert!(stdout.contains("in noisy\nresult 5\n"), "stdout: {stdout}");
}

#[test]
fn panic_inside_print_argument_writes_nothing() {
    let (code, stdout, stderr) = run_child("print_panic");
    assert_eq!(code, Some(101), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stderr.contains("division by zero on line 4"), "stderr: {stderr}");
    assert!(!stdout.contains("x ="), "stdout: {stdout}");
}

#[test]
fn print_arguments_evaluate_left_to_right() {
    let (code, stdout, _) = run_child("print_left_to_right");
    assert_eq!(code, Some(0), "stdout: {stdout}");
    assert!(stdout.contains("1 11 2\n"), "stdout: {stdout}");
}

#[test]
fn division_by_a_zero_variable_still_panics() {
    for case in ["div_by_variable_zero", "rem_by_variable_zero", "div_by_literal_zero"] {
        let (code, stdout, stderr) = run_child(case);
        assert_eq!(code, Some(101), "{case}
stdout: {stdout}
stderr: {stderr}");
        assert!(stderr.contains("division by zero"), "{case}
stderr: {stderr}");
        assert!(!stdout.contains("q =") && !stdout.contains("r ="), "{case}
stdout: {stdout}");
    }
}

#[test]
fn min_divided_by_a_variable_neg_one_still_panics() {
    let (code, stdout, stderr) = run_child("min_div_variable_neg_one");
    assert_eq!(code, Some(101), "stdout: {stdout}
stderr: {stderr}");
    assert!(stderr.contains("integer overflow"), "stderr: {stderr}");
    assert!(!stdout.contains("q ="), "stdout: {stdout}");
}

#[test]
fn emits_an_object_file() {
    let src = "main() {\n print(1)\n}\n";
    let (p, _) = parse(src);
    let (t, _) = kestrel_check::check(&p);
    let m = kestrel_kir::lower(&p, &t.unwrap(), &LineIndex::new(src));
    let bytes = kestrel_codegen::emit_object(&m, Options::default()).expect("object");
    assert!(bytes.len() > 100);
}

// TODO(2b): labelled args must evaluate in source order
#[test]
#[ignore = "labelled arguments are evaluated in parameter order until plan 2b"]
fn labelled_arguments_evaluate_in_source_order() {
    let (code, stdout, _) = run_child("labelled_arg_order");
    assert_eq!(code, Some(0), "stdout: {stdout}");
    assert!(stdout.contains("eval 1\neval 2\n"), "stdout: {stdout}");
}
