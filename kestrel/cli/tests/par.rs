//! `par for` gives the same results however many threads run it.
use std::path::Path;
use std::process::Command;

fn run(file: &str, threads: Option<&str>) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs").join(file);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    cmd.arg("run").arg(&path).env("KESTREL_POISON", "1");
    match threads {
        Some(t) => cmd.env("KESTREL_THREADS", t),
        None => cmd.env_remove("KESTREL_THREADS"),
    };
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn one_thread_and_many_agree() {
    let all = run("par_for.kes", None);
    assert_eq!(run("par_for.kes", Some("1")), all);
    assert_eq!(run("par_for.kes", Some("3")), all);
}

/// Runs a program that fails and returns its exit code and standard error.
fn run_failing(file: &str, threads: &str) -> (Option<i32>, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs").join(file);
    let out = Command::new(env!("CARGO_BIN_EXE_kestrel")).arg("run").arg(&path).env("KESTREL_THREADS", threads).output().unwrap();
    (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn the_lowest_failing_iteration_reports_at_any_thread_count() {
    let want = "kestrel: index 1020 out of bounds for length 1000 on line 31 of par_for_lowest_failure.kes\n";
    for threads in ["1", "2", "8", "16"] {
        for run in 0..5 {
            let (code, err) = run_failing("par_for_lowest_failure.kes", threads);
            assert_eq!(code, Some(101), "threads {threads}, run {run}: {err}");
            assert_eq!(err, want, "threads {threads}, run {run}");
        }
    }
}
