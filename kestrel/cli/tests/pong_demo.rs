//! The rollback Pong demo (`demos/pong`): two peers in one process over a simulated network,
//! run headless (`KESTREL_HEADLESS`, `KESTREL_HEADLESS_QUIET=1`), so no window ever opens. Player
//! 1 is a bot when no key is held and player 2 always is, so a headless run is scripted: the
//! same frames give the same game, in the JIT, a checked exe and a `--release` exe.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod common;

use common::hermetic;

/// The demo's five files. `pong.kes` is the program; the others are its modules.
const FILES: [&str; 5] = ["pong.kes", "game.kes", "net.kes", "peer.kes", "replay.kes"];

/// The demo's folder.
fn demo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../demos/pong")
}

/// A fresh, empty folder for one test's files under the target folder's scratch space. Each test
/// needs its own `name`: tests run in parallel, and the folder is cleared when a test starts.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("pong_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `kestrel` command with a clean environment.
fn kestrel() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    hermetic(&mut cmd);
    cmd
}

/// Sets up a quiet headless window of `frames` frames.
fn headless(cmd: &mut Command, frames: u32) -> &mut Command {
    cmd.env("KESTREL_HEADLESS", frames.to_string()).env("KESTREL_HEADLESS_QUIET", "1")
}

/// Runs `cmd`, which must succeed, and gives its output.
fn ok(cmd: &mut Command, what: &str) -> Output {
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{what}: exit {:?}\n{}", out.status.code(), String::from_utf8_lossy(&out.stderr));
    out
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `kestrel run` of the program at `main` (checks on), headless for `frames` frames.
fn run(main: &Path, frames: u32) -> Output {
    ok(headless(kestrel().arg("run").arg(main), frames), &format!("run {}", main.display()))
}

/// Builds the demo into `dir`, checked or `--release`, and gives the exe's path.
fn build(dir: &Path, release: bool) -> PathBuf {
    let exe = dir.join(format!("pong{}", std::env::consts::EXE_SUFFIX));
    let mut cmd = kestrel();
    cmd.arg("build").arg(demo_dir().join("pong.kes")).arg("-o").arg(&exe);
    if release {
        cmd.arg("--release");
    }
    ok(&mut cmd, if release { "build --release" } else { "build" });
    exe
}

/// The words of the output line that starts with `prefix`, after the prefix.
fn line_after<'a>(out: &'a str, prefix: &str) -> Vec<&'a str> {
    let line = out.lines().find_map(|l| l.strip_prefix(prefix)).unwrap_or_else(|| panic!("no line starting {prefix:?} in:\n{out}"));
    line.split(' ').collect()
}

/// The number at `i` of `words`.
fn number(words: &[&str], i: usize) -> i64 {
    words[i].parse().unwrap_or_else(|_| panic!("expected a number at word {i} of {words:?}"))
}

#[test]
fn pong_runs_headless_and_the_three_builds_agree() {
    let jit = stdout(&run(&demo_dir().join("pong.kes"), 600));
    let checked = stdout(&ok(headless(&mut Command::new(build(&scratch("checked"), false)), 600), "checked exe"));
    let release = stdout(&ok(headless(&mut Command::new(build(&scratch("release"), true)), 600), "release exe"));
    assert_eq!(checked, jit, "the checked exe printed something else than the JIT");
    assert_eq!(release, jit, "the release exe printed something else than the JIT");

    // `frames 600 confirmed <F> checksum <C>`, then `replay matches (<F> frames)`.
    let words = line_after(&jit, "frames 600 confirmed ");
    assert_eq!((words.len(), words[1]), (3, "checksum"), "unexpected frames line in:\n{jit}");
    let confirmed = number(&words, 0);
    number(&words, 2);
    assert!(confirmed > 500 && confirmed <= 600, "confirmed {confirmed} frames of 600:\n{jit}");
    assert!(jit.lines().any(|l| l == format!("replay matches ({confirmed} frames)")), "no matching replay line in:\n{jit}");
    assert!(!jit.contains("differ"), "the peers differ:\n{jit}");
    // The peers really compared their checksums (every 30 frames), not just never found a
    // difference: both got at least to frame 480 of 600.
    let words = line_after(&jit, "checksums compared up to frame ");
    let compared = number(&words, 0);
    assert!((480..=confirmed).contains(&compared), "checksums compared up to frame {compared}, {confirmed} confirmed:\n{jit}");
}

#[test]
fn pong_rolls_back_and_recovers_lost_packets() {
    let out = stdout(&run(&demo_dir().join("pong.kes"), 600));
    let words = line_after(&out, "rollbacks ");
    assert_eq!((words.len(), words[1], words[3]), (5, "deepest", "lost"), "unexpected rollbacks line in:\n{out}");
    let (rollbacks, deepest, lost) = (number(&words, 0), number(&words, 2), number(&words, 4));
    assert!(rollbacks > 0, "no rollback happened:\n{out}");
    assert!((1..=12).contains(&deepest), "the deepest rollback was {deepest} frames:\n{out}");
    assert!(lost > 0, "no packet was lost:\n{out}");
    // Every frame was confirmed in the end except the last few, so the lost packets' inputs
    // came with later packets.
    assert!(out.contains("replay matches ("), "no replay line:\n{out}");
}

#[test]
fn pong_reports_an_injected_desync() {
    let dir = scratch("desync");
    let mut replaced = 0;
    for file in FILES {
        let src = fs::read_to_string(demo_dir().join(file)).unwrap_or_else(|e| panic!("read {file}: {e}"));
        replaced += src.matches("let DESYNC_AT = -1").count();
        fs::write(dir.join(file), src.replace("let DESYNC_AT = -1", "let DESYNC_AT = 300")).unwrap();
    }
    assert_eq!(replaced, 1, "the demo should declare `let DESYNC_AT = -1` once");
    let out = stdout(&run(&dir.join("pong.kes"), 600));
    assert!(out.lines().any(|l| l == "peers differ at frame 330"), "the desync wasn't reported at frame 330:\n{out}");
    assert!(!out.contains("replay matches"), "the run should stop at the desync:\n{out}");
}

/// The program arena's size that a headless run of `frames` frames reports.
fn arena_bytes(frames: u32) -> u64 {
    let out = ok(headless(kestrel().arg("run").arg(demo_dir().join("pong.kes")), frames).env("KESTREL_ARENA_REPORT", "1"), "run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let line = stderr
        .lines()
        .find_map(|l| l.strip_prefix("kestrel: program arena "))
        .unwrap_or_else(|| panic!("no arena report in stderr:\n{stderr}"));
    line.strip_suffix(" bytes").and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("can't read the arena report {line:?}"))
}

#[test]
fn pong_memory_stays_flat() {
    // Everything is a ring or lives in a frame's `arena { }`, except the replay, which keeps
    // every confirmed frame's inputs and a checksum every 60 frames: about 12,400 bytes at 3,000
    // frames (twice that with the buffers its lists left behind when they doubled), which still
    // fit in the arena's first block. So both runs report the same size (65,536 bytes today),
    // and any other growth with the run's length shows.
    let (short, long) = (arena_bytes(600), arena_bytes(3000));
    assert_eq!(long, short, "the program arena grew from {short} bytes (600 frames) to {long} bytes (3,000 frames)");
}

#[test]
fn pong_proves_its_contracts() {
    for release in [false, true] {
        let mut cmd = kestrel();
        cmd.arg("check");
        if release {
            cmd.arg("--release");
        }
        let out = cmd.arg(demo_dir().join("pong.kes")).output().unwrap();
        let what = if release { "check --release" } else { "check" };
        let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "{what}: exit {:?}\n{stderr}", out.status.code());
        assert!(stdout.is_empty() && stderr.is_empty(), "{what} printed:\n{stdout}{stderr}");
    }
}
