//! The inline `float` math (every function but `pow`, and float `%`) is branch-free but for the one
//! special-input branch to the cold runtime call: no float `select`, no `fmin`/`fmax` and no
//! float-to-int conversion, each of which hides a jump inside an x64 pseudo-instruction.

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
/// `the_hidden_jump_names_are_right` makes the first four from Kestrel code; Kestrel has no unsigned
/// conversion, so `_to_uint` and `u64_to_f` are checked only against the printer's source.
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
    // `brif` prints typed (`brif.i8`) too.
    let brifs = ir.lines().filter(|l| l.split_whitespace().any(|t| t == "brif" || t.starts_with("brif."))).count();
    let calls = ir.lines().filter(|l| l.contains(" call fn")).count();
    let hidden = asm.lines().filter(|l| HIDDEN_JUMPS.iter().any(|h| l.contains(h))).count();
    (brifs, calls, hidden)
}

/// The machine-code lines that call a function (the cold slow-path call, or one Cranelift added
/// while lowering, such as a libcall), not counting unwind directives.
fn machine_calls(code: &str) -> Vec<&str> {
    let (_, asm) = parts(code);
    asm.lines()
        .map(str::trim)
        .filter(|l| l.split_whitespace().next().is_some_and(|t| t.starts_with("call")) && !l.contains("unwind"))
        .collect()
}

/// The machine-code lines that jump on a condition (`j<cc> ...`), not counting `jmp`.
fn conditional_jumps(code: &str) -> Vec<&str> {
    let (_, asm) = parts(code);
    asm.lines()
        .map(str::trim)
        .filter(|l| l.starts_with('j') && !l.starts_with("jmp"))
        .collect()
}

/// The machine-code lines that move an xmm register to a general register (`movq %xmm3, %rdx`,
/// `movd %xmm0, %eax`): a float's bits leaving the float registers. A spill to the stack doesn't
/// count.
fn xmm_to_gpr(code: &str) -> Vec<&str> {
    let (_, asm) = parts(code);
    asm.lines()
        .map(str::trim)
        .filter(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            matches!(t.as_slice(), [op, src, dst] if (*op == "movq" || *op == "movd") && src.starts_with("%xmm")
                && dst.starts_with('%') && !dst.starts_with("%xmm"))
        })
        .collect()
}

#[test]
fn the_hidden_jump_names_are_right() {
    let code = release_dump("pure g(a: float, b: float) -> float { min(a, b) }", "g");
    assert!(code.contains("xmm min seq"), "{code}");
    let code = release_dump("pure g(a: float, b: float) -> float { max(a, b) }", "g");
    assert!(code.contains("xmm max seq"), "{code}");
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
        assert_eq!(machine_calls(&code).len(), 1, "{op}: {:?}\n{code}", machine_calls(&code));
        // The special test is an integer compare, so its branch is one jump (no parity jump).
        let jumps = conditional_jumps(&code);
        assert!(jumps.len() <= 1, "{op}: {jumps:?}\n{code}");
    }
}

#[test]
fn the_special_test_has_no_float_compare() {
    for op in ["sin", "log"] {
        let code = release_dump(&format!("pure f(x: float) -> float {{ {op}(x) }}"), "f");
        let (ir, _) = parts(&code);
        assert!(!ir.lines().any(|l| l.contains(" fcmp ")), "{op}: {code}");
    }
}

#[test]
fn sin_and_cos_of_one_angle_share_one_reduction() {
    let code = release_dump("pure f(x: float) -> float { sin(x) * 3.0 + cos(x) }", "f");
    let (ir, _) = parts(&code);
    let promotes = ir.lines().filter(|l| l.contains(" fpromote")).count();
    assert_eq!(promotes, 1, "{code}");
    assert_eq!(fast_path(&code).2, 0, "{code}");
}

#[test]
fn every_inline_float_function_has_one_branch() {
    let sources = [
        ("exp", "pure f(x: float) -> float { exp(x) }"),
        ("log", "pure f(x: float) -> float { log(x) }"),
        ("atan2", "pure f(y: float, x: float) -> float { atan2(y, x) }"),
        ("atan", "pure f(x: float) -> float { atan(x) }"),
        ("asin", "pure f(x: float) -> float { asin(x) }"),
        ("acos", "pure f(x: float) -> float { acos(x) }"),
        ("%", "pure f(a: float, b: float) -> float { a % b }"),
    ];
    for (op, src) in sources {
        let code = release_dump(src, "f");
        assert_eq!(fast_path(&code), (1, 1, 0), "{op}: {code}");
        assert_eq!(machine_calls(&code).len(), 1, "{op}: {:?}\n{code}", machine_calls(&code));
        // The special tests are integer compares, and the float compares that pick values are
        // vector compares giving masks, so the only conditional jump is the special branch.
        let jumps = conditional_jumps(&code);
        assert!(jumps.len() <= 1, "{op}: {jumps:?}\n{code}");
    }
}

#[test]
fn pow_stays_one_call() {
    let code = release_dump("pure f(x: float, y: float) -> float { pow(x, y) }", "f");
    let (brifs, calls, _) = fast_path(&code);
    assert_eq!((brifs, calls), (0, 1), "{code}");
}

#[test]
fn the_xmm_to_gpr_pattern_is_right() {
    // `checksum` hashes the bits of an `f64` in a general register.
    let code = release_dump("pure g(x: f64) -> i64 { checksum(x) }", "g");
    let moves = xmm_to_gpr(&code);
    assert!(moves.iter().any(|l| l.starts_with("movq %xmm")), "{moves:?}
{code}");
}

#[test]
fn the_scalar_float_compare_pattern_is_right() {
    let code = release_dump("pure g(a: f64, b: f64) -> bool { a < b }", "g");
    assert!(parts(&code).1.contains("ucomisd"), "{code}");
}

/// The functions whose picks compare floats, with the number of `movq`s from an xmm register to a
/// general register each keeps: `exp`'s is `t`'s bits for `2^k`, `%`'s the quotient's bits for its
/// special test, and `acos`'s and `atan2`'s the two values of the sign pick, an integer `select`
/// (a mask in an xmm register measured slower there).
const FLOAT_PICKS: [(&str, &str, usize); 6] = [
    ("exp", "pure f(x: float) -> float { exp(x) }", 1),
    ("atan", "pure f(x: float) -> float { atan(x) }", 0),
    ("asin", "pure f(x: float) -> float { asin(x) }", 0),
    ("acos", "pure f(x: float) -> float { acos(x) }", 2),
    ("atan2", "pure f(y: float, x: float) -> float { atan2(y, x) }", 2),
    ("%", "pure f(a: float, b: float) -> float { a % b }", 1),
];

#[test]
fn the_float_picks_stay_in_the_float_registers() {
    // A pick on a float compare is a vector compare and a blend (or `minpd`/`maxpd`): no scalar
    // compare setting flags for a `cmov`, and no `f64` moved to a general register and back.
    for (op, src, kept) in FLOAT_PICKS {
        let code = release_dump(src, "f");
        let (_, asm) = parts(&code);
        assert!(!asm.contains("ucomis"), "{op}: {code}");
        assert!(asm.contains("cmpltpd") || asm.contains("minpd"), "{op}: {code}");
        let moves: Vec<&str> = xmm_to_gpr(&code).into_iter().filter(|l| l.starts_with("movq")).collect();
        assert_eq!(moves.len(), kept, "{op}: {moves:?}
{code}");
    }
}

/// Whether the machine code builds the `f64` constant `c` in a general register (`movabsq`).
fn builds_constant(code: &str, c: f64) -> bool {
    parts(code).1.contains(&format!("movabsq $0x{:x},", c.to_bits()))
}

#[test]
fn a_constant_first_is_built_in_a_general_register() {
    let code = release_dump("pure g(x: f64) -> f64 { 1.5 - x }", "g");
    assert!(builds_constant(&code, 1.5), "{code}");
}

#[test]
fn the_polynomials_read_their_constants_from_memory() {
    // The constants go second in the adds and multiplies, so x64 reads them from memory.
    let atan = release_dump("pure f(x: float) -> float { atan(x) }", "f");
    let exp = release_dump("pure f(x: float) -> float { exp(x) }", "f");
    let sin = release_dump("pure f(x: float) -> float { sin(x) }", "f");
    let log = release_dump("pure f(x: float) -> float { log(x) }", "f");
    for (code, cs) in [(&atan, &wide::A7[..]), (&exp, &wide::E8[..]), (&sin, &[wide::S1, wide::S3, wide::C2][..]), (&log, &wide::L5[..])] {
        for &c in cs {
            assert!(!builds_constant(code, c), "{c:e}: {code}");
        }
    }
}
