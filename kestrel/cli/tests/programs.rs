use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// What a test program declares about itself in `// expect...` comments.
struct Expect {
    stdout: String,
    exit: i32,
    stderr: Vec<String>,
}

fn expectations(src: &str) -> Expect {
    let mut lines = Vec::new();
    let mut exit = 0;
    let mut stderr = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("// expect-exit:") {
            exit = rest.trim().parse().expect("expect-exit number");
        } else if let Some(rest) = line.strip_prefix("// expect-stderr:") {
            stderr.push(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("// expect:") {
            lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
    }
    let stdout = lines.iter().map(|l| format!("{l}\n")).collect();
    Expect { stdout, exit, stderr }
}

pub fn programs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "kes"))
        .collect();
    files.sort();
    files
}

fn kestrel(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kestrel")).args(args).output().expect("run kestrel")
}

/// Compares one run against the program's expectations; describes any mismatch.
fn compare(name: &str, mode: &str, out: &Output, exp: &Expect) -> Option<String> {
    let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n");
    let code = out.status.code().unwrap_or(-1);
    let ok = stdout == exp.stdout && code == exp.exit && exp.stderr.iter().all(|s| stderr.contains(s.as_str()));
    (!ok).then(|| {
        format!(
            "{name} [{mode}]: exit {code} (expected {})\n--- stdout ---\n{stdout}--- expected ---\n{}--- stderr ---\n{stderr}",
            exp.exit, exp.stdout
        )
    })
}

#[test]
fn programs_run_via_jit() {
    let mut failures = Vec::new();
    for path in programs() {
        let src = fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let out = kestrel(&["run", path.to_str().unwrap()]);
        failures.extend(compare(&name, "jit", &out, &expectations(&src)));
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn check_command_reports_success() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs/hello.kes");
    let out = kestrel(&["check", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
}
