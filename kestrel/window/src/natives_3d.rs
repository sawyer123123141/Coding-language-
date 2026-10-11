//! The `draw3d` module's natives (`driver/std/draw3d.kes`).
//!
//! Each one is `kestrel_draw3d_<name>`. A `vec3`, `Camera` or `Color` argument is a pointer to
//! its floats, read once before anything else. Every native checks that the window is open, then
//! (for a drawing) moves the 2D/3D pass to 3D (`pass.rs`), which says so if no camera has been set
//! yet, and then either prints its line (headless) or draws with raylib. The shapes are raylib's:
//! drawn in 3D with the depth test on, and with no lighting.

use kestrel_runtime::panic::fail_at_call;

use crate::abi::{read, KCamera, KColor, V3};
use crate::headless::{self, Arg};
use crate::pass;
use crate::raylib as rl;
use crate::state::{self, Mode};

/// Checks the window and starts 3D with the camera if the pass isn't in 3D, then runs `real` to
/// draw with raylib, or prints the call's line in headless mode.
fn draw(name: &'static str, args: &[Arg], real: impl FnOnce()) {
    let mode = state::drawing(name);
    pass::enter_3d(name, mode == Mode::Real).unwrap_or_else(|msg| fail_at_call(&msg));
    match mode {
        Mode::Headless => headless::line(name, args),
        Mode::Real => real(),
    }
}

/// The box a size describes. A size has no direction here (the box is centred), so a negative width,
/// height or depth is the same box as the positive one; raylib would turn the faces inside out and
/// draw nothing you could see.
fn extent(size: V3) -> V3 {
    V3 { x: size.x.abs(), y: size.y.abs(), z: size.z.abs() }
}

/// Keeps `cam` (a copy; changing the program's `Camera` afterwards needs another call) and starts
/// 3D drawing with it. The camera stays set, across frames, until the next call.
///
/// # Safety
/// `cam` must point to a live `Camera`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw3d_camera(cam: *const KCamera) {
    let cam = read(cam);
    let mode = state::drawing("draw3d.camera");
    pass::set_camera(cam, mode == Mode::Real).unwrap_or_else(|msg| fail_at_call(&msg));
    if mode == Mode::Headless {
        headless::line("draw3d.camera", &[Arg::Camera(cam)]);
    }
}

/// A solid box centred on `center`. A negative size is the same box as a positive one (`extent`).
///
/// # Safety
/// `center` and `size` must point to live `vec3`s, `c` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw3d_cube(center: *const V3, size: *const V3, c: *const KColor) {
    let (center, size, c) = (read(center), read(size), read(c));
    let args = [Arg::Vec3(center), Arg::Vec3(size), Arg::Color(c)];
    // SAFETY (for every `unsafe` block below): the window is open, on this thread, in a 3D pass.
    draw("draw3d.cube", &args, || unsafe {
        let size = extent(size);
        rl::DrawCube(center.to_raylib(), size.x, size.y, size.z, c.to_raylib())
    });
}

/// A solid ball. A radius of 0 or less draws nothing, like `draw.circle`.
///
/// # Safety
/// `center` must point to a live `vec3`, `c` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw3d_sphere(center: *const V3, radius: f32, c: *const KColor) {
    let (center, c) = (read(center), read(c));
    let args = [Arg::Vec3(center), Arg::Float(radius), Arg::Color(c)];
    draw("draw3d.sphere", &args, || {
        if radius > 0.0 {
            unsafe { rl::DrawSphere(center.to_raylib(), radius, c.to_raylib()) }
        }
    });
}

/// A line one pixel wide: GPUs draw thick 3D lines poorly, so there is no width.
///
/// # Safety
/// `a` and `b` must point to live `vec3`s, `c` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw3d_line(a: *const V3, b: *const V3, c: *const KColor) {
    let (a, b, c) = (read(a), read(b), read(c));
    let args = [Arg::Vec3(a), Arg::Vec3(b), Arg::Color(c)];
    draw("draw3d.line", &args, || unsafe { rl::DrawLine3D(a.to_raylib(), b.to_raylib(), c.to_raylib()) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_size_is_the_same_box() {
        assert_eq!(extent(V3 { x: -1.0, y: 2.0, z: -3.5 }), V3 { x: 1.0, y: 2.0, z: 3.5 });
        assert_eq!(extent(V3 { x: 0.0, y: -0.0, z: 4.0 }), V3 { x: 0.0, y: 0.0, z: 4.0 });
    }
}
