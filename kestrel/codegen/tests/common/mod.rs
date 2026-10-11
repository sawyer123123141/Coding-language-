//! Helpers shared by the codegen test binaries.
#![allow(dead_code)]

use kestrel_codegen::{JitProgram, Options};
use kestrel_syntax::{parse, LineIndex};

pub fn jit_with(src: &str, opts: Options) -> JitProgram {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = kestrel_kir::lower(&p, &t.unwrap(), &LineIndex::new(src));
    kestrel_kir::inline::inline_small(&mut m);
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

/// Like `jit_with`, giving the JIT the addresses of Rust functions that the
/// program's `native` declarations import.
pub fn jit_natives(src: &str, extra: &[(&str, *const u8)]) -> Result<JitProgram, String> {
    jit_natives_with(src, extra, Options::default())
}

/// Like `jit_natives`, with the given options (`checked_arith: false` is `--release`).
pub fn jit_natives_with(src: &str, extra: &[(&str, *const u8)], opts: Options) -> Result<JitProgram, String> {
    jit_natives_as(src, extra, opts, false)
}

/// Like `jit_natives`, with the whole source marked as a built-in module, which may name
/// `GpuHandle`.
pub fn jit_natives_builtin(src: &str, extra: &[(&str, *const u8)]) -> Result<JitProgram, String> {
    jit_natives_as(src, extra, Options::default(), true)
}

/// Compiles `src` with the given natives; `builtin` marks the whole source as a built-in module.
fn jit_natives_as(src: &str, extra: &[(&str, *const u8)], opts: Options, builtin: bool) -> Result<JitProgram, String> {
    let (mut p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    if builtin {
        p.builtin_ranges = std::iter::once(0..src.len() as u32).collect();
    }
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = kestrel_kir::lower(&p, &t.unwrap(), &LineIndex::new(src));
    kestrel_kir::inline::inline_small(&mut m);
    JitProgram::compile_with(&m, opts, extra)
}

/// Runs the program's `main` on this thread and returns what it printed.
/// (`run_main` uses another thread, whose output can't be captured here.)
pub fn run_captured(p: &JitProgram) -> String {
    kestrel_runtime::print::capture_output();
    call0::<()>(p, "main");
    String::from_utf8(kestrel_runtime::print::take_output()).expect("print writes UTF-8")
}
