//! The `window` module's natives (`driver/std/window.kes`).
//!
//! Each one is `kestrel_window_<name>`. Strings arrive as a pointer to their
//! header, and `size` writes its `vec2` through the pointer it is given.

use kestrel_runtime::panic::fail_at_call;

use crate::abi::{c_string, str_bytes, write, KStr, V2};
use crate::gpu;
use crate::headless::{self, Arg};
use crate::paths;
use crate::raylib as rl;
use crate::real;
use crate::state::{self, Mode, Settings, View};

/// # Safety
/// `title` must point to a live string header.
#[no_mangle]
pub unsafe extern "C" fn kestrel_window_open(width: i32, height: i32, title: *const KStr) {
    let settings = Settings::from_env().unwrap_or_else(|msg| fail_at_call(&msg));
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
    state::frame().unwrap_or_else(|msg| fail_at_call(&msg)) as u8
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
    match state::view("window.delta") {
        // SAFETY: the window is open.
        View::Real { .. } => unsafe { rl::GetFrameTime() },
        View::Fixed(values) => values.delta,
    }
}

#[no_mangle]
pub extern "C" fn kestrel_window_time() -> f32 {
    match state::view("window.time") {
        View::Real { time_base } => real::time(time_base),
        View::Fixed(values) => values.time,
    }
}

/// Microseconds since `window.open`. A headless window counts 60 frames a second.
#[no_mangle]
pub extern "C" fn kestrel_window_ticks_usec() -> i64 {
    match state::view("window.ticks_usec") {
        View::Real { time_base } => real::ticks(time_base),
        View::Fixed(values) => values.ticks,
    }
}

/// # Safety
/// `out` must be valid for writing a `vec2`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_window_size(out: *mut V2) {
    let size = match state::view("window.size") {
        View::Real { .. } => real::size(),
        View::Fixed(values) => values.size,
    };
    write(out, size);
}

#[no_mangle]
pub extern "C" fn kestrel_window_fps() -> i32 {
    match state::view("window.fps") {
        // SAFETY: the window is open.
        View::Real { .. } => unsafe { rl::GetFPS() },
        View::Fixed(values) => values.fps,
    }
}

/// # Safety
/// `path` must point to a live string header.
#[no_mangle]
pub unsafe extern "C" fn kestrel_window_screenshot(path: *const KStr) {
    let path = str_bytes(path);
    match state::drawing("window.screenshot") {
        Mode::Headless => headless::line("window.screenshot", &[Arg::Str(path)]),
        Mode::Real => {
            if !save_screenshot(path) {
                fail_at_call(&format!(
                    "window.screenshot could not save \"{}\"; give a path that ends in .png, in a folder that exists",
                    String::from_utf8_lossy(path)
                ));
            }
        }
    }
}

/// Saves what has been drawn so far this frame, to `path` read the way `paths::resolve` reads it.
/// `false` if raylib couldn't write the file.
fn save_screenshot(path: &[u8]) -> bool {
    let path = c_string(paths::resolve(path).to_string_lossy().as_bytes());
    // SAFETY: the window is open. `path` is a NUL-terminated string, and the image
    // is freed once, after it is exported.
    unsafe {
        // Shapes are batched: send the ones drawn so far, or the capture would miss them.
        rl::rlDrawRenderBatchActive();
        let image = rl::LoadImageFromScreen();
        let saved = rl::ExportImage(image, path.as_ptr());
        rl::UnloadImage(image);
        saved
    }
}
