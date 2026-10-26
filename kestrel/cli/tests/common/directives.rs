//! The `// ...` lines a test program declares about itself (`// expect:`, `// env:` and so on),
//! and the comparison of a run against them. Shared by the program tests (`programs.rs`) and the
//! voxel engine's tests (`voxel_engine.rs`).

use std::path::{Component, Path};
use std::process::Output;

/// What a test program declares about itself in its `// ...` comments.
#[derive(Clone)]
pub struct Directives {
    pub stdout: String,
    pub exit: i32,
    pub stderr: Vec<String>,
    /// `// env: KEY=VALUE` lines: variables set for both the JIT run and the exe run.
    pub env: Vec<(String, String)>,
    /// `// asset: <file>` lines: files in `tests/programs` the program reads by a relative path.
    /// The JIT run finds them in place; the exe run gets a copy next to the exe.
    pub assets: Vec<String>,
    /// `// expect-exit-proofs:` and `// expect-stderr-proofs:`: what a `tests/release` program's
    /// run with the proof check expects instead (a trusted assumption is evaluated there). Its
    /// standard output must be the same in both runs.
    pub exit_proofs: Option<i32>,
    pub stderr_proofs: Vec<String>,
    /// `// copy: <repo-relative file>` lines: files copied into the folder the program runs in.
    /// Only the voxel engine's tests run programs beside modules, so `programs.rs` refuses these.
    pub copies: Vec<String>,
    /// `// replace: <file> <old> => <new>` lines, done in order on the copies in that folder.
    pub replaces: Vec<Replace>,
    /// `// release`: the program is also run with `--release`.
    pub release: bool,
    /// `// only-jit`: the program is too slow as an executable, so only the JIT run does it.
    pub only_jit: bool,
}

/// A literal text replacement in one file of the folder a program runs in.
#[derive(Clone, Debug, PartialEq)]
pub struct Replace {
    pub file: String,
    pub old: String,
    pub new: String,
}

impl Directives {
    /// What the run with the proof check expects.
    pub fn with_proofs(&self) -> Directives {
        let stderr = if self.stderr_proofs.is_empty() { self.stderr.clone() } else { self.stderr_proofs.clone() };
        Directives { exit: self.exit_proofs.unwrap_or(self.exit), stderr, exit_proofs: None, stderr_proofs: Vec::new(), ..self.clone() }
    }
}

/// Reads the directives of program `name`. Panics on a malformed or unknown directive, or on a
/// comment that looks like a misspelt one (`near_miss`), so a typo can't silently drop a check.
pub fn parse(name: &str, src: &str) -> Directives {
    let mut lines = Vec::new();
    let mut d = Directives {
        stdout: String::new(),
        exit: 0,
        stderr: Vec::new(),
        env: Vec::new(),
        assets: Vec::new(),
        exit_proofs: None,
        stderr_proofs: Vec::new(),
        copies: Vec::new(),
        replaces: Vec::new(),
        release: false,
        only_jit: false,
    };
    for line in src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("// asset:") {
            d.assets.push(inside_path(name, rest.trim(), line, "asset", "tests/programs"));
        } else if let Some(rest) = line.strip_prefix("// copy:") {
            d.copies.push(inside_path(name, rest.trim(), line, "copy", "the repo"));
        } else if let Some(rest) = line.strip_prefix("// replace:") {
            d.replaces.push(replacement(name, rest, line));
        } else if line == "// release" {
            d.release = true;
        } else if line == "// only-jit" {
            d.only_jit = true;
        } else if let Some(rest) = line.strip_prefix("// env:") {
            let (key, value) = rest
                .split_once('=')
                .map(|(key, value)| (key.trim(), value.trim()))
                .filter(|(key, _)| !key.is_empty())
                .unwrap_or_else(|| panic!("{name}: bad env directive (expected KEY=VALUE): {line}"));
            d.env.push((key.to_string(), value.to_string()));
        } else if let Some(rest) = line.strip_prefix("// expect-exit-proofs:") {
            d.exit_proofs = Some(rest.trim().parse().unwrap_or_else(|_| panic!("{name}: bad expect-exit-proofs number: {line}")));
        } else if let Some(rest) = line.strip_prefix("// expect-stderr-proofs:") {
            d.stderr_proofs.push(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("// expect-exit:") {
            d.exit = rest.trim().parse().unwrap_or_else(|_| panic!("{name}: bad expect-exit number: {line}"));
        } else if let Some(rest) = line.strip_prefix("// expect-stderr:") {
            d.stderr.push(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("// expect:") {
            lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        } else if line.starts_with("// expect") {
            panic!("{name}: unknown expect directive: {line}");
        } else if let Some(known) = near_miss(line) {
            panic!("{name}: unknown directive `{line}`; did you mean `// {known}`? (fix the spelling, or reword the comment)");
        }
    }
    let headless = d.env.iter().any(|(key, value)| key == "KESTREL_HEADLESS" && !value.is_empty());
    assert!(
        headless || !opens_a_window(src),
        "{name}: opens a window but has no `// env: KESTREL_HEADLESS=<frames>` line, so the test would open a real window and hang the suite"
    );
    d.stdout = lines.iter().map(|l| format!("{l}\n")).collect();
    d
}

/// The directive a comment line looks like a misspelling of, if any, so a typo such as
/// `// relase` fails instead of silently dropping the release pass. Two shapes are checked:
/// a comment that is one word (`// release`, `// only-jit`) and one whose first word ends in a
/// colon (`// copy:`, `// replace:`, `// env:`, `// asset:`). A word counts as a misspelling when
/// it differs from the directive by one or two letters (one for the short `copy`, `env` and
/// `asset`), so ordinary comments never match.
fn near_miss(line: &str) -> Option<&'static str> {
    const ALONE: [&str; 2] = ["release", "only-jit"];
    const COLON: [&str; 4] = ["asset", "copy", "replace", "env"];
    let rest = line.strip_prefix("// ")?;
    let mut words = rest.split_whitespace();
    let first = words.next()?;
    if let Some(word) = first.strip_suffix(':') {
        if COLON.contains(&word) {
            return None;
        }
        return COLON.into_iter().find(|k| edits(word, k) <= if k.len() <= 5 { 1 } else { 2 }).map(|k| match k {
            "asset" => "asset:",
            "copy" => "copy:",
            "replace" => "replace:",
            _ => "env:",
        });
    }
    if words.next().is_some() || ALONE.contains(&first) {
        return None;
    }
    ALONE.into_iter().find(|k| edits(first, k) <= 2)
}

/// How many one-letter insertions, deletions or substitutions turn `a` into `b`.
fn edits(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for j in 0..b.len() {
            let here = row[j + 1];
            row[j + 1] = (here + 1).min(row[j] + 1).min(prev + usize::from(ca != b[j]));
            prev = here;
        }
    }
    row[b.len()]
}

/// The file a `// asset:` or `// copy:` line names: a path inside `base` (said in the message),
/// so not absolute and not climbing out with `..`.
fn inside_path(name: &str, file: &str, line: &str, directive: &str, base: &str) -> String {
    let path = Path::new(file);
    let inside = path.components().all(|part| matches!(part, Component::Normal(_)));
    assert!(!file.is_empty() && inside, "{name}: bad {directive} directive (expected a file inside {base}): {line}");
    file.to_string()
}

/// The `<file> <old> => <new>` of a `// replace:` line. The text is literal and runs from after
/// the file name to the ` => `; the new text may be empty (`... =>` at the end of the line).
fn replacement(name: &str, rest: &str, line: &str) -> Replace {
    let (file, body) = rest.trim_start().split_once(char::is_whitespace).unwrap_or_else(|| bad_replace(name, line));
    let body = body.trim_start();
    let (old, new) = body.split_once(" => ").or_else(|| body.strip_suffix(" =>").map(|old| (old, ""))).unwrap_or_else(|| bad_replace(name, line));
    if file.is_empty() || old.is_empty() {
        bad_replace(name, line);
    }
    Replace { file: inside_path(name, file, line, "replace", "the program's folder"), old: old.to_string(), new: new.to_string() }
}

fn bad_replace(name: &str, line: &str) -> ! {
    panic!("{name}: bad replace directive (expected <file> <old text> => <new text>): {line}")
}

/// Whether the program calls `window.open` (or `open` imported from `window`), which opens a
/// real window unless `KESTREL_HEADLESS` says otherwise. A text check, not a parse: a comment
/// is skipped, and a call hidden in a string would be a false alarm.
fn opens_a_window(src: &str) -> bool {
    src.lines().map(str::trim).filter(|l| !l.starts_with("//")).any(|l| {
        l.contains("window.open(") || (l.starts_with("use ") && l.contains("open") && l.ends_with(" from window }"))
    })
}

/// The sentence a failed proof check ends a program with (see `KESTREL_CHECK_PROOFS`).
pub const PROOF_FAILED: &str = "range analysis removed";

/// The files whose names start with one of the comma-separated `prefixes` (empty entries and
/// spaces around a prefix are ignored); all of them for `None`, or when no prefix is left
/// (`KESTREL_TEST_ONLY=` set but empty).
pub fn only_prefixed(files: Vec<std::path::PathBuf>, prefixes: Option<&str>) -> Vec<std::path::PathBuf> {
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

/// Compares one run against the program's expectations; describes any mismatch.
pub fn compare(name: &str, mode: &str, out: &Output, exp: &Directives) -> Option<String> {
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
