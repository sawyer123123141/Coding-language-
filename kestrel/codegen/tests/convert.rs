mod common;

use common::{call1, jit_with};
use kestrel_codegen::Options;

const NARROW: &str = "to_u8(x: int) -> int {\n int(u8(x))\n}\nto_int(x: int) -> int {\n let big = i64(x) * 3000000\n int(big)\n}\nmain() {}\n";

#[test]
fn narrowing_keeps_values_that_fit() {
    let p = jit_with(NARROW, Options::default());
    assert_eq!(call1(&p, "to_u8", 200), 200);
    assert_eq!(call1(&p, "to_int", 100), 300_000_000);
}

#[test]
fn unchecked_narrowing_truncates() {
    let p = jit_with(NARROW, Options { checked_arith: false, ..Options::default() });
    assert_eq!(call1(&p, "to_u8", 300), 44);
    assert_eq!(call1(&p, "to_u8", -1), 255);
    assert_eq!(call1(&p, "to_int", 1000), 3_000_000_000_i64 as i32);
}
