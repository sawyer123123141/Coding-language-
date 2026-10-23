//! `vec4` in codegen: one `F32X4` register value, passed and returned by Cranelift's own calling
//! convention (direct, indirect and through interfaces), and refused for a CPU without SSE4.1.

mod common;

use common::{call0, jit};
use cranelift_codegen::settings::{self, Configurable};
use kestrel_codegen::{dump, emit_object_for, Options};

fn release() -> Options {
    Options { checked_arith: false, ..Options::default() }
}

#[test]
fn vec4_math_stays_in_registers() {
    let m = common::optimized("pure f(a: vec4, b: vec4) -> vec4 {\n a * b + a\n}\nmain() {}\n");
    let code = dump(&m, release(), "f").unwrap();
    assert!(code.contains("f32x4") && code.contains("fmul"), "{code}");
    assert!(!code.contains("explicit_slot"), "{code}");
}

/// Five vectors and an `int`, rotated through a recursive call (so it isn't inlined): more vector
/// arguments than registers, each passed by Cranelift's implicit pointer on Windows x64.
const PASS: &str = "pass(a: vec4, b: vec4, c: vec4, d: vec4, e: vec4, n: int) -> vec4 {
    if n == 0 {
        return a + b + c + d + e
    }
    pass(b, c, d, e, a, n - 1)
}

t() -> float {
    pass(vec4(1), vec4(2), vec4(3), vec4(4), vec4(5), 3).w
}

t_lanes() -> float {
    let v = pass(vec4(1, 2, 3, 4), vec4(0), vec4(0), vec4(0), vec4(10, 20, 30, 40), 2)
    v.x + v.y * 10 + v.z * 100 + v.w * 1000
}
";

#[test]
fn vector_calls_follow_the_abi() {
    let p = jit(PASS);
    assert_eq!(call0::<f32>(&p, "t"), 15.0);
    assert_eq!(call0::<f32>(&p, "t_lanes"), 11.0 + 220.0 + 3300.0 + 44000.0);
}

const VALUES: &str = "interface Mover {
    pure moved(self, by: vec4) -> vec4
}

struct Walker: Mover {
    step: vec4

    pure moved(self, by: vec4) -> vec4 {
        by + self.step
    }
}

through(m: Mover, by: vec4) -> vec4 {
    m.moved(by)
}

twice(f: (vec4) -> vec4, v: vec4) -> vec4 {
    f(f(v))
}

t_closure() -> float {
    let k = vec4(1, 2, 3, 4)
    let f = (v: vec4) -> vec4 { v * 2 + k }
    let r = twice(f, vec4(1))
    r.x + r.w * 10
}

t_iface() -> float {
    let r = through(Walker(step: vec4(0.5, 1, 1.5, 2)), vec4(10))
    r.x + r.w * 100
}
";

#[test]
fn lanes_through_function_values_and_interfaces() {
    let p = jit(VALUES);
    // ((1 * 2 + k) * 2 + k): x = (2 + 1) * 2 + 1 = 7, w = (2 + 4) * 2 + 4 = 16.
    assert_eq!(call0::<f32>(&p, "t_closure"), 7.0 + 160.0);
    assert_eq!(call0::<f32>(&p, "t_iface"), 10.5 + 1200.0);
}

/// An x64 Windows ISA with SSE4.1 turned off, through `isa::lookup` and the flag.
fn isa_without_sse41() -> cranelift_codegen::isa::OwnedTargetIsa {
    let mut isa = cranelift_codegen::isa::lookup_by_name("x86_64-pc-windows-msvc").unwrap();
    isa.set("has_sse41", "false").unwrap();
    let mut flags = settings::builder();
    flags.set("is_pic", "true").unwrap();
    isa.finish(settings::Flags::new(flags)).unwrap()
}

#[test]
fn without_sse41_vectors_are_refused() {
    let with = common::optimized("pure f(v: vec4) -> float {\n v.x\n}\nmain() {\n print(f(vec4(1)))\n}\n");
    let err = emit_object_for(&with, Options::default(), isa_without_sse41()).unwrap_err();
    assert_eq!(
        err,
        "`vec4` and `ivec4` need a CPU with SSE4.1 (every x64 CPU since about 2011); this one doesn't have it, so use `vec2`, `vec3` or plain numbers instead"
    );
    // A vector only in an expression counts too.
    let inline = common::optimized("main() {\n print(vec4(1, 2, 3, 4).y)\n}\n");
    assert!(emit_object_for(&inline, Options::default(), isa_without_sse41()).is_err());
    let without = common::optimized("pure f(v: vec3) -> float {\n v.x\n}\nmain() {\n print(f(vec3(1)))\n}\n");
    assert!(emit_object_for(&without, Options::default(), isa_without_sse41()).is_ok());
}
