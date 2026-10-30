//! The `time`, `args` and `env` modules (`driver/std/`), run through `kestrel run` (the JIT) and
//! through a built executable. Each test has its own scratch folder under the target folder's
//! scratch space, so tests running in parallel never share a program or an executable.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod common;

use common::hermetic;

/// A fresh, empty folder `name` under the scratch space, cleared first.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("small_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `kestrel` command with a clean environment (and freed memory poisoned, see `hermetic`).
fn kestrel() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    hermetic(&mut cmd);
    cmd
}

/// Runs `cmd`, which must exit 0, and gives its standard output.
fn ok(cmd: &mut Command, what: &str) -> String {
    let out: Output = cmd.output().unwrap();
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{what}: exit {:?}\n{stdout}\n{stderr}", out.status.code());
    stdout.into_owned()
}

/// A program written as `prog.kes` in folder `name`, and built into `prog.exe` beside it.
struct Program {
    source: PathBuf,
    exe: PathBuf,
}

impl Program {
    fn new(name: &str, src: &str) -> Program {
        let dir = scratch(name);
        let source = dir.join("prog.kes");
        fs::write(&source, src).unwrap();
        let exe = dir.join(format!("prog{}", std::env::consts::EXE_SUFFIX));
        ok(kestrel().arg("build").arg(&source).arg("-o").arg(&exe), "kestrel build");
        Program { source, exe }
    }

    /// `kestrel run prog.kes <words>`, with `env` set on the child, and its output.
    fn run(&self, words: &[&str], env: &[(&str, &str)]) -> String {
        let mut cmd = kestrel();
        cmd.arg("run").arg(&self.source).args(words).envs(env.iter().copied());
        ok(&mut cmd, "kestrel run")
    }

    /// The built executable with `words`, with `env` set on the child, and its output.
    fn exe(&self, words: &[&str], env: &[(&str, &str)]) -> String {
        let mut cmd = Command::new(&self.exe);
        hermetic(&mut cmd).args(words).envs(env.iter().copied());
        ok(&mut cmd, "the built program")
    }
}

const ARGS: &str = r#"use args
main() {
    let all = args.all()
    print(all.len)
    for a in all {
        print("[{a}]")
    }
    print(args.program())
}
"#;

/// The lines a run of `ARGS` prints for `words`, without the last (the program's name), and that name.
fn args_and_name(out: &str) -> (Vec<&str>, &str) {
    let mut lines: Vec<&str> = out.lines().collect();
    let name = lines.pop().expect("the program's name");
    (lines, name)
}

#[test]
fn args_reach_the_program() {
    let program = Program::new("args", ARGS);
    let expected = ["2", "[a]", "[b]"];

    let out = program.run(&["a", "b"], &[]);
    let (lines, name) = args_and_name(&out);
    assert_eq!(lines, expected, "kestrel run: {out}");
    assert!(name.ends_with("prog.kes"), "kestrel run: {out}");
    assert_eq!(Path::new(name), program.source, "kestrel run: the path as it was given");

    let out = program.exe(&["a", "b"], &[]);
    let (lines, name) = args_and_name(&out);
    assert_eq!(lines, expected, "the built program: {out}");
    assert!(name.ends_with(&format!("prog{}", std::env::consts::EXE_SUFFIX)), "the built program: {out}");
}

#[test]
fn the_compilers_flags_stay_with_it_and_dashes_pass_everything_on() {
    let program = Program::new("args_flags", ARGS);
    // `--release` is the compiler's; after `--` every word is the program's, even `--release`.
    let out = program.run(&["a", "--release", "", "--", "--release", "-o", "x y"], &[]);
    let (lines, _) = args_and_name(&out);
    assert_eq!(lines, ["5", "[a]", "[]", "[--release]", "[-o]", "[x y]"], "{out}");

    let out = program.run(&[], &[]);
    let (lines, _) = args_and_name(&out);
    assert_eq!(lines, ["0"], "{out}");

    // An executable gets every word, with no flags of its own.
    let out = program.exe(&["--release", "--"], &[]);
    let (lines, _) = args_and_name(&out);
    assert_eq!(lines, ["2", "[--release]", "[--]"], "{out}");
}

#[test]
fn env_get_reads_a_variable() {
    let src = r#"use env
main() {
    let v = env.get("KES_TEST_VAR") else {
        print("unset")
        return
    }
    print("[{v}]")
    print(env.get("KES_TEST_UNSET_VAR"), env.get(""), env.get("A=B"))
}
"#;
    let program = Program::new("env", src);
    for (what, out) in [
        ("kestrel run", program.run(&[], &[("KES_TEST_VAR", "héllo world")])),
        ("the built program", program.exe(&[], &[("KES_TEST_VAR", "héllo world")])),
    ] {
        assert_eq!(out, "[héllo world]\nnone none none\n", "{what}");
    }
    assert_eq!(program.run(&[], &[]), "unset\n");
    assert_eq!(program.exe(&[], &[]), "unset\n");
}

#[test]
fn time_now_increases_and_sleep_waits() {
    let src = r#"use time
main() {
    let t1 = time.now()
    time.sleep(0.05)
    let t2 = time.now()
    print(t1 >= 0.0, t2 - t1 >= 0.05)
    // A negative or NaN wait returns at once.
    time.sleep(-1.0)
    let zero: f64 = 0
    time.sleep(zero / zero)
    print(time.now() - t2 < 1.0)
}
"#;
    let program = Program::new("time", src);
    assert_eq!(program.run(&[], &[]), "true true\ntrue\n");
    assert_eq!(program.exe(&[], &[]), "true true\ntrue\n");
}

#[test]
fn time_unix_is_recent() {
    let src = "use time\nmain() {\n    print(time.unix() > 1700000000)\n}\n";
    let program = Program::new("unix", src);
    assert_eq!(program.run(&[], &[]), "true\n");
    assert_eq!(program.exe(&[], &[]), "true\n");
}
