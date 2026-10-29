//! The inline `float` `sin`, `cos` and `tan` are branch-free but for the one special-input branch
//! to the cold runtime call: no float `select`, no `fmin`/`fmax` and no float-to-int conversion,
//! each of which hides a jump inside an x64 pseudo-instruction.

mod common;

use kestrel_codegen::{dump, Options};
use kestrel_runtime::math::wide;

fn release() -> Options {
    Options { checked_arith: false, ..Options::default() }
}

/// The printed forms of the x64 pseudo-instructions that hold a jump, from cranelift-codegen
/// 0.137's `isa/x64/inst/mod.rs` printer: `XmmCmove` (a float `select`) ends in a `$next:` label,
/// `XmmMinMaxSeq` prints `xmm min seq` or `xmm max seq`, `CvtFloatToSintSeq` and
/// `CvtFloatToUintSeq` print `cvt_float<N>_to_sint<M>_seq` and `cvt_float<N>_to_uint<M>_seq`
/// (with `_sat` before `_seq` when saturating), and `CvtUint64ToFloatSeq` prints `u64_to_f<N>_seq`.
const HIDDEN_JUMPS: [&str; 6] = ["$next:", "xmm min seq", "xmm max seq", "_to_sint", "_to_uint", "u64_to_f"];

/// The release dump of `func` in `src` (with an empty `main` added).
fn release_dump(src: &str, func: &str) -> String {
    dump(&common::optimized(&format!("{src}\nmain() {{}}\n")), release(), func).unwrap()
}

/// The IR part of a dump (it comes first and ends at the function's closing brace) and the
/// machine-code part.
fn parts(code: &str) -> (&str, &str) {
    code.split_once("\n}\n").unwrap_or((code, ""))
}

/// In a dump: the `brif` lines and the `call` lines of the IR, and the lines of the machine code
/// that hold any of `HIDDEN_JUMPS`.
fn fast_path(code: &str) -> (usize, usize, usize) {
    let (ir, asm) = parts(code);
    let brifs = ir.lines().filter(|l| l.contains(" brif ")).count();
    let calls = ir.lines().filter(|l| l.contains(" call fn")).count();
    let hidden = asm.lines().filter(|l| HIDDEN_JUMPS.iter().any(|h| l.contains(h))).count();
    (brifs, calls, hidden)
}

/// The machine-code lines that jump on a condition (`j<cc> ...`), not counting `jmp`.
fn conditional_jumps(code: &str) -> Vec<&str> {
    let (_, asm) = parts(code);
    asm.lines()
        .map(str::trim)
        .filter(|l| l.starts_with('j') && !l.starts_with("jmp"))
        .collect()
}

#[test]
fn the_hidden_jump_names_are_right() {
    let code = release_dump("pure g(a: float, b: float) -> float { min(a, b) }", "g");
    assert!(code.contains("xmm min seq"), "{code}");
    // Kestrel's `round` picks with a float `select`.
    let code = release_dump("pure g(x: float) -> float { round(x) }", "g");
    assert!(parts(&code).0.contains("select"), "{code}");
    assert!(code.contains("$next:"), "{code}");
    let code = release_dump("pure g(x: float) -> int { int(x) }", "g");
    assert!(code.contains("_to_sint"), "{code}");
}

#[test]
fn sin_cos_tan_have_one_branch_and_no_hidden_jumps() {
    for op in ["sin", "cos", "tan"] {
        let code = release_dump(&format!("pure f(x: float) -> float {{ {op}(x) }}"), "f");
        let (brifs, calls, hidden) = fast_path(&code);
        if op == "tan" && !wide::TAN_INLINE {
            assert_eq!((brifs, calls), (0, 1), "tan is a call: {code}");
            continue;
        }
        assert_eq!((brifs, calls, hidden), (1, 1, 0), "{op}: {code}");
        // The special test is an integer compare, so its branch is one jump (no parity jump).
        let jumps = conditional_jumps(&code);
        assert!(jumps.len() <= 1, "{op}: {jumps:?}\n{code}");
    }
}

#[test]
fn the_special_test_has_no_float_compare() {
    let code = release_dump("pure f(x: float) -> float { sin(x) }", "f");
    let (ir, _) = parts(&code);
    assert!(!ir.lines().any(|l| l.contains(" fcmp ")), "{code}");
}
