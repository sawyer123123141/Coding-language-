//! The one test that opens a real window: draws on the GPU, takes a screenshot and checks its
//! pixels, under both `kestrel run` (the JIT) and `kestrel build` (an exe).
//!
//! The window is hidden (`KESTREL_HIDDEN=1`) so the test doesn't flash a window at whoever runs
//! it. If a hidden window doesn't read back right on some machine, the test runs again with a
//! visible window and asserts on that run, and says on stderr which mode passed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Three frames of a red square on black, a screenshot on the third, then `close`. After the
/// loop it prints what `frame` and `time` say, so the real backend's behaviour is pinned.
const PROGRAM: &str = r#"use window
use draw

main() {
    window.open(64, 64, "smoke")
    var n = 0
    while window.frame() {
        n += 1
        draw.clear(draw.BLACK)
        draw.rect(vec2(16, 16), vec2(32, 32), draw.RED)
        if n == 3 {
            window.screenshot("SHOT_PATH")
            print("time small", window.time() >= 0 and window.time() < 5)
            window.close()
        }
    }
    print("frames", n)
    print("frame after close", window.frame())
}
"#;

/// The lines the program prints when it ran as designed.
const EXPECTED_STDOUT: &str = "time small true\nframes 3\nframe after close false\n";

const RED: [u8; 4] = [255, 0, 0, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// Which window a run used.
#[derive(Clone, Copy)]
enum Window {
    Hidden,
    Visible,
}

impl Window {
    fn name(self) -> &'static str {
        match self {
            Window::Hidden => "hidden",
            Window::Visible => "visible",
        }
    }
}

/// How a run starts the program: under the JIT, or as a built exe.
#[derive(Clone, Copy)]
enum How {
    Jit,
    Exe,
}

impl How {
    fn name(self) -> &'static str {
        match self {
            How::Jit => "kestrel run",
            How::Exe => "kestrel build",
        }
    }
}

/// A scratch folder for one test (an ASCII path, so raylib and the Kestrel string literal agree).
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("window_smoke").join(name);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The command that runs the program: `kestrel run <source>`, or the built exe.
fn command(how: How, source: &Path, exe: &Path) -> Command {
    match how {
        How::Jit => {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
            cmd.arg("run").arg(source);
            cmd
        }
        How::Exe => Command::new(exe),
    }
}

/// Runs the program with `window`, giving its output and the screenshot's pixels (the centre of
/// the square, and two corners). A pixel is `None` if the screenshot is missing or too small.
fn run(how: How, window: Window, source: &Path, exe: &Path, shot: &Path) -> (Output, [Option<[u8; 4]>; 3]) {
    let _ = fs::remove_file(shot);
    let mut cmd = command(how, source, exe);
    // Anything inherited would make the run headless, and a headless run takes no screenshot.
    cmd.env_remove("KESTREL_HEADLESS").env_remove("KESTREL_HEADLESS_QUIET").env_remove("KESTREL_HIDDEN");
    if matches!(window, Window::Hidden) {
        cmd.env("KESTREL_HIDDEN", "1");
    }
    let out = cmd.output().expect("run the program");
    // (raylib logs a warning to stdout when it can't open a file, so don't ask it for a missing one.)
    let pixel = |x, y| shot.exists().then(|| kestrel_window::png_pixel(shot, x, y)).flatten();
    let pixels = [pixel(32, 32), pixel(2, 2), pixel(61, 61)];
    (out, pixels)
}

/// Whether the run drew a red square on black and printed what it should.
fn passed(out: &Output, pixels: &[Option<[u8; 4]>; 3]) -> bool {
    out.status.success()
        && String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n") == EXPECTED_STDOUT
        && *pixels == [Some(RED), Some(BLACK), Some(BLACK)]
}

/// Describes a run for a failure message.
fn describe(how: How, window: Window, out: &Output, pixels: &[Option<[u8; 4]>; 3]) -> String {
    format!(
        "{} with a {} window: exit {:?}\n--- stdout ---\n{}--- stderr ---\n{}--- pixels (centre, top left, bottom right; want {:?}, {:?}, {:?}) ---\n{:?}",
        how.name(),
        window.name(),
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        Some(RED),
        Some(BLACK),
        Some(BLACK),
        pixels,
    )
}

/// Writes the program, builds it if `how` says to, then runs it with a hidden window, falling back
/// to a visible one if the hidden run didn't pass.
fn smoke(how: How, name: &str) {
    let dir = scratch(name);
    let shot = dir.join("shot.png");
    let source = dir.join("smoke.kes");
    fs::write(&source, PROGRAM.replace("SHOT_PATH", &shot.to_str().unwrap().replace('\\', "/"))).unwrap();
    let exe = dir.join("smoke").with_extension(std::env::consts::EXE_EXTENSION);
    if matches!(how, How::Exe) {
        let built = Command::new(env!("CARGO_BIN_EXE_kestrel")).arg("build").arg(&source).arg("-o").arg(&exe).output().unwrap();
        assert!(built.status.success(), "building the program:\n{}", String::from_utf8_lossy(&built.stderr));
    }

    let (out, pixels) = run(how, Window::Hidden, &source, &exe, &shot);
    if passed(&out, &pixels) {
        eprintln!("window smoke test: {} passed with a hidden window", how.name());
        return;
    }
    eprintln!("window smoke test: the hidden window did not pass, trying a visible one:\n{}", describe(how, Window::Hidden, &out, &pixels));
    let (out, pixels) = run(how, Window::Visible, &source, &exe, &shot);
    assert!(passed(&out, &pixels), "{}", describe(how, Window::Visible, &out, &pixels));
    eprintln!("window smoke test: {} passed with a visible window", how.name());
}

#[test]
fn a_real_window_draws_under_the_jit() {
    smoke(How::Jit, "jit");
}

#[test]
fn a_real_window_draws_in_an_exe() {
    smoke(How::Exe, "exe");
}
