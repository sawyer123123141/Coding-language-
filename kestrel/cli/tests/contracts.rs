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

/// `pub f(x: int) -> int` with `depth` nested `if`s around `y = x * 2`: past the 256 levels range
/// analysis goes down, so it gives the function up. `head` goes before the nesting.
fn deep(name: &str, head: &str, depth: usize) -> String {
    let mut src = format!("pub {name}(x: int) -> int {{\n    var y = 0\n{head}");
    for d in 0..depth {
        src.push_str(&format!("if x > {d} {{\n"));
    }
    src.push_str("y = x * 2\n");
    for _ in 0..depth {
        src.push_str("}\n");
    }
    src.push_str("y\n}\n");
    src
}

#[test]
fn release_contracts_use_the_release_build() {
    // `a * b` keeps its overflow check in a checked build; `--release` has none.
    let src = "@prove(no_checks)\npure m(a: int, b: int) -> int { a * b }\n\nmain() { print(m(6, 7)) }\n";
    let dir = project("release", &[("release.kes", src)]);
    let file = dir.join("release.kes");
    let out = kestrel(&["check", "--diagnostics=short"], &file);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).starts_with("1:8: error: `m` can't be proven `no_checks`: it has an overflow check on line 2: `a * b` may exceed `int`"), "{}", stderr(&out));
    let out = kestrel(&["check", "--release"], &file);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stderr(&out), "");
    let exe = dir.join(format!("release{}", std::env::consts::EXE_SUFFIX));
    let out = kestrel(&["build", "--release", "-o", exe.to_str().unwrap()], &file);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&Command::new(&exe).output().expect("run the executable")), "42\n");
    // A checked run fails the contract and runs nothing.
    let out = kestrel(&["run"], &file);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("can't be proven `no_checks`"), "{}", stderr(&out));
}

#[test]
fn the_report_prints_when_contracts_fail() {
    let src = "@prove(no_alloc)\npub grow(var ys: list[int]) {\n    ys.add(1)\n}\n\nmain() {}\n";
    let dir = project("report_fails", &[("grow.kes", src)]);
    let out = kestrel(&["check", "--report", "--diagnostics=short"], &dir.join("grow.kes"));
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "1:8: error: `grow` can't be proven `no_alloc`: it allocates on line 3: `add` may grow `ys`\n");
    assert!(stdout(&out).starts_with("grow.kes\n  function `grow` (line 2): no checks, 1 allocation, 1 other way to fail\n"), "{}", stdout(&out));
}

/// Runs `kestrel check --diagnostics=short` on `src` (in a directory named after `name`).
fn check_short(name: &str, src: &str) -> Output {
    let dir = project(name, &[(&format!("{name}.kes"), src)]);
    kestrel(&["check", "--diagnostics=short"], &dir.join(format!("{name}.kes")))
}

#[test]
fn a_function_the_analysis_gave_up_on_fails_every_contract() {
    let src = format!("@prove(no_alloc, inline)\n{}\nmain() {{}}\n", deep("f", "", 300));
    let out = check_short("gave_up", &src);
    assert_eq!(out.status.code(), Some(1));
    let said = "can't be proven `{}`: range analysis stopped early in it (its code nests too deeply), so nothing can be proven about it; split it into smaller functions";
    for (col, c) in [(8, "no_alloc"), (18, "inline")] {
        let want = format!("1:{col}: error: `f` {}", said.replace("{}", c));
        assert!(stderr(&out).contains(&want), "{want}\n{}", stderr(&out));
    }
}

#[test]
fn a_callee_given_up_on_fails_its_callers() {
    // `g` is far too big to inline, so `f` calls it.
    let src = format!("{}\n@prove(no_checks)\npub f(x: int) -> int {{\n    g(x ^ 1)\n}}\n\nmain() {{}}\n", deep("g", "", 300));
    let line = src.lines().count() - 3;
    let out = check_short("callee_gave_up", &src);
    assert_eq!(out.status.code(), Some(1));
    let want = format!("error: `f` can't be proven `no_checks`: it calls `g` (line {line}), where range analysis stopped early (its code nests too deeply)\n");
    assert!(stderr(&out).ends_with(&want), "{want}\n{}", stderr(&out));
}

#[test]
fn an_inlined_copy_in_a_caller_given_up_on_fails() {
    // `get`'s own copy holds (its assumption removes the bounds check); its copy inside `big`,
    // which the analysis gives up on, keeps every check.
    let get = "@prove(no_checks)\npure get(xs: list[int], i: int) -> int {\n    @assume(i >= 0 and i < xs.len)\n    xs[i]\n}\n";
    let big = deep("big", "    let xs: list[int] = [1, 2, 3]\n    y = get(xs, x)\n", 300);
    let src = format!("{get}\n{big}\nmain() {{}}\n");
    let out = check_short("copy_gave_up", &src);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let want = "1:8: error: `get` can't be proven `no_checks` where it is inlined into `big` (line 10): range analysis stopped early in `big` (its code nests too deeply)";
    assert!(stderr(&out).starts_with(want), "{want}\n{}", stderr(&out));
}

#[test]
fn full_diagnostics_show_the_help() {
    let src = "interface Shape {\n    area(self) -> int\n}\n\n@prove(no_alloc)\npub size(s: Shape) -> int { s.area() }\n\n@prove(no_panic)\npub apply(f: (int) -> int) -> int { f(1) }\n\npub fill(var xs: list[int]) {\n    @prove(no_alloc)\n    par for x in xs {\n        x = 1\n    }\n}\n\nmain() {}\n";
    let dir = project("help", &[("help.kes", src)]);
    let out = kestrel(&["check"], &dir.join("help.kes"));
    assert_eq!(out.status.code(), Some(1));
    let text = stderr(&out).replace("\r\n", "\n");
    for help in [
        "= help: add `@prove(no_alloc)` to the interface method `Shape.area`; then every implementation must keep it\n",
        "= help: contracts can't see into a function value yet; call a named function or an interface method instead\n",
        "= help: a `par for` makes an arena for each thread; put `@prove(no_alloc)` on the function its body calls\n",
    ] {
        assert!(text.contains(help), "{help}\n{text}");
    }
    assert!(text.ends_with("3 errors\n"), "{text}");
}

#[test]
fn a_chain_into_another_file_names_it() {
    let geom = "pub grow(var ys: list[int], n: int) {\n    if n <= 0 {\n        return\n    }\n    ys.add(n)\n}\n";
    let main = "use { grow from prove_geom }\n\n@prove(no_alloc)\npub step(var ys: list[int]) {\n    grow(var ys, 3)\n}\n\nmain() {}\n";
    let dir = project("other_file", &[("prove_other_file.kes", main), ("prove_geom.kes", geom)]);
    let out = kestrel(&["check", "--diagnostics=short"], &dir.join("prove_other_file.kes"));
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "3:8: error: `step` can't be proven `no_alloc`: it calls `grow` (line 5), which adds to `ys` (prove_geom.kes:5) and may grow the list\n");
}
