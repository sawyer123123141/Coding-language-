//! `@assume` in machine code: a checked build tests every assumption; `--release` tests only the
//! ones a bounds check rests on, and the proof check tests the trusted ones too.

mod common;

use std::process::Command;

use kestrel_codegen::{dump, JitProgram, Options};

use common::optimized;

const D: &str = "pure d(a: int, b: int) -> int {\n @assume(b > 0)\n a / b\n}\nmain() {}\n";
const GET: &str = "pure get(xs: [int], i: int) -> int {\n @assume(i >= 0 and i < xs.len)\n xs[i]\n}\nmain() {}\n";

const RELEASE: Options = Options { checked_arith: false, remove_checks: true, verify_proofs: false };

fn traps(src: &str, opts: Options, func: &str) -> usize {
    dump(&optimized(src), opts, func).unwrap().matches("trap user2").count()
}

#[test]
fn a_checked_build_tests_the_assumption() {
    // One check: the assumption's. The division's zero and overflow checks are gone.
    assert_eq!(traps(D, Options::default(), "d"), 1);
    // The near miss: with every check kept, the division's two are there too.
    assert_eq!(traps(D, Options { remove_checks: false, ..Options::default() }, "d"), 3);
}

#[test]
fn a_trusted_assumption_emits_nothing() {
    assert_eq!(traps(D, RELEASE, "d"), 0);
}

#[test]
fn a_kept_assumption_is_tested_under_release() {
    // The assumption's test, and no bounds check.
    assert_eq!(traps(GET, RELEASE, "get"), 1);
}

const CHILD_ENV: &str = "KESTREL_ASSUME_CHILD";

/// Child half of `the_proof_check_tests_a_trusted_assumption`; does nothing unless the parent set
/// `CHILD_ENV`.
#[test]
fn trusted_assumption_child() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let p = JitProgram::compile(&optimized(D), Options { verify_proofs: true, ..RELEASE }).expect("codegen");
    p.run_init();
    let d: extern "C" fn(i32, i32) -> i32 = unsafe { std::mem::transmute(p.fn_ptr("d").unwrap()) };
    assert_eq!(d(7, 2), 3, "an assumption that holds passes");
    d(1, 0);
    panic!("the false assumption should have ended the process");
}

#[test]
fn the_proof_check_tests_a_trusted_assumption() {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["trusted_assumption_child", "--exact", "--nocapture"])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(101), "{stderr}");
    assert!(stderr.contains("assumption `b > 0` is false on line 2"), "{stderr}");
    assert!(!stderr.contains("range analysis removed"), "{stderr}");
}
