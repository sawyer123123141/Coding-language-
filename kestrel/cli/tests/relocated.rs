//! `kestrel build` finds the runtime library next to the executable, so a
//! copied binary keeps working.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const LIB_FILE: &str = if cfg!(windows) { "kestrel_runtime.lib" } else { "libkestrel_runtime.a" };

/// A fresh directory holding only a copy of the `kestrel` binary.
fn relocated_binary(name: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(format!("kestrel{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_kestrel"), &exe).unwrap();
    (dir, exe)
}

fn build_hello(kestrel: &Path, dir: &Path) -> Output {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs/hello.kes");
    let out = dir.join(format!("hello{}", std::env::consts::EXE_SUFFIX));
    Command::new(kestrel).arg("build").arg(src).arg("-o").arg(out).output().unwrap()
}

#[test]
fn copied_binary_falls_back_to_the_library_it_was_built_with() {
    let (dir, kestrel) = relocated_binary("relocated_alone");
    let out = build_hello(&kestrel, &dir);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn copied_binary_uses_the_library_next_to_it() {
    let (dir, kestrel) = relocated_binary("relocated_with_lib");
    fs::copy(env!("KESTREL_RUNTIME_LIB"), dir.join(LIB_FILE)).unwrap();
    let out = build_hello(&kestrel, &dir);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn library_next_to_the_binary_wins_over_the_built_in_path() {
    let (dir, kestrel) = relocated_binary("relocated_bad_lib");
    fs::write(dir.join(LIB_FILE), b"not a library").unwrap();
    let out = build_hello(&kestrel, &dir);
    assert!(!out.status.success(), "the bogus library next to the binary should have been linked");
    assert!(String::from_utf8_lossy(&out.stderr).contains("linking failed"));
}
