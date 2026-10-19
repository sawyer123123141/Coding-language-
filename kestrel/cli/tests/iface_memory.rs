//! A struct passed straight to an interface parameter is borrowed: the interface value points at
//! the caller's variable, so the call allocates nothing. Calling `show(g)` 100,000 times leaves the
//! program arena the size 10 calls leave it.
//!
//! Memory is measured with the runtime's own report (`KESTREL_ARENA_REPORT=1`: the size of the
//! program arena's blocks when `main` returns).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

use common::hermetic;

/// A program that passes a struct to an interface parameter `n` times.
fn passes(n: u32) -> String {
    format!(
        "interface Damageable {{\n    pure hp(self) -> int\n}}\n\nstruct Goblin: Damageable {{\n    health: int = 10\n    name: string = \"a goblin with a name\"\n    pure hp(self) -> int {{ self.health + self.name.len }}\n}}\n\nshow(d: Damageable) -> int {{ d.hp() }}\n\nmain() {{\n    var g = Goblin()\n    var total = 0\n    for i from 0 to {n} {{\n        total += show(g)\n    }}\n    print(total / {n})\n}}\n"
    )
}

/// Builds `src` as an executable named `name` and gives its path.
fn build(name: &str, src: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("iface_memory");
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
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), stdout);
    let line = stderr
        .lines()
        .find_map(|l| l.strip_prefix("kestrel: program arena "))
        .unwrap_or_else(|| panic!("no arena report in stderr:\n{stderr}"));
    line.strip_suffix(" bytes").and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("can't read the arena report {line:?}"))
}

#[test]
fn passing_a_struct_to_an_interface_parameter_allocates_nothing() {
    let few = arena_bytes(&build("show_10", &passes(10)), "30");
    let many = arena_bytes(&build("show_100000", &passes(100_000)), "30");
    assert_eq!(few, many, "100,000 calls with a borrowed interface value should take no more memory than 10");
}
