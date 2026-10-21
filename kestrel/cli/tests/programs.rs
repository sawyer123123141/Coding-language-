use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod common;
#[path = "programs/directives.rs"]
mod directives;
#[path = "programs/release.rs"]
mod release;

use common::{hermetic, scrub};

/// What a test program declares about itself in `// expect...` comments.
struct Expect {
    stdout: String,
    exit: i32,
    stderr: Vec<String>,
    /// `// env: KEY=VALUE` lines: variables set for both the JIT run and the exe run.
    env: Vec<(String, String)>,
    /// `// asset: <file>` lines: files in `tests/programs` the program reads by a relative path.
    /// The JIT run finds them in place; the exe run gets a copy next to the exe.
    assets: Vec<String>,
    /// `// expect-exit-proofs:` and `// expect-stderr-proofs:`: what a `tests/release` program's
    /// run with the proof check expects instead (a trusted assumption is evaluated there). Its
    /// standard output must be the same in both runs.
    exit_proofs: Option<i32>,
    stderr_proofs: Vec<String>,
}

impl Expect {
    /// What the run with the proof check expects.
    fn with_proofs(&self) -> Expect {
        let stderr = if self.stderr_proofs.is_empty() { self.stderr.clone() } else { self.stderr_proofs.clone() };
        Expect {
            stdout: self.stdout.clone(),
            exit: self.exit_proofs.unwrap_or(self.exit),
            stderr,
            env: self.env.clone(),
            assets: self.assets.clone(),
            exit_proofs: None,
            stderr_proofs: Vec::new(),
        }
    }
}

/// Reads the directives of program `name`. Panics on a malformed or unknown
/// `// expect...`, `// env:` or `// asset:` line so a typo can't silently drop a check.
fn expectations(name: &str, src: &str) -> Expect {
    let mut lines = Vec::new();
    let mut exit = 0;
    let mut stderr = Vec::new();
    let mut env = Vec::new();
    let mut assets = Vec::new();
    let (mut exit_proofs, mut stderr_proofs) = (None, Vec::new());
    for line in src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("// asset:") {
            assets.push(asset_path(name, rest.trim(), line));
        } else if let Some(rest) = line.strip_prefix("// env:") {
            let (key, value) = rest
                .split_once('=')
                .map(|(key, value)| (key.trim(), value.trim()))
                .filter(|(key, _)| !key.is_empty())
                .unwrap_or_else(|| panic!("{name}: bad env directive (expected KEY=VALUE): {line}"));
            env.push((key.to_string(), value.to_string()));
        } else if let Some(rest) = line.strip_prefix("// expect-exit-proofs:") {
            exit_proofs = Some(rest.trim().parse().unwrap_or_else(|_| panic!("{name}: bad expect-exit-proofs number: {line}")));
        } else if let Some(rest) = line.strip_prefix("// expect-stderr-proofs:") {
            stderr_proofs.push(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("// expect-exit:") {
            exit = rest.trim().parse().unwrap_or_else(|_| panic!("{name}: bad expect-exit number: {line}"));
        } else if let Some(rest) = line.strip_prefix("// expect-stderr:") {
            stderr.push(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("// expect:") {
            lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        } else if line.starts_with("// expect") {
            panic!("{name}: unknown expect directive: {line}");
        }
    }
    let headless = env.iter().any(|(key, value)| key == "KESTREL_HEADLESS" && !value.is_empty());
    assert!(
        headless || !opens_a_window(src),
        "{name}: opens a window but has no `// env: KESTREL_HEADLESS=<frames>` line, so the test would open a real window and hang the suite"
    );
    let stdout = lines.iter().map(|l| format!("{l}\n")).collect();
    Expect { stdout, exit, stderr, env, assets, exit_proofs, stderr_proofs }
}

/// The file an `// asset:` line names: a path inside `tests/programs`, so not absolute and not
/// climbing out with `..` (the exe run copies it next to the exe).
fn asset_path(name: &str, file: &str, line: &str) -> String {
    let path = Path::new(file);
    let inside = path.components().all(|part| matches!(part, std::path::Component::Normal(_)));
    assert!(!file.is_empty() && inside, "{name}: bad asset directive (expected a file inside tests/programs): {line}");
    file.to_string()
}

/// Copies each of `assets` from `from` to the same relative path in `to`, creating folders.
fn copy_assets(from: &Path, assets: &[String], to: &Path) {
    for asset in assets {
        let (source, target) = (from.join(asset), to.join(asset));
        fs::create_dir_all(target.parent().expect("an asset path has a folder")).unwrap();
        fs::copy(&source, &target).unwrap_or_else(|e| panic!("asset {asset}: can't copy {}: {e}", source.display()));
    }
}

/// Whether the program calls `window.open` (or `open` imported from `window`), which opens a
/// real window unless `KESTREL_HEADLESS` says otherwise. A text check, not a parse: a comment
/// is skipped, and a call hidden in a string would be a false alarm.
fn opens_a_window(src: &str) -> bool {
    src.lines().map(str::trim).filter(|l| !l.starts_with("//")).any(|l| {
        l.contains("window.open(") || (l.starts_with("use ") && l.contains("open") && l.ends_with(" from window }"))
    })
}

fn programs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs")
}

/// The sentence a failed proof check ends a program with (see `KESTREL_CHECK_PROOFS`).
const PROOF_FAILED: &str = "range analysis removed";

/// The programs whose file names start with one of the comma-separated `prefixes` (empty
/// entries and spaces around a prefix are ignored); all of them for `None`, or when no prefix is
/// left (`KESTREL_TEST_ONLY=` set but empty).
pub(crate) fn only_prefixed(files: Vec<PathBuf>, prefixes: Option<&str>) -> Vec<PathBuf> {
    let Some(prefixes) = prefixes else { return files };
    let prefixes: Vec<&str> = prefixes.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    // Set but empty (`KESTREL_TEST_ONLY=`) is every program, as when it isn't set.
    if prefixes.is_empty() {
        return files;
    }
    files
        .into_iter()
        .filter(|f| {
            let name = f.file_name().unwrap().to_string_lossy();
            prefixes.iter().any(|p| name.starts_with(p))
        })
        .collect()
}

/// Every program in `tests/programs`, or with `KESTREL_TEST_ONLY=a,b` only those whose names
/// start with `a` or `b` (to run a few programs while working on one change).
pub fn programs() -> Vec<PathBuf> {
    let dir = programs_dir();
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "kes"))
        .collect();
    assert!(!files.is_empty(), "no .kes programs in {}", dir.display());
    files.sort();
    let only = std::env::var("KESTREL_TEST_ONLY").ok();
    let files = only_prefixed(files, only.as_deref());
    assert!(!files.is_empty(), "KESTREL_TEST_ONLY={} matches no program in {}", only.unwrap_or_default(), dir.display());
    files
}

fn kestrel(args: &[&str]) -> Output {
    hermetic(Command::new(env!("CARGO_BIN_EXE_kestrel")).args(args)).output().expect("run kestrel")
}

/// Runs `kestrel <path>`'s program with the variables its `// env:` lines set.
fn kestrel_run(path: &Path, exp: &Expect) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    // The proof check is on: the compiler keeps every check it proved, and a failing one fails the run.
    hermetic(cmd.args(["run", path.to_str().unwrap()]))
        .envs(exp.env.iter().map(|(k, v)| (k, v)))
        .env("KESTREL_CHECK_PROOFS", "1");
    cmd.output().expect("run kestrel")
}

/// Compares one run against the program's expectations; describes any mismatch.
fn compare(name: &str, mode: &str, out: &Output, exp: &Expect) -> Option<String> {
    let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n");
    let code = out.status.code().unwrap_or(-1);
    // A run that stopped on a failed proof is a failure whatever else it printed.
    let proof_failed = stderr.contains(PROOF_FAILED);
    let ok = !proof_failed && stdout == exp.stdout && code == exp.exit && exp.stderr.iter().all(|s| stderr.contains(s.as_str()));
    (!ok).then(|| {
        let note = if proof_failed { "a proof failed: " } else { "" };
        format!(
            "{name} [{mode}]: {note}exit {code} (expected {})\n--- stdout ---\n{stdout}--- expected ---\n{}--- stderr ---\n{stderr}",
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
        let exp = expectations(&name, &src);
        let out = kestrel_run(&path, &exp);
        failures.extend(compare(&name, "jit", &out, &exp));
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

#[test]
fn assets_are_copied_with_their_folders() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("asset_copy");
    let _ = fs::remove_dir_all(&root);
    let (from, to) = (root.join("from"), root.join("to").join("deeper"));
    fs::create_dir_all(from.join("sub")).unwrap();
    fs::write(from.join("a.bin"), [1u8, 2, 3]).unwrap();
    fs::write(from.join("sub").join("b.txt"), "bee").unwrap();
    fs::write(from.join("not_listed.txt"), "no").unwrap();
    copy_assets(&from, &["a.bin".to_string(), "sub/b.txt".to_string()], &to);
    assert_eq!(fs::read(to.join("a.bin")).unwrap(), [1, 2, 3]);
    assert_eq!(fs::read_to_string(to.join("sub").join("b.txt")).unwrap(), "bee");
    assert!(!to.join("not_listed.txt").exists());
}

#[test]
#[should_panic(expected = "asset missing.png: ")]
fn a_missing_asset_panics() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("asset_missing");
    fs::create_dir_all(&root).unwrap();
    copy_assets(&root, &["missing.png".to_string()], &root.join("out"));
}

#[test]
#[should_panic(expected = "typo.kes: bad env directive (expected KEY=VALUE): // env: KESTREL_HEADLESS")]
fn env_directive_without_equals_panics() {
    expectations("typo.kes", "// env: KESTREL_HEADLESS\nmain() {}\n");
}

#[test]
#[should_panic(expected = "win.kes: opens a window but has no `// env: KESTREL_HEADLESS=<frames>` line")]
fn a_window_program_without_a_headless_line_panics() {
    expectations("win.kes", "use window\nmain() {\n    window.open(64, 64, \"w\")\n}\n");
}

#[test]
#[should_panic(expected = "win.kes: opens a window but has no `// env: KESTREL_HEADLESS=<frames>` line")]
fn an_imported_open_counts_as_opening_a_window() {
    expectations("win.kes", "use { open from window }\nmain() {\n    open(64, 64, \"w\")\n}\n");
}

#[test]
#[should_panic(expected = "win.kes: opens a window but has no `// env: KESTREL_HEADLESS=<frames>` line")]
fn hidden_alone_is_not_headless() {
    expectations("win.kes", "// env: KESTREL_HIDDEN=1\nuse window\nmain() {\n    window.open(64, 64, \"w\")\n}\n");
}

#[test]
fn headless_window_programs_and_programs_that_never_open_one_are_accepted() {
    let headless = "// env: KESTREL_HEADLESS=3\nuse window\nmain() {\n    window.open(64, 64, \"w\")\n}\n";
    assert_eq!(expectations("win.kes", headless).env, [("KESTREL_HEADLESS".to_string(), "3".to_string())]);
    // Using the module without opening: the natives report the error.
    expectations("early.kes", "use window\nmain() {\n    print(window.fps())\n}\n");
    // A comment that mentions it is not a call.
    expectations("note.kes", "// call window.open(1, 2, \"x\") first\nmain() {}\n");
}

#[test]
fn the_childs_environment_loses_every_kestrel_variable() {
    let mut cmd = Command::new("kestrel");
    let names = ["KESTREL_HEADLESS", "kestrel_hidden", "KESTREL_X", "PATH", "KESTRELS", "MY_KESTREL_HEADLESS"];
    scrub(&mut cmd, names.iter().map(OsString::from));
    let mut removed: Vec<String> = cmd
        .get_envs()
        .map(|(key, value)| {
            assert!(value.is_none(), "{key:?} is removed, not set");
            key.to_string_lossy().into_owned()
        })
        .collect();
    removed.sort();
    assert_eq!(removed, ["KESTREL_HEADLESS", "KESTREL_X", "kestrel_hidden"]);
}

#[test]
fn a_variable_set_after_the_scrub_wins() {
    let mut cmd = Command::new("kestrel");
    scrub(&mut cmd, [OsString::from("KESTREL_HEADLESS")].into_iter());
    cmd.envs([("KESTREL_HEADLESS", "3")]);
    let set: Vec<_> = cmd.get_envs().collect();
    assert_eq!(set, [(OsStr::new("KESTREL_HEADLESS"), Some(OsStr::new("3")))]);
}

fn build(path: &Path, exe: &Path, extra: &[&str]) -> Output {
    build_with(path, exe, extra, false)
}

/// `kestrel build`, with `KESTREL_CHECK_PROOFS=1` when `proofs` is set.
fn build_with(path: &Path, exe: &Path, extra: &[&str], proofs: bool) -> Output {
    let mut args = vec!["build", path.to_str().unwrap(), "-o", exe.to_str().unwrap()];
    args.extend_from_slice(extra);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    hermetic(cmd.args(&args));
    if proofs {
        cmd.env("KESTREL_CHECK_PROOFS", "1");
    }
    cmd.output().expect("run kestrel")
}

#[test]
fn programs_run_as_executables() {
    let out_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("programs");
    fs::create_dir_all(&out_dir).unwrap();
    // `KESTREL_TEST_VERIFY_EXE=1` builds the executables with the proof check on too.
    let proofs = std::env::var_os("KESTREL_TEST_VERIFY_EXE").is_some_and(|v| v == "1");
    let mut failures = Vec::new();
    for path in programs() {
        let src = fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let exp = expectations(&name, &src);
        let exe = out_dir.join(path.file_stem().unwrap()).with_extension(std::env::consts::EXE_EXTENSION);
        let built = build_with(&path, &exe, &[], proofs);
        if !built.status.success() {
            failures.push(format!("{name} [build]: {}", String::from_utf8_lossy(&built.stderr)));
            continue;
        }
        copy_assets(&programs_dir(), &exp.assets, &out_dir);
        let out = hermetic(&mut Command::new(&exe)).envs(exp.env.iter().map(|(k, v)| (k, v))).output().expect("run built exe");
        failures.extend(compare(&name, "exe", &out, &exp));
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn release_builds_wrap_on_overflow() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs/overflow.kes");
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("overflow_release")
        .with_extension(std::env::consts::EXE_EXTENSION);
    let built = build(&path, &exe, &["--release"]);
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let out = hermetic(&mut Command::new(&exe)).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"), "before\nafter\n");
}

#[test]
fn release_builds_still_panic_on_division_by_zero() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs/div_zero.kes");
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("div_zero_release")
        .with_extension(std::env::consts::EXE_EXTENSION);
    let built = build(&path, &exe, &["--release"]);
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let out = hermetic(&mut Command::new(&exe)).output().unwrap();
    assert_eq!(out.status.code(), Some(101));
    assert!(String::from_utf8_lossy(&out.stderr).contains("division by zero on line 5"));
}
