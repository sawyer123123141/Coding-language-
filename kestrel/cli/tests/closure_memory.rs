//! Closures that capture values keep them in a heap environment in the current arena, and a
//! capture is a path (`s.name`), not a whole variable. So storing `() { s.name }` into `s.cb`
//! again and again copies one string each time: the program arena grows in proportion to the
//! number of stores. (Capturing all of `s` would copy the old closure into the new one, nesting
//! the copies, and the growth would be quadratic.)
//!
//! A closure literal passed straight to a call gets a stack environment instead, in the caller's
//! frame: calling `apply((x) { x + k }, i)` any number of times allocates nothing.
//!
//! Memory is measured with the runtime's own report (`KESTREL_ARENA_REPORT=1`: the size of the
//! program arena's 64 KiB blocks when `main` returns).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

use common::hermetic;

const BLOCK: u64 = 64 * 1024;

/// A program that stores a closure capturing `s.name` into `s.cb` `n` times.
fn stores(n: u32) -> String {
    format!(
        "struct S {{\n    name: string\n    cb: () -> string = () {{ \"\" }}\n}}\n\nmain() {{\n    var s = S(name: \"a name long enough that each copy of it is easy to see in the report\")\n    for i from 0 to {n} {{\n        s.cb = () {{ s.name }}\n    }}\n    print(s.cb().len)\n}}\n"
    )
}

/// A program that passes a capturing closure literal straight to a function `n` times.
fn passes(n: u32) -> String {
    format!(
        "apply(f: (int) -> int, i: int) -> int {{ f(i) }}\n\nmain() {{\n    let k = 3\n    let s = \"ab\"\n    var last = 0\n    for i from 0 to {n} {{\n        last = apply((x) {{ x + k + s.len }}, i)\n    }}\n    print(last - {n})\n}}\n"
    )
}

/// Builds `src` as an executable named `name` and gives its path.
fn build(name: &str, src: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("closure_memory");
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join(format!("{name}.kes"));
    fs::write(&source, src).unwrap();
    let exe = dir.join(name).with_extension(std::env::consts::EXE_EXTENSION);
    let out = hermetic(Command::new(env!("CARGO_BIN_EXE_kestrel")).arg("build").arg(&source).arg("-o").arg(&exe)).output().unwrap();
    assert!(out.status.success(), "building {name}:\n{}", String::from_utf8_lossy(&out.stderr));
    exe
}

/// Runs `exe`, checks that it printed `stdout`, and gives the program arena's size it reported.
fn arena_bytes(exe: &Path, stdout: &str) -> u64 {
    let out = hermetic(&mut Command::new(exe)).env("KESTREL_ARENA_REPORT", "1").output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "exit {:?}\n{stderr}", out.status.code());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), stdout, "the closure should give the captured values");
    let line = stderr
        .lines()
        .find_map(|l| l.strip_prefix("kestrel: program arena "))
        .unwrap_or_else(|| panic!("no arena report in stderr:\n{stderr}"));
    line.strip_suffix(" bytes").and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("can't read the arena report {line:?}"))
}

#[test]
fn storing_a_capturing_closure_again_and_again_grows_memory_in_proportion() {
    let few = arena_bytes(&build("stores_1000", &stores(1_000)), "68");
    let many = arena_bytes(&build("stores_10000", &stores(10_000)), "68");
    assert!(many > few + BLOCK, "10,000 stores should take more memory than 1,000: {few} and {many} bytes");
    assert!(many <= 11 * few + BLOCK, "the copies nest: {few} bytes for 1,000 stores, {many} bytes for 10,000");
}

#[test]
fn passing_a_capturing_closure_literal_to_a_call_allocates_nothing() {
    let few = arena_bytes(&build("passes_10", &passes(10)), "4");
    let many = arena_bytes(&build("passes_100000", &passes(100_000)), "4");
    assert_eq!(few, many, "100,000 calls with a closure literal should take no more memory than 10");
}

/// A program that calls `any` and `all` with a capturing literal `n` times in the program arena,
/// and `filter` and `map` in an `arena { }` block that frees their lists each time.
fn list_fns(n: u32) -> String {
    format!(
        "main() {{\n    let xs = [1, 2, 3, 4]\n    let k = 2\n    let s = \"ab\"\n    var last = 0\n    for i from 0 to {n} {{\n        if xs.any((e) {{ e > k + s.len }}) {{ last += 1 }}\n        if xs.all((e) {{ e > k }}) {{ last += 1 }}\n        arena {{\n            last = xs.filter((e) {{ e > k }}).len + xs.map((e) {{ e + k }}).len\n        }}\n    }}\n    print(last)\n}}\n"
    )
}

#[test]
fn the_list_methods_allocate_nothing_for_a_literal_and_what_an_arena_block_frees() {
    let few = arena_bytes(&build("list_fns_10", &list_fns(10)), "6");
    let many = arena_bytes(&build("list_fns_100000", &list_fns(100_000)), "6");
    assert_eq!(few, many, "100,000 calls should take no more memory than 10");
}
