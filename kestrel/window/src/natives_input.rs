//! The `input` module's natives (`driver/std/input.kes`).
//!
//! Each one is `kestrel_input_<name>`. A `Key` arrives as a pointer to its tag
//! byte (see `keys.rs`). A headless window, and a window that has closed, read
//! as nothing held, nothing pressed, the mouse at (0, 0), no mouse movement and no scroll,
//! and `capture` does nothing there. Every result is recorded and replayed (`kestrel_runtime::replay`).

use std::ffi::c_int;

use kestrel_runtime::replay::{record_or_play, NativeId};

use crate::abi::{read, write, V2};
use crate::keys::{self, Binding};
use crate::raylib as rl;
use crate::state::{self, View};

/// Whether `key` is in the state raylib's `on_key` (or, for a mouse button, `on_mouse`) tests.
///
/// # Safety
/// `key` must point to a live `Key` value.
unsafe fn key_state(
    name: &'static str,
    key: *const u8,
    on_key: unsafe extern "C" fn(c_int) -> bool,
    on_mouse: unsafe extern "C" fn(c_int) -> bool,
) -> u8 {
    match state::view(name) {
        View::Fixed(_) => 0,
        View::Real { .. } => {
            // SAFETY (for the raylib calls below): the window is open.
            let tag = read(key);
            let hit = match keys::binding(tag) {
                Some(Binding::Keys(codes)) => codes.iter().any(|&code| unsafe { on_key(code) }),
                Some(Binding::Mouse(button)) => unsafe { on_mouse(button) },
                None => false,
            };
            hit as u8
        }
    }
}

/// # Safety
/// `key` must point to a live `Key` value.
#[no_mangle]
pub unsafe extern "C" fn kestrel_input_held(key: *const u8) -> u8 {
    u8::from(record_or_play(NativeId::InputHeld, || key_state("input.held", key, rl::IsKeyDown, rl::IsMouseButtonDown) != 0))
}

/// # Safety
/// `key` must point to a live `Key` value.
#[no_mangle]
pub unsafe extern "C" fn kestrel_input_pressed(key: *const u8) -> u8 {
    u8::from(record_or_play(NativeId::InputPressed, || key_state("input.pressed", key, rl::IsKeyPressed, rl::IsMouseButtonPressed) != 0))
}

/// # Safety
/// `key` must point to a live `Key` value.
#[no_mangle]
pub unsafe extern "C" fn kestrel_input_released(key: *const u8) -> u8 {
    u8::from(record_or_play(NativeId::InputReleased, || {
        key_state("input.released", key, rl::IsKeyReleased, rl::IsMouseButtonReleased) != 0
    }))
}

/// # Safety
/// `out` must be valid for writing a `vec2`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_input_mouse(out: *mut V2) {
    let [x, y] = record_or_play(NativeId::InputMouse, || match state::view("input.mouse") {
        View::Fixed(_) => [0.0, 0.0],
        View::Real { .. } => {
            // SAFETY: the window is open.
            let p = unsafe { rl::GetMousePosition() };
            [p.x, p.y]
        }
    });
    write(out, V2 { x, y });
}

#[no_mangle]
pub extern "C" fn kestrel_input_wheel() -> f32 {
    record_or_play(NativeId::InputWheel, || match state::view("input.wheel") {
        View::Fixed(_) => 0.0,
        // SAFETY: the window is open.
        View::Real { .. } => unsafe { rl::GetMouseWheelMove() },
    })
}

/// Locks and hides the mouse (`on` is not 0), or gives it back.
#[no_mangle]
pub extern "C" fn kestrel_input_capture(on: u8) {
    match state::view("input.capture") {
        View::Fixed(_) => {}
        View::Real { .. } => {
            // SAFETY: the window is open.
            unsafe {
                if on != 0 {
                    rl::DisableCursor();
                } else {
                    rl::EnableCursor();
                }
            }
        }
    }
}

/// # Safety
/// `out` must be valid for writing a `vec2`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_input_mouse_delta(out: *mut V2) {
    let [x, y] = record_or_play(NativeId::InputMouseDelta, || match state::view("input.mouse_delta") {
        View::Fixed(_) => [0.0, 0.0],
        View::Real { .. } => {
            // SAFETY: the window is open.
            let d = unsafe { rl::GetMouseDelta() };
            [d.x, d.y]
        }
    });
    write(out, V2 { x, y });
}
