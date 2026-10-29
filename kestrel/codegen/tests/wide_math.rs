//! The inline `float` `sin`, `cos` and `tan` give the same bits as the runtime's wide functions
//! (`kestrel_rt_sinf` and so on) on every sweep input, in checked and release builds: alone, with
//! two functions sharing one reduction, and in a loop whose values live across the slow call.

mod common;

use kestrel_codegen::{JitProgram, Options};
use kestrel_runtime::math::sweep::{edges, inputs, same};
use kestrel_runtime::math::{kestrel_rt_cosf, kestrel_rt_sinf, kestrel_rt_tanf};

fn release() -> Options {
    Options { checked_arith: false, ..Options::default() }
}

/// Checked (`Options::default()`) and release.
fn builds() -> [(&'static str, Options); 2] {
    [("checked", Options::default()), ("release", release())]
}

/// `f` from `src`, compiled with `opts`, as a function of one `float`.
fn jit_float_fn(src: &str, opts: Options) -> (JitProgram, extern "C" fn(f32) -> f32) {
    let p = common::jit_with(&format!("{src}\nmain() {{}}\n"), opts);
    let f: extern "C" fn(f32) -> f32 = unsafe { std::mem::transmute(p.fn_ptr("f").expect("f")) };
    (p, f)
}

/// Panics, naming the first five inputs where `got` and `want` differ with both results' bits.
fn assert_same(what: &str, xs: impl Iterator<Item = f32>, got: impl Fn(f32) -> f32, want: impl Fn(f32) -> f32) {
    let mut count = 0usize;
    let mut first = Vec::new();
    for x in xs {
        let (g, w) = (got(x), want(x));
        if !same(g, w) {
            count += 1;
            if first.len() < 5 {
                first.push(format!("x = {x:e} ({:#010X}): inline {:#010X}, runtime {:#010X}", x.to_bits(), g.to_bits(), w.to_bits()));
            }
        }
    }
    assert_eq!(count, 0, "{what}: {count} inputs differ; the first:\n{}", first.join("\n"));
}

fn one_argument(name: &str) -> impl Iterator<Item = f32> {
    inputs(name).into_iter().map(|(a, _)| a)
}

#[test]
fn inline_sin_cos_tan_match_the_runtime() {
    let functions: [(&str, extern "C" fn(f32) -> f32); 3] =
        [("sin", kestrel_rt_sinf), ("cos", kestrel_rt_cosf), ("tan", kestrel_rt_tanf)];
    for (op, want) in functions {
        let xs: Vec<f32> = one_argument(op).collect();
        for (build, opts) in builds() {
            let (_p, f) = jit_float_fn(&format!("pure f(x: float) -> float {{ {op}(x) }}"), opts);
            assert_same(&format!("{op}, {build}"), xs.iter().copied(), |x| f(x), |x| want(x));
        }
    }
}

#[test]
fn sin_and_cos_of_one_angle_match() {
    let xs: Vec<f32> = one_argument("sin").collect();
    for (build, opts) in builds() {
        let (_p, f) = jit_float_fn("pure f(x: float) -> float { sin(x) * 3.0 + cos(x) }", opts);
        let want = |x: f32| kestrel_rt_sinf(x) * 3.0 + kestrel_rt_cosf(x);
        assert_same(&format!("sin(x) * 3 + cos(x), {build}"), xs.iter().copied(), |x| f(x), want);
    }
}

#[test]
fn trig_in_a_loop_matches() {
    const COUNT: usize = 20_000;
    const N: i32 = 8;
    // Every edge (the special inputs take the slow call inside the loop) and random inputs.
    let e = edges();
    let all = inputs("sin");
    let random = &all[..all.len() - e.len()];
    let step = random.len() / (COUNT - e.len());
    let xs: Vec<f32> = random.iter().step_by(step).take(COUNT - e.len()).map(|&(a, _)| a).chain(e).collect();
    assert_eq!(xs.len(), COUNT);
    let src = "pure f(x: float, n: int) -> float {\n var acc: float = 0.0\n for i from 0 to n {\n  acc += sin(x + float(i)) * tan(x)\n }\n acc\n}\nmain() {}\n";
    let want = |x: f32| {
        let mut acc = 0f32;
        for i in 0..N {
            acc += kestrel_rt_sinf(x + i as f32) * kestrel_rt_tanf(x);
        }
        acc
    };
    for (build, opts) in builds() {
        let p = common::jit_with(src, opts);
        let f: extern "C" fn(f32, i32) -> f32 = unsafe { std::mem::transmute(p.fn_ptr("f").expect("f")) };
        assert_same(&format!("the loop, {build}"), xs.iter().copied(), |x| f(x, N), want);
    }
}
