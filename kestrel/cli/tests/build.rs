//! `kestrel build` argument handling.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SRC: &str = "main() {\n    print(1)\n}\n";

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn build(src: &Path, out: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kestrel")).arg("build").arg(src).arg("-o").arg(out).output().unwrap()
}

#[test]
fn build_refuses_to_overwrite_the_source() {
    let dir = scratch("build_overwrite");
    let src = dir.join("prog.kes");
    fs::write(&src, SRC).unwrap();
    let out = build(&src, &src);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is the source file"));
    assert_eq!(fs::read_to_string(&src).unwrap(), SRC);
}

#[test]
fn build_refuses_another_spelling_of_the_source_path() {
    let dir = scratch("build_overwrite_alias");
    fs::create_dir(dir.join("sub")).unwrap();
    let src = dir.join("prog.kes");
    fs::write(&src, SRC).unwrap();
    let alias = dir.join("sub").join("..").join("prog.kes");
    let out = build(&src, &alias);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(fs::read_to_string(&src).unwrap(), SRC);
}

#[test]
fn build_to_a_different_path_still_works() {
    let dir = scratch("build_ok");
    let src = dir.join("prog.kes");
    fs::write(&src, SRC).unwrap();
    let exe = dir.join(format!("prog{}", std::env::consts::EXE_SUFFIX));
    let out = build(&src, &exe);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(exe.is_file());
}
