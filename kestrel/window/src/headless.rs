//! Headless mode: no window, and each call prints one line instead of drawing.
//!
//! A line is the qualified name (`draw.circle`), then each argument after a
//! space, printed the way `print` prints it: numbers as Rust's `{}` does,
//! a `vec2` as `vec2(x: 1, y: 2)`, a colour as `Color(r: 1, g: 0.3, b: 0.3, a: 1)`
//! and a string in double quotes. The lines go through the runtime's print
//! functions, so they stay in order with the program's own `print` output.
//!
//! With `KESTREL_HEADLESS_QUIET=1` no line is printed at all: a program that
//! draws thousands of times a frame can run headless without flooding its output.

use std::cell::Cell;

use kestrel_runtime::print::{
    kestrel_rt_print_f32, kestrel_rt_print_i64, kestrel_rt_print_newline, kestrel_rt_print_space, kestrel_rt_print_str,
};

use crate::abi::{KColor, V2};

/// Frames per second the headless clock pretends to run at.
pub const FPS: i32 = 60;

/// One argument of a printed call.
#[derive(Clone, Copy)]
pub enum Arg<'a> {
    Int(i32),
    Float(f32),
    Str(&'a [u8]),
    Vec2(V2),
    Color(KColor),
}

thread_local! {
    /// Set when the headless window opens; the window and its lines belong to one thread.
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// Turns the lines off (or on) for this thread's window.
pub fn set_quiet(quiet: bool) {
    QUIET.set(quiet);
}

/// Prints `name` and its arguments on one line, unless the window is quiet.
pub fn line(name: &str, args: &[Arg]) {
    if QUIET.get() {
        return;
    }
    text(name.as_bytes());
    for arg in args {
        kestrel_rt_print_space();
        match *arg {
            Arg::Int(v) => kestrel_rt_print_i64(i64::from(v)),
            Arg::Float(v) => kestrel_rt_print_f32(v),
            Arg::Str(bytes) => {
                text(b"\"");
                text(bytes);
                text(b"\"");
            }
            Arg::Vec2(v) => vec2(v),
            Arg::Color(c) => color(c),
        }
    }
    kestrel_rt_print_newline();
}

fn text(bytes: &[u8]) {
    // SAFETY: `bytes` is a live slice. The runtime ignores a zero length.
    unsafe { kestrel_rt_print_str(bytes.as_ptr(), bytes.len() as i64) }
}

fn field(label: &[u8], v: f32) {
    text(label);
    kestrel_rt_print_f32(v);
}

fn vec2(v: V2) {
    field(b"vec2(x: ", v.x);
    field(b", y: ", v.y);
    text(b")");
}

fn color(c: KColor) {
    field(b"Color(r: ", c.r);
    field(b", g: ", c.g);
    field(b", b: ", c.b);
    field(b", a: ", c.a);
    text(b")");
}

/// How wide `draw.text_width` reports a text in headless mode, where there is
/// no font to measure: half the size for each character. It only has to be
/// the same on every run.
pub fn text_width(text: &[u8], size: f32) -> f32 {
    String::from_utf8_lossy(text).chars().count() as f32 * size * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestrel_runtime::print::{capture_output, take_output};

    /// Runs `body` on a fresh thread, because the print sink is thread-local.
    fn captured(body: impl FnOnce() + Send + 'static) -> String {
        std::thread::spawn(move || {
            capture_output();
            body();
            String::from_utf8(take_output()).unwrap()
        })
        .join()
        .unwrap()
    }

    #[test]
    fn a_line_prints_the_name_then_each_argument() {
        let out = captured(|| {
            line(
                "draw.circle",
                &[Arg::Vec2(V2 { x: 400.0, y: 300.0 }), Arg::Float(20.0), Arg::Color(KColor { r: 1.0, g: 0.3, b: 0.3, a: 1.0 })],
            );
            line("window.open", &[Arg::Int(320), Arg::Int(-1), Arg::Str(b"hi")]);
            line("window.close", &[]);
            line("draw.text", &[Arg::Str(b""), Arg::Float(0.016666668)]);
        });
        assert_eq!(
            out,
            "draw.circle vec2(x: 400, y: 300) 20 Color(r: 1, g: 0.3, b: 0.3, a: 1)\n\
             window.open 320 -1 \"hi\"\n\
             window.close\n\
             draw.text \"\" 0.016666668\n"
        );
    }

    #[test]
    fn a_quiet_window_prints_no_lines() {
        let out = captured(|| {
            set_quiet(true);
            line("draw.clear", &[]);
            line("window.close", &[]);
            set_quiet(false);
            line("draw.clear", &[]);
        });
        assert_eq!(out, "draw.clear\n");
    }

    #[test]
    fn the_headless_text_width_counts_characters() {
        assert_eq!(text_width(b"", 10.0), 0.0);
        assert_eq!(text_width(b"abcde", 10.0), 25.0);
        assert_eq!(text_width("héllo".as_bytes(), 10.0), 25.0, "characters, not bytes");
    }
}
