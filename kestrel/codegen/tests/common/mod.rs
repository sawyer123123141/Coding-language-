//! Helpers shared by the codegen test binaries.
#![allow(dead_code)]

use kestrel_codegen::{JitProgram, Options};
use kestrel_syntax::{parse, LineIndex};

pub fn jit_with(src: &str, opts: Options) -> JitProgram {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let m = kestrel_kir::lower(&p, &t.unwrap(), &LineIndex::new(src));
    JitProgram::compile(&m, opts).expect("codegen")
}

pub fn jit(src: &str) -> JitProgram {
    jit_with(&format!("{src}\nmain() {{}}\n"), Options::default())
}

pub fn call0<R>(p: &JitProgram, name: &str) -> R {
    let f: extern "C" fn() -> R = unsafe { std::mem::transmute(p.fn_ptr(name).expect(name)) };
    f()
}

pub fn call1(p: &JitProgram, name: &str, x: i32) -> i32 {
    let f: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(p.fn_ptr(name).expect(name)) };
    f(x)
}
