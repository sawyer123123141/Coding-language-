//! The release pass of the program tests: every program under `kestrel run --release` with the
//! proof check on, and the tests of the harness pieces that go with it.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use super::common::hermetic;
use super::{build_with, compare, expectations, only_prefixed, programs, Expect, PROOF_FAILED};

/// How long the release pass lets one program run: a program whose checked run stops on an
/// overflow may loop when it wraps.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs `kestrel run --release` on the program with the proof check on, killing it after
/// `RELEASE_TIMEOUT`. `None` when it timed out.
fn kestrel_run_release(path: &Path, exp: &Expect) -> Option<Output> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    hermetic(cmd.args(["run", "--release", path.to_str().unwrap()]))
        .envs(exp.env.iter().map(|(k, v)| (k, v)))
        .env("KESTREL_CHECK_PROOFS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("run kestrel");
    // Read both pipes on threads, so a program that prints a lot can't block on a full pipe.
    fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    }
    let (stdout, stderr) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for kestrel") {
            break Some(status);
        }
        if start.elapsed() > RELEASE_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (stdout, stderr) = (stdout.join().unwrap(), stderr.join().unwrap());
    status.map(|status| Output { status, stdout, stderr })
}

/// The release pass: every program under `kestrel run --release` with the proof check on. The
/// checked expectations don't hold there (an overflow wraps instead of stopping), so it fails only
/// when a run's standard error has the failed-proof sentence. A run that takes over 30 seconds is
/// killed (a program that stops on an overflow when checked may loop when it wraps); it doesn't
/// fail the test, but it is listed (see `timeouts_note`).
#[test]
fn programs_verify_release() {
    let mut failures = Vec::new();
    let mut timeouts = Vec::new();
    let mut ran = 0;
    for path in programs() {
        let src = fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let exp = expectations(&name, &src);
        ran += 1;
        match kestrel_run_release(&path, &exp) {
            None => timeouts.push(name),
            Some(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n");
                if stderr.contains(PROOF_FAILED) {
                    failures.push(format!("{name} [release]: a proof failed\n--- stderr ---\n{stderr}"));
                }
            }
        }
    }
    let note = timeouts_note(ran, &timeouts);
    // Written to the real standard error: libtest captures `eprintln!` from a test that passes, so
    // a timeout would otherwise never be seen.
    let _ = writeln!(std::io::stderr(), "{note}");
    assert!(failures.is_empty(), "\n{}\n{note}", failures.join("\n"));
}

/// The release pass's summary line: how many programs ran, and which were stopped after
/// `RELEASE_TIMEOUT` (their proofs were checked only as far as they got).
fn timeouts_note(ran: usize, timeouts: &[String]) -> String {
    let secs = RELEASE_TIMEOUT.as_secs();
    match timeouts {
        [] => format!("release pass: {ran} programs, none stopped after {secs} seconds"),
        _ => format!("release pass: {ran} programs; stopped after {secs} seconds: {}", timeouts.join(", ")),
    }
}

#[test]
fn the_release_summary_names_the_programs_that_timed_out() {
    assert_eq!(timeouts_note(3, &[]), "release pass: 3 programs, none stopped after 30 seconds");
    let names = ["a.kes".to_string(), "b.kes".to_string()];
    assert_eq!(timeouts_note(3, &names), "release pass: 3 programs; stopped after 30 seconds: a.kes, b.kes");
}

/// A finished run with these streams and exit code, without running anything.
fn fake_output(code: i32, stdout: &str, stderr: &str) -> Output {
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;
    #[cfg(windows)]
    let status = std::process::ExitStatus::from_raw(code as u32);
    #[cfg(unix)]
    let status = std::process::ExitStatus::from_raw(code << 8);
    Output { status, stdout: stdout.as_bytes().to_vec(), stderr: stderr.as_bytes().to_vec() }
}

#[test]
fn a_run_whose_stderr_names_a_failed_proof_fails_the_comparison() {
    let exp = expectations("p.kes", "// expect: hi\nmain() {}\n");
    assert_eq!(compare("p.kes", "jit", &fake_output(0, "hi\n", ""), &exp), None);
    let failed = "kestrel: range analysis removed the bounds check on line 3 of p.kes, but it fails; this is a compiler bug\n";
    let report = compare("p.kes", "jit", &fake_output(0, "hi\n", failed), &exp).expect("the proof failure is reported");
    assert!(report.contains("p.kes [jit]: a proof failed: ") && report.contains("range analysis removed"), "{report}");
    // Even a program that expects that stderr and exit fails: the sentence is never expected.
    let exp = expectations("p.kes", "// expect-exit: 101\n// expect-stderr: range analysis removed\nmain() {}\n");
    assert!(compare("p.kes", "jit", &fake_output(101, "", failed), &exp).is_some());
}

#[test]
fn test_only_keeps_the_programs_whose_names_start_with_a_prefix() {
    let files: Vec<PathBuf> = ["array_too_long.kes", "arith.kes", "hello.kes", "range_keep_a.kes"].map(PathBuf::from).to_vec();
    let names = |picked: Vec<PathBuf>| -> Vec<String> { picked.iter().map(|p| p.to_string_lossy().into_owned()).collect() };
    assert_eq!(names(only_prefixed(files.clone(), None)).len(), 4, "unset keeps all");
    assert_eq!(names(only_prefixed(files.clone(), Some("array_too"))), ["array_too_long.kes"]);
    assert_eq!(names(only_prefixed(files.clone(), Some("ar"))), ["array_too_long.kes", "arith.kes"]);
    assert_eq!(names(only_prefixed(files.clone(), Some("hello, range_keep"))), ["hello.kes", "range_keep_a.kes"]);
    assert_eq!(names(only_prefixed(files.clone(), Some("hello,,"))), ["hello.kes"], "empty entries are ignored");
    assert!(only_prefixed(files.clone(), Some("ello")).is_empty(), "a prefix, not a substring");
    assert_eq!(names(only_prefixed(files.clone(), Some(""))).len(), 4, "set but empty keeps all");
    assert_eq!(names(only_prefixed(files, Some(" , "))).len(), 4, "no prefix at all keeps all");
}

/// The programs in `tests/release`, which only fail as they should when built with `--release`
/// (with `KESTREL_TEST_ONLY`, those whose names start with one of its prefixes).
fn release_programs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/release");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "kes"))
        .collect();
    files.sort();
    only_prefixed(files, std::env::var("KESTREL_TEST_ONLY").ok().as_deref())
}

/// Every `tests/release` program built with `--release` and run, compared as
/// `programs_run_as_executables` does; then built again with the proof check on.
#[test]
fn release_programs_run_as_executables() {
    let out_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("release_programs");
    fs::create_dir_all(&out_dir).unwrap();
    let mut failures = Vec::new();
    for path in release_programs() {
        let src = fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let exp = expectations(&name, &src);
        for proofs in [false, true] {
            let mode = if proofs { "release exe, proofs checked" } else { "release exe" };
            let exe = out_dir.join(path.file_stem().unwrap()).with_extension(std::env::consts::EXE_EXTENSION);
            let built = build_with(&path, &exe, &["--release"], proofs);
            if !built.status.success() {
                failures.push(format!("{name} [{mode}, build]: {}", String::from_utf8_lossy(&built.stderr)));
                continue;
            }
            let out = hermetic(&mut Command::new(&exe)).envs(exp.env.iter().map(|(k, v)| (k, v))).output().expect("run built exe");
            let proofs_exp;
            let exp = if proofs {
                proofs_exp = exp.with_proofs();
                &proofs_exp
            } else {
                &exp
            };
            failures.extend(compare(&name, mode, &out, exp));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
