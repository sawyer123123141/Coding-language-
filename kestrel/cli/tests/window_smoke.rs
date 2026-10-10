//! The tests that open a real window: draw on the GPU, take a screenshot and check its pixels,
//! under both `kestrel run` (the JIT) and `kestrel build` (an exe).
//!
//! The window is hidden (`KESTREL_HIDDEN=1`) so the test doesn't flash a window at whoever runs
//! it. A hidden window might not render on some machine's driver, so when the hidden run
//! behaved (exit 0, the expected stdout) and only the screenshot's pixels differ, the program
//! runs again with a visible window and the test asserts on that run, saying on stderr which
//! mode passed. Any other failure (a crash, a wrong exit code, wrong output) fails at once: a
//! visible window would not fix it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod common;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// A program to run in a window, what it prints, and what its screenshot must show.
struct Case {
    name: &'static str,
    /// Kestrel source; `SHOT_PATH` stands for where the screenshot goes.
    program: &'static str,
    stdout: &'static str,
    /// The screenshot's width and height.
    size: (i32, i32),
    /// Pixels that must be exactly this colour: where, what colour, and what is there (for the
    /// failure message).
    pixels: &'static [(i32, i32, [u8; 4], &'static str)],
    /// Rectangles `(x, y, width, height, what)` that must hold at least one pixel that isn't black.
    inked: &'static [(i32, i32, i32, i32, &'static str)],
}

/// Three frames of a red square on black, a screenshot on the third, then `close`. After the
/// loop it prints what `frame` and `time` say, so the real backend's behaviour is pinned.
const SQUARE: Case = Case {
    name: "square",
    program: r#"use window
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
"#,
    stdout: "time small true\nframes 3\nframe after close false\n",
    size: (64, 64),
    pixels: &[(32, 32, RED, "the square's centre"), (2, 2, BLACK, "top left corner"), (61, 61, BLACK, "bottom right corner")],
    inked: &[],
};

/// One of each shape on black: a filled circle, a line, a triangle in each winding (raylib draws
/// only one of them as given), a circle outline and some text.
const SHAPES: Case = Case {
    name: "shapes",
    program: r#"use window
use draw

main() {
    window.open(128, 128, "shapes")
    var n = 0
    while window.frame() {
        n += 1
        draw.clear(draw.BLACK)
        draw.circle(vec2(24, 24), 14, draw.GREEN)
        draw.line(vec2(50, 14), vec2(110, 14), 6, draw.BLUE)
        draw.triangle(vec2(10, 50), vec2(10, 80), vec2(40, 80), draw.YELLOW)
        draw.triangle(vec2(60, 50), vec2(90, 80), vec2(60, 80), draw.RED)
        draw.circle_outline(vec2(100, 70), 18, 6, draw.WHITE)
        draw.text("Hello", vec2(8, 96), 24, draw.PURPLE)
        if n == 3 {
            window.screenshot("SHOT_PATH")
            window.close()
        }
    }
    print("frames", n)
}
"#,
    stdout: "frames 3\n",
    size: (128, 128),
    pixels: &[
        (24, 24, GREEN, "the circle's centre"),
        (44, 24, BLACK, "beside the circle"),
        (80, 14, BLUE, "the line's middle"),
        (80, 26, BLACK, "below the line"),
        (20, 70, YELLOW, "inside the counter-clockwise triangle"),
        (30, 55, BLACK, "outside its slanted edge"),
        (70, 70, RED, "inside the clockwise triangle"),
        (85, 55, BLACK, "outside its slanted edge"),
        (115, 70, WHITE, "on the circle outline's ring"),
        (100, 70, BLACK, "the circle outline's hollow middle"),
        (122, 70, BLACK, "outside the ring"),
    ],
    inked: &[(8, 96, 60, 24, "the text")],
};

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

    fn dir_name(self) -> &'static str {
        match self {
            How::Jit => "jit",
            How::Exe => "exe",
        }
    }
}

/// A scratch folder for one test (an ASCII path, so raylib and the Kestrel string literal agree).
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("window_smoke").join(name);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The command that runs the program: `kestrel run <source>`, or the built exe. No `KESTREL_*`
/// variable of the shell reaches it (a stray `KESTREL_HEADLESS` would make the run headless,
/// and a headless run takes no screenshot).
fn command(how: How, source: &Path, exe: &Path) -> Command {
    let mut cmd = match how {
        How::Jit => {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
            cmd.arg("run").arg(source);
            cmd
        }
        How::Exe => Command::new(exe),
    };
    common::hermetic(&mut cmd);
    cmd
}

/// Runs the program with `window`.
fn run(how: How, window: Window, source: &Path, exe: &Path, shot: &Path) -> Output {
    let _ = fs::remove_file(shot);
    let mut cmd = command(how, source, exe);
    if matches!(window, Window::Hidden) {
        cmd.env("KESTREL_HIDDEN", "1");
    }
    cmd.output().expect("run the program")
}

/// Whether the program exited 0 and printed what it should.
fn ran_right(out: &Output, case: &Case) -> bool {
    out.status.success() && String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n") == case.stdout
}

/// Compares the screenshot with what the case expects; says what differs.
fn check_shot(shot: &Path, case: &Case) -> Result<(), String> {
    if !shot.exists() {
        // (raylib logs a warning to stdout when it can't open a file, so don't ask it for a missing one.)
        return Err("the program wrote no screenshot".to_string());
    }
    let (w, h) = case.size;
    // The last pixel is inside, one past it in either direction is outside: the exact size.
    let mut points = vec![(w - 1, h - 1), (w, h - 1), (w - 1, h)];
    points.extend(case.pixels.iter().map(|&(x, y, _, _)| (x, y)));
    let rects = || case.inked.iter().flat_map(|&(x, y, w, h, _)| (y..y + h).flat_map(move |py| (x..x + w).map(move |px| (px, py))));
    points.extend(rects());
    let Some(read) = kestrel_window::png_pixels(shot, &points) else { return Err("the screenshot isn't a readable image".to_string()) };

    let mut wrong = Vec::new();
    if read[0].is_none() || read[1].is_some() || read[2].is_some() {
        wrong.push(format!("the screenshot isn't {w}x{h}"));
    }
    let mut next = 3;
    for &(x, y, want, what) in case.pixels {
        if read[next] != Some(want) {
            wrong.push(format!("({x}, {y}), {what}: want {want:?}, got {:?}", read[next]));
        }
        next += 1;
    }
    for &(x, y, w, h, what) in case.inked {
        let area = (w * h) as usize;
        if !read[next..next + area].iter().flatten().any(|p| p[..3] != [0, 0, 0]) {
            wrong.push(format!("({x}, {y}) {w}x{h}, {what}: only black pixels"));
        }
        next += area;
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(wrong.join("\n"))
    }
}

/// Describes a run for a failure message.
fn describe(how: How, window: Window, out: &Output) -> String {
    format!(
        "{} with a {} window: exit {:?}\n--- stdout ---\n{}--- stderr ---\n{}",
        how.name(),
        window.name(),
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    )
}

/// Writes the program, builds it if `how` says to, then runs it with a hidden window, trying a
/// visible one only if the hidden run behaved and just its pixels were off.
fn smoke(how: How, case: &Case) {
    let dir = scratch(&format!("{}_{}", case.name, how.dir_name()));
    let shot = dir.join("shot.png");
    let source = dir.join("smoke.kes");
    fs::write(&source, case.program.replace("SHOT_PATH", &shot.to_str().unwrap().replace('\\', "/"))).unwrap();
    let exe = dir.join("smoke").with_extension(std::env::consts::EXE_EXTENSION);
    if matches!(how, How::Exe) {
        let mut build = Command::new(env!("CARGO_BIN_EXE_kestrel"));
        let built = common::hermetic(build.arg("build").arg(&source).arg("-o").arg(&exe)).output().unwrap();
        assert!(built.status.success(), "building the program:\n{}", String::from_utf8_lossy(&built.stderr));
    }

    let out = run(how, Window::Hidden, &source, &exe, &shot);
    assert!(ran_right(&out, case), "{}", describe(how, Window::Hidden, &out));
    let hidden = match check_shot(&shot, case) {
        Ok(()) => {
            eprintln!("window smoke test ({}): {} passed with a hidden window", case.name, how.name());
            return;
        }
        Err(why) => why,
    };
    eprintln!("window smoke test ({}): the hidden window's pixels were wrong, trying a visible one:\n{hidden}", case.name);
    let out = run(how, Window::Visible, &source, &exe, &shot);
    assert!(ran_right(&out, case), "{}", describe(how, Window::Visible, &out));
    if let Err(why) = check_shot(&shot, case) {
        panic!("{} with a visible window drew the wrong pixels:\n{why}", how.name());
    }
    eprintln!("window smoke test ({}): {} passed with a visible window", case.name, how.name());
}

#[test]
fn a_real_window_draws_under_the_jit() {
    smoke(How::Jit, &SQUARE);
}

#[test]
fn a_real_window_draws_in_an_exe() {
    smoke(How::Exe, &SQUARE);
}

#[test]
fn every_shape_and_text_draw_under_the_jit() {
    smoke(How::Jit, &SHAPES);
}

#[test]
fn every_shape_and_text_draw_in_an_exe() {
    smoke(How::Exe, &SHAPES);
}
