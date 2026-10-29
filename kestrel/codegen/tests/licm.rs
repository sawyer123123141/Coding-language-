//! Loop-invariant work is computed once, before the loop: the code generator drops the block
//! parameters that never change (`phis`) and takes a checked operation's value from a plain
//! instruction (`int_arith`), so Cranelift's elaborator hoists out of the body.

mod common;

use kestrel_codegen::{dump, Options};

fn release() -> Options {
    Options { checked_arith: false, ..Options::default() }
}

/// The lines of the optimised IR's blocks other than the entry block that multiply by `k`'s
/// constant (`0x6000_0005`, 1610612741), compiled with `opts`.
fn multiplies_after_entry(src: &str, func: &str, opts: Options) -> Vec<String> {
    let m = common::optimized(src);
    let code = dump(&m, opts, func).unwrap();
    // The IR comes first; its blocks are separated by blank lines, the entry block first, after
    // the signatures of any calls.
    let ir = code.split("\n}\n").next().unwrap();
    let blocks: Vec<&str> = ir.split("\n\n").filter(|b| b.lines().any(|l| l.starts_with("block"))).collect();
    let rest = &blocks[1..];
    assert!(!rest.is_empty() && blocks[0].contains("imul"), "{code}");
    rest.iter().flat_map(|b| b.lines()).filter(|l| l.contains("imul") && l.contains("0x6000_0005")).map(String::from).collect()
}

#[test]
fn a_value_made_before_a_while_loop_stays_out_of_it() {
    let src = "pure f(seed: int, n: int) -> int {\n var acc: i64 = 0\n let k = i64(seed) * 1610612741\n var i = 0\n while i < n {\n  acc = acc ^ (i64(i) * 3 ^ k)\n  i += 1\n }\n int(acc & 0xFFFF)\n}\nmain() {}\n";
    assert_eq!(multiplies_after_entry(src, "f", release()), Vec::<String>::new());
}

#[test]
fn a_loop_invariant_value_used_in_a_for_loop_is_hoisted() {
    let src = "pure f(seed: int, n: int) -> int {\n var acc: i64 = 0\n for i from 0 to n {\n  acc = acc ^ (i64(i) * 3 ^ i64(seed) * 1610612741)\n }\n int(acc & 0xFFFF)\n}\nmain() {}\n";
    assert_eq!(multiplies_after_entry(src, "f", release()), Vec::<String>::new());
}

#[test]
fn a_loop_left_by_an_else_break_is_hoisted_out_of() {
    let src = "pure f(seed: int, n: int) -> int {\n var acc: i64 = 0\n var i = 0\n while true {\n  if i < n {\n   acc = acc ^ (i64(i) * 3 ^ i64(seed) * 1610612741)\n   i += 1\n  } else {\n   break\n  }\n }\n int(acc & 0xFFFF)\n}\nmain() {}\n";
    assert_eq!(multiplies_after_entry(src, "f", release()), Vec::<String>::new());
}

#[test]
fn code_after_an_if_whose_branch_breaks_is_hoisted_out_of() {
    // Cranelift before wasmtime 50 (PR #14383) lost the loop when its walk of the `if`'s branch
    // reached the `break`'s block, outside the loop, before the code after the `if`.
    let src = "pure f(seed: int, n: int) -> int {\n var acc: i64 = 0\n var i = 0\n while i < n {\n  if i > 5 {\n   if acc == 12345 {\n    break\n   }\n  }\n  acc = acc ^ (i64(i) * 3 ^ i64(seed) * 1610612741)\n  i += 1\n }\n int(acc & 0xFFFF)\n}\nmain() {}\n";
    assert_eq!(multiplies_after_entry(src, "f", release()), Vec::<String>::new());
}

#[test]
fn code_after_an_if_whose_branch_holds_a_check_is_hoisted_out_of() {
    // The check's failure block is outside the loop, like a `break`'s.
    let src = "pure f(seed: int, n: int) -> int {\n var acc: i64 = 0\n var c = 0\n for i from 0 to n {\n  if i > 5 {\n   c = c + i\n  }\n  acc = acc ^ (i64(i) * 3 ^ i64(seed) * 1610612741)\n }\n int(acc & 0xFFFF) + c\n}\nmain() {}\n";
    assert_eq!(multiplies_after_entry(src, "f", Options::default()), Vec::<String>::new());
}

#[test]
fn an_inner_loop_does_not_redo_work_that_depends_only_on_the_outer_one() {
    // `y * 19349663` depends on the outer loop only, `seed * 1610612741` on neither: the inner
    // body multiplies by 3 and nothing else. (Cranelift hoists a value with a constant operand one
    // loop at a time, so `seed`'s goes to the outer body, not the entry.)
    let src = "pure f(seed: int, n: int) -> int {\n var acc: i64 = 0\n for y from 0 to n {\n  for x from 0 to n {\n   acc = acc ^ (i64(x) * 3 ^ i64(y) * 19349663 ^ i64(seed) * 1610612741)\n  }\n }\n int(acc & 0xFFFF)\n}\nmain() {}\n";
    let code = dump(&common::optimized(src), release(), "f").unwrap();
    let ir = code.split("\n}\n").next().unwrap();
    let imuls = |b: &str| b.lines().filter(|l| l.contains("= imul")).count();
    let inner = ir.split("\n\n").find(|b| b.lines().any(|l| l.contains("= 3"))).expect(&code);
    assert_eq!(imuls(inner), 1, "{code}");
    assert_eq!(imuls(ir), 3, "{code}");
}

#[test]
fn a_checked_value_is_hoisted_and_its_check_stays() {
    // `(c * 32 + y) * 7` is checked (`c` can be any `int`). Its checks stay in the inner loop,
    // where it is written, but its value, made by plain `imul` and `iadd`, moves to the outer body:
    // no block that multiplies also holds a check.
    let src = "pure f(c: int, n: int) -> int {\n var acc = 0\n for y from 0 to n {\n  for x from 0 to n {\n   acc = acc ^ ((c * 32 + y) * 7 ^ x)\n  }\n }\n acc\n}\nmain() {}\n";
    let code = dump(&common::optimized(src), Options::default(), "f").unwrap();
    let ir = code.split("\n}\n").next().unwrap();
    let multiplies: Vec<&str> = ir.split("\n\n").filter(|b| b.contains("= imul")).collect();
    assert!(!multiplies.is_empty() && multiplies.iter().all(|b| !b.contains("_overflow")), "{code}");
    assert_eq!(ir.matches("smul_overflow").count(), 2, "{code}");
}

#[test]
fn a_negated_loop_test_keeps_its_meaning() {
    // `loop_exits` negates a loop's test. A float test is negated to its exact opposite, so NaN
    // still fails `x < 10.0`; a test that isn't a comparison is compared with zero.
    let p = common::jit("pure count(x: float) -> int {\n var y = x\n var n = 0\n while y < 10.0 {\n  y += 1.0\n  n += 1\n }\n n\n}\npure steps(n: int) -> int {\n var i = 0\n var go = n > 0\n while go {\n  i += 1\n  go = i < n\n }\n i\n}\npure ne(x: float) -> int {\n var y = x\n var n = 0\n while y != 3.0 {\n  y = 3.0\n  n += 1\n }\n n\n}\npure below(n: int) -> int {\n var i = n\n var k = 0\n while i < 10 {\n  i += 1\n  k += 1\n }\n k\n}\npure small(n: u8) -> int {\n var i = n\n var k = 0\n while i < 200 {\n  i += 50\n  k += 1\n }\n k\n}");
    let count: extern "C" fn(f32) -> i32 = unsafe { std::mem::transmute(p.fn_ptr("count").unwrap()) };
    assert_eq!((count(7.5), count(10.0), count(f32::NAN)), (3, 0, 0));
    assert_eq!((common::call1(&p, "steps", 5), common::call1(&p, "steps", 0)), (5, 0));
    let ne: extern "C" fn(f32) -> i32 = unsafe { std::mem::transmute(p.fn_ptr("ne").unwrap()) };
    assert_eq!((ne(3.0), ne(1.0), ne(f32::NAN)), (0, 1, 1));
    assert_eq!((common::call1(&p, "below", 7), common::call1(&p, "below", 12), common::call1(&p, "below", -3)), (3, 0, 13));
    let small: extern "C" fn(u8) -> i32 = unsafe { std::mem::transmute(p.fn_ptr("small").unwrap()) };
    assert_eq!((small(0), small(199), small(200)), (4, 1, 0));
}

#[test]
fn pow_in_a_particle_loop_is_called_once_before_the_loop() {
    // `pow(KEEP, dt)` reads the inliner's temporary for `dt`, which the hoisting pass sees through,
    // so `kestrel_rt_powf` is called once per chunk, in the entry block, not once per particle.
    let src = "let KEEP = 0.5\nstruct Particle {\n pos: vec2\n vel: vec2\n pure step(var self, dt: float, target: vec2) {\n  self.vel += (target - self.pos).normalized() * 900.0 * dt\n  self.vel *= pow(KEEP, dt)\n  self.pos += self.vel * dt\n }\n}\nrun(var ps: list[Particle], dt: float, target: vec2) {\n par for p in ps {\n  p.step(dt, target)\n }\n}\nmain() {}\n";
    let m = common::optimized(src);
    let body = &m.funcs[m.par_bodies[0].0 as usize].name;
    let code = dump(&m, release(), body).unwrap();
    let ir = code.split("\n}\n").next().unwrap();
    let blocks: Vec<&str> = ir.split("\n\n").filter(|b| b.lines().any(|l| l.starts_with("block"))).collect();
    let calls = |b: &str| b.lines().filter(|l| l.contains(" call ")).count();
    assert_eq!(calls(blocks[0]), 1, "{code}");
    assert_eq!(blocks.iter().map(|b| calls(b)).sum::<usize>(), 1, "{code}");
}
