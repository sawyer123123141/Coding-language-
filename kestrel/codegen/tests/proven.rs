//! Proven checks: the markers range analysis puts in KIR (`ExprKind::Proven`, `IndexCheck::Proven`)
//! leave their checks out of the machine code, or keep them as proof checks when asked.
//!
//! The analysis itself comes later; `prove` here marks by hand, so only the helper's marks
//! count (`remove_checks` is off).

mod common;

use std::process::Command;

use kestrel_codegen::{dump, JitProgram, Options};
use kestrel_kir::visit_mut::{walk_expr, walk_place, VisitMut};
use kestrel_kir::{BinOp, Expr, ExprKind, IndexCheck, KTy, Module, Place, Proj};

use common::{call1, optimized};

/// Wraps every integer `+`, `*` and `/` in `Proven`, and proves every checked index, in `func`.
struct Prove;

impl VisitMut for Prove {
    fn visit_expr(&mut self, e: &mut Expr) {
        walk_expr(self, e);
        let integer = matches!(e.ty, KTy::I32 | KTy::I64 | KTy::U8);
        if integer && matches!(e.kind, ExprKind::Binary(BinOp::Add | BinOp::Mul | BinOp::Div, ..)) {
            let (ty, line) = (e.ty, e.line);
            let inner = std::mem::replace(e, Expr { kind: ExprKind::Bool(false), ty, line });
            *e = Expr { kind: ExprKind::Proven(Box::new(inner)), ty, line };
        }
    }

    fn visit_place(&mut self, p: &mut Place) {
        walk_place(self, p);
        for proj in &mut p.path {
            if let Proj::Index { check, .. } = proj {
                if *check == IndexCheck::Checked {
                    *check = IndexCheck::Proven;
                }
            }
        }
    }
}

fn prove(m: &mut Module, func: &str) {
    let f = m.funcs.iter_mut().find(|f| f.name == func).unwrap_or_else(|| panic!("no function `{func}`"));
    Prove.visit_stmts(&mut f.body);
}

/// Options where only `prove`'s marks remove checks.
fn only_marks(verify_proofs: bool) -> Options {
    Options { remove_checks: false, verify_proofs, ..Options::default() }
}

fn proven_module(src: &str, func: &str) -> Module {
    let mut m = optimized(src);
    prove(&mut m, func);
    m
}

fn checks_left(m: &Module, func: &str) -> usize {
    dump(m, only_marks(false), func).unwrap().matches("trap user2").count()
}

fn jit_proven(m: &Module, verify_proofs: bool) -> JitProgram {
    let p = JitProgram::compile(m, only_marks(verify_proofs)).expect("codegen");
    p.run_init();
    p
}

const ADD: &str = "pure f(x: int) -> int { x + 1 }\nmain() {}\n";

#[test]
fn a_proven_add_has_no_check() {
    assert_eq!(checks_left(&optimized(ADD), "f"), 1, "unproven, it keeps its check");
    let m = proven_module(ADD, "f");
    assert_eq!(checks_left(&m, "f"), 0);
    // With the check gone, the add wraps.
    let p = jit_proven(&m, false);
    assert_eq!(call1(&p, "f", 2147483647), -2147483648);
    assert_eq!(call1(&p, "f", 2), 3);
}

#[test]
fn a_proven_index_has_no_check() {
    let src = "pure g(xs: [int], i: int) -> int { xs[i] }\nmain() {}\n";
    assert_eq!(checks_left(&optimized(src), "g"), 1, "unproven, it keeps its check");
    assert_eq!(checks_left(&proven_module(src, "g"), "g"), 0);
}

#[test]
fn a_proven_division_divides_plainly() {
    let src = "pure d(a: int, b: int) -> int { a / b }\nmain() {}\n";
    assert!(checks_left(&optimized(src), "d") >= 2, "unproven, it checks for zero and for the minimum by -1");
    let m = proven_module(src, "d");
    assert_eq!(checks_left(&m, "d"), 0);
    let f: extern "C" fn(i32, i32) -> i32 = unsafe { std::mem::transmute(jit_proven(&m, false).fn_ptr("d").unwrap()) };
    assert_eq!(f(7, 2), 3);
    assert_eq!(f(-7, 2), -3);
}

#[test]
fn a_proven_operation_inside_another_keeps_the_outer_checks_it_has() {
    // `x * 2` is proven, `- 1` is not: one check is left.
    let src = "pure h(x: int) -> int { x * 2 - 1 }\nmain() {}\n";
    let m = proven_module(src, "h");
    assert_eq!(checks_left(&m, "h"), 1);
    let p = jit_proven(&m, false);
    assert_eq!(call1(&p, "h", 5), 9);
}

const CHILD_ENV: &str = "KESTREL_PROVEN_CHILD";

/// Child half of `verify_mode_still_checks_a_proof`; does nothing unless the parent set `CHILD_ENV`.
#[test]
fn proof_failure_child() {
    if std::env::var_os(CHILD_ENV).is_some() {
        let m = proven_module(ADD, "f");
        let p = jit_proven(&m, true);
        assert_eq!(call1(&p, "f", 2), 3, "a proof that holds passes");
        call1(&p, "f", 2147483647);
        panic!("the failed proof should have ended the process");
    }
}

#[test]
fn verify_mode_still_checks_a_proof() {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["proof_failure_child", "--exact", "--nocapture"])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(101), "{stderr}");
    assert!(stderr.contains("range analysis removed the overflow check on line 1"), "{stderr}");
    assert!(stderr.contains("but it fails; this is a compiler bug"), "{stderr}");
}

#[test]
fn verify_mode_keeps_the_proven_checks_in_the_code() {
    for (src, func, kept) in [
        (ADD, "f", 1),
        ("pure g(xs: [int], i: int) -> int { xs[i] }\nmain() {}\n", "g", 1),
        ("pure d(a: int, b: int) -> int { a / b }\nmain() {}\n", "d", 2),
    ] {
        let m = proven_module(src, func);
        let left = dump(&m, only_marks(true), func).unwrap().matches("trap user2").count();
        assert!(left >= kept, "{func}: {left} checks left when verifying");
    }
}

#[test]
fn define_module_removes_proven_checks() {
    // `(x & 255) * 1000` is at most 255,000: range analysis proves the multiply, and codegen's
    // entry runs it by default.
    let m = optimized("pure h(x: int) -> int { (x & 255) * 1000 }\nmain() {}\n");
    let traps = |opts: Options| dump(&m, opts, "h").unwrap().matches("trap user2").count();
    assert_eq!(traps(Options::default()), 0);
    assert_eq!(traps(Options { remove_checks: false, ..Options::default() }), 1);
    // The module itself is left as it was: the analysis marks a copy.
    assert!(!format!("{:?}", m.funcs).contains("Proven"));
}
