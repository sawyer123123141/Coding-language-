//! `@prove` and `@assume` at the command line: `kestrel check` warns about an assumption that removes
//! no check in either build; `run` and `build` don't, and annotations change nothing that runs.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory holding `files` (name, text).
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("contracts_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        fs::write(dir.join(file), text).unwrap();
    }
    dir
}

fn kestrel(args: &[&str], file: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kestrel")).args(args).arg(file).output().expect("run kestrel")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const UNUSED: &str = "show(n: int) {
    @assume(n > 0)
    print(n)
}

main() { show(4) }
";

#[test]
fn an_assumption_that_removes_nothing_warns() {
    let dir = project("unused", &[("unused.kes", UNUSED)]);
    let file = dir.join("unused.kes");
    for args in [&["check"][..], &["check", "--release"]] {
        let out = kestrel(args, &file);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        assert!(stderr(&out).contains("warning: `@assume(n > 0)` removes no check; delete it"), "{args:?}: {}", stderr(&out));
        assert!(stderr(&out).contains("unused.kes:2:5"), "{}", stderr(&out));
    }
    // `run` doesn't repeat it.
    let out = kestrel(&["run"], &file);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!((stdout(&out).as_str(), stderr(&out).as_str()), ("4\n", ""));
}

#[test]
fn an_assumption_for_overflow_checks_does_not_warn_under_release() {
    // It removes `a * 3`'s overflow check in a checked build; `--release` has none to remove.
    let src = "show(a: int) {\n    @assume(a >= 0 and a <= 1000)\n    print(a * 3)\n}\n\nmain() { show(5) }\n";
    let dir = project("overflow", &[("overflow.kes", src)]);
    for args in [&["check"][..], &["check", "--release"]] {
        let out = kestrel(args, &dir.join("overflow.kes"));
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        assert_eq!(stderr(&out), "", "{args:?}");
    }
}

#[test]
fn a_merged_assumption_warns_at_each_condition() {
    let src = "show(n: int, m: int) {\n    @assume(n > 0) @assume(m > 0)\n    print(n + m)\n}\n\nmain() { show(1, 2) }\n";
    let dir = project("merged", &[("merged.kes", src)]);
    let out = kestrel(&["check", "--diagnostics=short"], &dir.join("merged.kes"));
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        "2:5: warning: `@assume(n > 0)` removes no check; delete it\n2:20: warning: `@assume(m > 0)` removes no check; delete it\n"
    );
}

#[test]
fn the_report_reuses_the_analysis_and_still_warns() {
    let dir = project("report", &[("report.kes", UNUSED)]);
    let out = kestrel(&["check", "--report", "--diagnostics=short"], &dir.join("report.kes"));
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stderr(&out), "2:5: warning: `@assume(n > 0)` removes no check; delete it\n");
    assert!(stdout(&out).starts_with("report.kes\n"), "{}", stdout(&out));
}

#[test]
fn annotations_do_not_change_what_runs() {
    let src = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs/prove_parses.kes")).unwrap();
    let dir = project("same", &[("prove_parses.kes", &src)]);
    let file = dir.join("prove_parses.kes");
    for release in [false, true] {
        let flag: &[&str] = if release { &["--release"] } else { &[] };
        let out = kestrel(&[&["run"][..], flag].concat(), &file);
        assert!(out.status.success(), "run {flag:?}: {}", stderr(&out));
        assert_eq!((stdout(&out).as_str(), stderr(&out).as_str()), ("23\n", ""), "run {flag:?}");
        let exe = dir.join(format!("prove_parses_{release}{}", std::env::consts::EXE_SUFFIX));
        let out = kestrel(&[&["build"][..], flag, &["-o", exe.to_str().unwrap()]].concat(), &file);
        assert!(out.status.success(), "build {flag:?}: {}", stderr(&out));
        let ran = Command::new(&exe).output().expect("run the executable");
        assert!(ran.status.success(), "{}", stderr(&ran));
        assert_eq!(stdout(&ran), "23\n", "build {flag:?}");
    }
}
