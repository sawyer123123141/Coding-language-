//! The `window` module's natives (`driver/std/window.kes`).
//!
//! Each one is `kestrel_window_<name>`. Strings arrive as a pointer to their
//! header, and `size` writes its `vec2` through the pointer it is given. Every result is
//! recorded and replayed (`kestrel_runtime::replay`).

use std::path::Path;

use kestrel_runtime::panic::fail_at_call;
use kestrel_runtime::replay::{self, record_or_play, NativeId};

use crate::abi::{str_bytes, write, KStr, V2};
use crate::gpu;
use crate::headless::{self, Arg};
use crate::image_file;
use crate::pass;
use crate::paths;
use crate::raylib as rl;
use crate::real;
use crate::state::{self, Mode, Settings, View};

/// # Safety
/// `title` must point to a live string header.
#[no_mangle]
pub unsafe extern "C" fn kestrel_window_open(width: i32, height: i32, title: *const KStr) {
    // A replay's window is the replay backend whatever the environment says (`state::open`).
    let settings = match replay::window_mode_for_replay() {
        Some(_) => Settings { headless: None, hidden: false, quiet: true },
        None => Settings::from_env().unwrap_or_else(|msg| fail_at_call(&msg)),
    };
    open_with(&settings, width, height, title);
}

/// `window.open` with the environment's settings already read.
///
/// # Safety
/// `title` must point to a live string header.
pub unsafe fn open_with(settings: &Settings, width: i32, height: i32, title: *const KStr) {
    // Dropping a texture or mesh, on any thread, queues it for the window's thread (see `gpu.rs`).
    kestrel_runtime::gpu::set_drop_hook(gpu::queue_drop);
    let title = str_bytes(title);
    match state::open(settings, width, height, title) {
        Ok(Mode::Headless) => headless::line("window.open", &[Arg::Int(width), Arg::Int(height), Arg::Str(title)]),
        Ok(Mode::Real) => {}
        Err(msg) => fail_at_call(&msg),
    }
}

#[no_mangle]
pub extern "C" fn kestrel_window_frame() -> u8 {
    // `print` is buffered until the program ends. A program that runs frames for
    // minutes would show nothing meanwhile, so show it once per frame.
    kestrel_runtime::print::flush();
    replay::frame(|| state::frame().unwrap_or_else(|msg| fail_at_call(&msg))) as u8
}

#[no_mangle]
pub extern "C" fn kestrel_window_close() {
    match state::close().unwrap_or_else(|msg| fail_at_call(&msg)) {
        Some(Mode::Headless) => headless::line("window.close", &[]),
        Some(Mode::Real) | None => {}
    }
}

#[no_mangle]
pub extern "C" fn kestrel_window_delta() -> f32 {
    record_or_play(NativeId::WindowDelta, || match state::view("window.delta") {
        // SAFETY: the window is open.
        View::Real { .. } => unsafe { rl::GetFrameTime() },
        View::Fixed(values) => values.delta,
    })
}

#[no_mangle]
pub extern "C" fn kestrel_window_time() -> f32 {
    record_or_play(NativeId::WindowTime, || match state::view("window.time") {
        View::Real { time_base } => real::time(time_base),
        View::Fixed(values) => values.time,
    })
}

/// Microseconds since `window.open`. A headless window counts 60 frames a second.
#[no_mangle]
pub extern "C" fn kestrel_window_ticks_usec() -> i64 {
    record_or_play(NativeId::WindowTicksUsec, || match state::view("window.ticks_usec") {
        View::Real { time_base } => real::ticks(time_base),
        View::Fixed(values) => values.ticks,
    })
}

/// # Safety
/// `out` must be valid for writing a `vec2`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_window_size(out: *mut V2) {
    let [x, y] = record_or_play(NativeId::WindowSize, || {
        let size = match state::view("window.size") {
            View::Real { .. } => real::size(),
            View::Fixed(values) => values.size,
        };
        [size.x, size.y]
    });
    write(out, V2 { x, y });
}

#[no_mangle]
pub extern "C" fn kestrel_window_fps() -> i32 {
    record_or_play(NativeId::WindowFps, || match state::view("window.fps") {
        // SAFETY: the window is open.
        View::Real { .. } => unsafe { rl::GetFPS() },
        View::Fixed(values) => values.fps,
    })
}

/// # Safety
/// `path` must point to a live string header.
#[no_mangle]
pub unsafe extern "C" fn kestrel_window_screenshot(path: *const KStr) {
    let path = str_bytes(path);
    let mode = state::drawing("window.screenshot");
    // A 3D pass ends first, which draws what the pass had batched.
    pass::enter_2d(mode == Mode::Real);
    match mode {
        Mode::Headless => headless::line("window.screenshot", &[Arg::Str(path)]),
        Mode::Real => {
            let resolved = paths::resolve(path);
            if !save_screenshot(&resolved) {
                fail_at_call(&screenshot_error(path, &resolved));
            }
        }
    }
}

/// The error for a screenshot that couldn't be saved: the path as the program wrote it, and where
/// it was tried, since a relative path doesn't go where the program was run from.
fn screenshot_error(path: &[u8], resolved: &Path) -> String {
    format!(
        "window.screenshot couldn't save {} (tried {}); give a path that ends in .png, in a folder that exists",
        String::from_utf8_lossy(path),
        paths::display(resolved)
    )
}

/// Saves what has been drawn so far this frame to the PNG file `path`.
/// `false` if the path doesn't end in `.png` or the file couldn't be written.
fn save_screenshot(path: &Path) -> bool {
    if image_file::file_type(path) != Some(".png") {
        return false;
    }
    // SAFETY: the window is open. The image is freed once, after it is encoded.
    let png = unsafe {
        // Shapes are batched: send the ones drawn so far, or the capture would miss them.
        rl::rlDrawRenderBatchActive();
        let image = rl::LoadImageFromScreen();
        let png = image_file::encode_png(image);
        rl::UnloadImage(image);
        png
    };
    // Rust writes the file, because raylib's own `fopen` can't open every Windows path.
    png.is_some_and(|bytes| std::fs::write(path, bytes).is_ok())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn a_failed_screenshot_names_the_path_and_where_it_was_tried() {
        // A real failed save can't run in the headless suite, so check the message itself.
        let resolved: PathBuf = Path::new("game").join("shots").join("a.png");
        assert_eq!(
            screenshot_error(b"shots/a.png", &resolved),
            "window.screenshot couldn't save shots/a.png (tried game/shots/a.png); \
             give a path that ends in .png, in a folder that exists"
        );
    }

    #[test]
    fn only_a_png_path_is_saved_and_the_rest_fail_before_touching_the_screen() {
        // No window is open here; a path that isn't a PNG's is refused before the capture.
        for name in ["shot.jpg", "shot.bmp", "shot.tga", "shot", "shot.png.txt"] {
            assert!(!save_screenshot(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn a_path_that_is_not_utf8_is_still_named() {
        let message = screenshot_error(b"bad\xff.png", Path::new("bad.png"));
        assert!(message.starts_with("window.screenshot couldn't save bad\u{fffd}.png (tried bad.png); "), "{message}");
    }
}
