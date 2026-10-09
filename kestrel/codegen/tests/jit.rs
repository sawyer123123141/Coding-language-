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
