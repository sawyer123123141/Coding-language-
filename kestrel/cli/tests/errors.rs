use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn errors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/errors")
}

fn kestrel(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kestrel")).args(args).output().expect("run kestrel")
}

/// `// error <line>: <message>` comments.
fn expected_errors(src: &str) -> Vec<(u32, String)> {
    src.lines()
        .filter_map(|l| l.trim().strip_prefix("// error "))
        .map(|rest| {
            let (line, msg) = rest.split_once(':').expect("`// error <line>: <message>`");
            (line.trim().parse().expect("line number"), msg.trim().to_string())
        })
        .collect()
}

/// `LINE:COL: error: MESSAGE` lines from `--diagnostics=short`.
fn actual_errors(stderr: &str) -> Vec<(u32, String)> {
    stderr
        .lines()
        .filter_map(|l| {
            let (pos, msg) = l.split_once(": error: ")?;
            let line = pos.split(':').next()?.parse().ok()?;
            Some((line, msg.to_string()))
        })
        .collect()
}

#[test]
fn error_programs_report_exactly_the_expected_errors() {
    let mut files: Vec<PathBuf> = fs::read_dir(errors_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "kes"))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    let mut failures = Vec::new();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = fs::read_to_string(&path).unwrap();
        let out = kestrel(&["check", path.to_str().unwrap(), "--diagnostics=short"]);
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if out.status.code() != Some(1) {
            failures.push(format!("{name}: exit {:?}, expected 1\n{stderr}", out.status.code()));
            continue;
        }
        let mut actual = actual_errors(&stderr);
        for (line, msg) in expected_errors(&src) {
            match actual.iter().position(|(l, m)| *l == line && m.contains(&msg)) {
                Some(i) => {
                    actual.remove(i);
                }
                None => failures.push(format!("{name}: missing error on line {line}: {msg}\n{stderr}")),
            }
        }
        for (line, msg) in actual {
            failures.push(format!("{name}: unexpected error on line {line}: {msg}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn full_diagnostics_show_location_source_and_caret() {
    let path = errors_dir().join("unknown.kes");
    let out = kestrel(&["check", path.to_str().unwrap()]);
    let stderr = String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr.contains("error: unknown name `y`"), "{stderr}");
    assert!(stderr.contains("unknown.kes:4:13"), "{stderr}");
    assert!(stderr.contains(" 4 |     let x = y\n"), "{stderr}");
    assert!(stderr.contains("^"), "{stderr}");
    assert!(stderr.ends_with("2 errors\n"), "{stderr}");
}
