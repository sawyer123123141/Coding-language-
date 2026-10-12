mod common;

use kestrel_codegen::{dump, Options};

const RELEASE: Options = Options { checked_arith: false };

#[test]
fn dump_shows_the_ir_and_the_machine_code() {
    let m = common::optimized("pure sq(x: int) -> int { x * x }\nmain() { print(sq(3)) }\n");
    let code = dump(&m, RELEASE, "sq").unwrap();
    assert!(code.contains("function") && code.contains("imul") && code.contains("ret"), "{code}");
}

#[test]
fn dump_names_a_missing_function() {
    let m = common::optimized("main() {}\n");
    assert_eq!(dump(&m, RELEASE, "nope").unwrap_err(), "no function named `nope`");
}
