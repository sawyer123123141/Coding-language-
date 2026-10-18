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
