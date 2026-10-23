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

#[test]
fn an_ivec4_add_has_one_check() {
    let traps = |src: &str, opts: Options| dump(&common::optimized(src), opts, "g").unwrap().matches("trap user2").count();
    let plain = "pure g(a: ivec4, b: ivec4) -> ivec4 {\n a + b\n}\nmain() {}\n";
    assert_eq!(traps(plain, Options::default()), 1);
    assert_eq!(traps(plain, release()), 0);
    let wrapping = "pure g(a: ivec4, b: ivec4) -> ivec4 {\n a +% b\n}\nmain() {}\n";
    assert_eq!(traps(wrapping, Options::default()), 0);
    // `-v`, `*` and a shift: one check each.
    let more = "pure g(a: ivec4, b: ivec4, n: int) -> ivec4 {\n (-a * b) << n\n}\nmain() {}\n";
    assert_eq!(traps(more, Options::default()), 3);
    assert_eq!(traps(more, release()), 0);
}

#[test]
fn ivec4_shift_right_is_arithmetic() {
    let m = common::optimized("pure g(h: ivec4) -> ivec4 {\n h >> 1\n}\nmain() {}\n");
    for opts in [Options::default(), release()] {
        let code = dump(&m, opts, "g").unwrap();
        assert!(code.contains("sshr") && !code.contains("ushr"), "{code}");
    }
}

#[test]
fn ivec4_calls_follow_the_abi() {
    let src = PASS.replace("vec4", "ivec4").replace("-> float", "-> int");
    let p = jit(&src);
    assert_eq!(call0::<i32>(&p, "t"), 15);
    assert_eq!(call0::<i32>(&p, "t_lanes"), 11 + 220 + 3300 + 44000);
}

/// The CLIF instructions that take a vector and give a scalar.
const VECTOR_TO_SCALAR: [&str; 4] = ["extractlane", "vhigh_bits", "vany_true", "vall_true"];

/// Every CLIF line of `code` that loads or stores a SIMD vector: `load.f32x4`, `store.i32x4`, or a
/// `store` of a value that is a vector: a block parameter or a result the CLIF gives a vector
/// type, or the result of an instruction with a vector operand (`v7 = imul v5, v6`, whose type the
/// text leaves out), other than one that gives a scalar (`VECTOR_TO_SCALAR`).
fn vector_accesses(code: &str) -> Vec<&str> {
    let is_vector = |ty: &str| ty == "f32x4" || ty == "i32x4";
    let mut vectors: Vec<&str> = Vec::new();
    for line in code.lines().map(str::trim) {
        if line.starts_with("block") {
            let params = line.split(['(', ')']).nth(1).unwrap_or("");
            vectors.extend(params.split(", ").filter_map(|p| p.split_once(": ")).filter(|(_, t)| is_vector(t)).map(|(v, _)| v));
        } else if let Some((v, rest)) = line.split_once(" = ") {
            let op = rest.split_whitespace().next().unwrap_or("");
            let typed = op.split_once('.').is_some_and(|(_, t)| is_vector(t));
            let name = op.split('.').next().unwrap_or(op);
            let mut operands = rest[op.len()..].split(',').filter_map(|a| a.split_whitespace().last());
            let computed = !VECTOR_TO_SCALAR.contains(&name) && operands.any(|a| vectors.contains(&a));
            if typed || computed {
                vectors.push(v);
            }
        }
    }
    code.lines()
        .filter(|line| {
            let l = line.trim();
            let typed = ["load.f32x4", "load.i32x4", "store.f32x4", "store.i32x4"].iter().any(|op| l.starts_with(op) || l.contains(&format!("= {op}")));
            let stored = l.strip_prefix("store ").and_then(|rest| rest.split(", ").next()).and_then(|first| first.split_whitespace().last());
            typed || stored.is_some_and(|v| vectors.contains(&v))
        })
        .collect()
}

/// No load or store of a vector carries Cranelift's `aligned` flag (spec R1, M19): an element, a
/// struct field and a capture are read, and an element and a struct field are written, each also
/// with a computed vector.
#[test]
fn vector_memory_access_is_never_aligned() {
    let src = "struct P {\n id: u8\n pos: vec4\n cell: ivec4\n}
pure elem(vs: [vec4], i: int) -> vec4 {\n vs[i] * 2\n}
pure ielem(vs: [ivec4], i: int) -> ivec4 {\n vs[i] *% 2\n}
pure field(p: P) -> vec4 {\n p.pos + p.pos\n}
pure ifield(p: P) -> ivec4 {\n p.cell ^ p.cell\n}
set(var vs: [ivec4], var ws: [vec4], i: int, v: ivec4, w: vec4) {\n vs[i] = v\n ws[i] = w\n}
make(v: vec4, c: ivec4) -> P {\n P(id: 1, pos: v, cell: c)\n}
seti(var vs: [ivec4], i: int, v: ivec4) {\n vs[i] = v *% 2\n}
makei(v: vec4, c: ivec4) -> P {\n P(id: 2, pos: v * 2, cell: c ^ c)\n}
captured(k: vec4, c: ivec4) -> int {\n let f = () -> int { int((k * 2).x) + (c *% 3).w }\n f()\n}
main() {}\n";
    let m = common::optimized(src);
    let mut seen = 0;
    for name in ["elem", "ielem", "field", "ifield", "set", "make", "seti", "makei", "captured"] {
        let code = dump(&m, release(), name).unwrap();
        let accesses = vector_accesses(&code);
        assert!(!accesses.is_empty(), "no vector access found in `{name}`:\n{code}");
        // A computed vector is stored with no type in the text: it is found by its operands.
        if name.ends_with('i') {
            let stores = accesses.iter().filter(|l| l.trim().starts_with("store ")).count();
            assert!(stores >= 1, "no store of a computed vector found in `{name}`:\n{code}");
        }
        for line in &accesses {
            assert!(!line.contains("aligned"), "`{name}`: {line}\n{code}");
        }
        seen += accesses.len();
    }
    // The closure's body reads both captures, whole, from its environment.
    let names: Vec<&str> = m.funcs.iter().map(|f| f.name.as_str()).collect();
    let body = *names.iter().find(|n| n.starts_with("__kestrel_closure")).unwrap_or_else(|| panic!("no closure body in {names:?}"));
    let code = dump(&m, release(), body).unwrap();
    let accesses = vector_accesses(&code);
    assert!(accesses.len() >= 2, "{code}");
    assert!(accesses.iter().all(|l| !l.contains("aligned")), "{code}");
    assert!(seen + accesses.len() >= 10, "{seen}");
}
