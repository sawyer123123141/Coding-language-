//! The switch between 2D and 3D drawing.
//!
//! raylib draws in 2D until `BeginMode3D(camera)` and in 3D until `EndMode3D()`. A Kestrel program
//! never says which it wants. `draw3d.camera` starts 3D, the next `draw.*` call ends it, and a
//! later `draw3d.*` call starts it again with the same camera. This module keeps track, so each
//! native only says which kind of drawing it is about to do (`enter_2d`, `enter_3d`).
//!
//! What a pass does to raylib:
//!
//! - **`BeginMode3D`** first flushes the 2D batch, so what was drawn before stays underneath. It
//!   pushes the projection matrix (the 2D one) and sets a perspective projection with raylib's
//!   near and far planes (0.05 and 4,000) from the camera's vertical field of view and the window's
//!   aspect ratio, sets the modelview matrix to the camera's view, and turns the depth test on.
//! - **`EndMode3D`** flushes the 3D batch, pops the projection matrix, resets the modelview matrix
//!   and turns the depth test off. 2D drawing does not test depth, so it covers whatever is there.
//! - Neither touches backface culling. It stays as raylib has it: on, with back faces culled
//!   (`bulk.rs` turns it off around a draw and back on). raylib's cubes and spheres are wound for
//!   that, and the mesh natives that need the other side must put it back.
//! - Neither clears anything. `draw.clear` clears depth as well as colour.
//!
//! The state is the window's thread's, like the window state. A headless window runs the same state
//! machine and only skips the raylib calls, so a program that forgot `draw3d.camera` is told so
//! there too. The camera stays across frames; the pass itself does not: a frame starts in 2D, and
//! a pass that is still going when a frame ends is ended first (`state.rs`).

use std::cell::Cell;

use crate::abi::{float_text, KCamera, V3};
use crate::raylib as rl;

/// Where drawing is: the camera `draw3d.camera` last set, and whether the pass is in 3D.
#[derive(Clone, Copy)]
struct Pass {
    camera: Option<KCamera>,
    /// Whether raylib is between `BeginMode3D` and `EndMode3D` (or would be, headless).
    in_3d: bool,
}

thread_local! {
    static PASS: Cell<Pass> = const { Cell::new(Pass { camera: None, in_3d: false }) };
}

/// Whether the pass is in 3D (for tests of the natives, which can't see the private state).
#[cfg(test)]
pub(crate) fn in_3d() -> bool {
    PASS.get().in_3d
}

/// Before any `draw.*` drawing: ends a 3D pass if one is going (EndMode3D when real).
pub fn enter_2d(real: bool) {
    let pass = PASS.get();
    if pass.in_3d {
        PASS.set(Pass { in_3d: false, ..pass });
        if real {
            // SAFETY: `real` means a real window is open, on this thread, and the pass was begun in it.
            unsafe { rl::EndMode3D() }
        }
    }
}

/// Before a `draw3d.*` drawing native: begins 3D with the camera if needed, or says why it can't:
/// "{name} needs a camera; call draw3d.camera first".
pub fn enter_3d(name: &str, real: bool) -> Result<(), String> {
    let pass = PASS.get();
    if pass.in_3d {
        return Ok(());
    }
    let camera = pass.camera.ok_or_else(|| format!("{name} needs a camera; call draw3d.camera first"))?;
    begin(camera, real);
    Ok(())
}

/// `draw3d.camera`: checks `cam` (see `check_camera`), ends any 3D pass, keeps it, and begins 3D.
/// A camera that fails the checks changes nothing.
pub fn set_camera(cam: KCamera, real: bool) -> Result<(), String> {
    check_camera(&cam)?;
    enter_2d(real);
    begin(cam, real);
    Ok(())
}

/// At the start of each frame: drawing starts in 2D; the camera stays. (A real window's pass was
/// ended before the last frame was: this is what resets a headless one, which has no raylib to end.)
pub fn start_frame() {
    PASS.set(Pass { in_3d: false, ..PASS.get() });
}

/// Begins a pass with `camera`, which is also the camera from now on.
fn begin(camera: KCamera, real: bool) {
    PASS.set(Pass { camera: Some(camera), in_3d: true });
    if real {
        // SAFETY: `real` means a real window is open, on this thread. No pass was going (the callers
        // end one first), so the matrix stack `BeginMode3D` pushes on is balanced by an `EndMode3D`.
        unsafe { rl::BeginMode3D(camera.to_raylib()) }
    }
}

/// The checks `draw3d.camera` makes, as the messages it fails with.
///
/// - `pos` and `target` must differ. This is an exact comparison, not a tolerance: any two
///   different points make a direction, however short. (A coordinate that is not a number compares
///   unequal, so it passes here and the camera draws nothing; the spec has no message for it.)
/// - `up` must not point along the view. The sine of the angle between `target - pos` and `up` has
///   to be above `1e-6`, i.e. the length of their cross product above `1e-6` times the product of
///   their lengths. A zero `up` has a cross product and a length of 0 and is refused too, so the
///   comparison is "at or below".
/// - `fov` must be above 0 and below 180 degrees. A fov that is not a number is neither.
pub fn check_camera(cam: &KCamera) -> Result<(), String> {
    if cam.pos == cam.target {
        return Err("draw3d.camera needs pos and target to differ".to_string());
    }
    let view = difference(cam.target, cam.pos);
    let up = [f64::from(cam.up.x), f64::from(cam.up.y), f64::from(cam.up.z)];
    if length(cross(view, up)) <= 1e-6 * length(view) * length(up) {
        return Err("draw3d.camera needs an up that doesn't point along the view".to_string());
    }
    if !(cam.fov > 0.0 && cam.fov < 180.0) {
        return Err(format!("draw3d.camera needs a fov between 0 and 180 degrees, not {}", float_text(cam.fov)));
    }
    Ok(())
}

/// `a - b`, in double precision so the checks neither overflow nor lose the small differences.
fn difference(a: V3, b: V3) -> [f64; 3] {
    [f64::from(a.x) - f64::from(b.x), f64::from(a.y) - f64::from(b.y), f64::from(a.z) - f64::from(b.z)]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::V3;

    /// Runs `body` on a fresh thread: the pass state is per thread, and tests share a process.
    fn on_a_thread<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::spawn(body).join().unwrap()
    }

    fn v3(x: f32, y: f32, z: f32) -> V3 {
        V3 { x, y, z }
    }

    /// A camera three units behind the origin, looking at it, y up, 75 degrees.
    fn good() -> KCamera {
        KCamera { pos: v3(0.0, 0.0, -3.0), target: v3(0.0, 0.0, 0.0), up: v3(0.0, 1.0, 0.0), fov: 75.0 }
    }

    fn message(cam: KCamera) -> String {
        check_camera(&cam).unwrap_err()
    }

    #[test]
    fn a_good_camera_passes() {
        assert_eq!(check_camera(&good()), Ok(()));
        // Any up that isn't along the view will do, including a slanted or a backwards one.
        assert_eq!(check_camera(&KCamera { up: v3(0.0, -1.0, 0.0), ..good() }), Ok(()));
        assert_eq!(check_camera(&KCamera { up: v3(1.0, 1.0, 1.0), ..good() }), Ok(()));
        assert_eq!(check_camera(&KCamera { up: v3(0.0, 1e-3, 1.0), ..good() }), Ok(()), "a little off the view is enough");
        // Any fov strictly between 0 and 180, however small or wide.
        for fov in [0.001, 1.0, 75.0, 120.0, 179.9] {
            assert_eq!(check_camera(&KCamera { fov, ..good() }), Ok(()), "fov {fov}");
        }
        // A pos and target only a little apart are different.
        assert_eq!(check_camera(&KCamera { target: v3(0.0, 0.0, -2.999), ..good() }), Ok(()));
    }

    #[test]
    fn pos_and_target_must_differ() {
        let same = "draw3d.camera needs pos and target to differ";
        assert_eq!(message(KCamera { target: good().pos, ..good() }), same);
        let at = v3(1.0, 2.0, 3.0);
        assert_eq!(message(KCamera { pos: at, target: at, ..good() }), same);
        // Differing in a single axis is enough, and -0 equals 0.
        assert_eq!(check_camera(&KCamera { pos: v3(1.0, 2.0, 3.0), target: v3(1.0, 2.0, 3.5), ..good() }), Ok(()));
        assert_eq!(message(KCamera { pos: v3(0.0, 0.0, 0.0), target: v3(-0.0, -0.0, -0.0), ..good() }), same);
        // It is checked before the up and the fov.
        assert_eq!(message(KCamera { target: good().pos, up: v3(0.0, 0.0, 0.0), fov: 0.0, ..good() }), same);
    }

    #[test]
    fn up_must_not_point_along_the_view() {
        let along = "draw3d.camera needs an up that doesn't point along the view";
        // The view is +z. Up along it, against it, scaled, and zero (which points nowhere) are all refused.
        for up in [v3(0.0, 0.0, 1.0), v3(0.0, 0.0, -1.0), v3(0.0, 0.0, 250.0), v3(0.0, 0.0, 0.0)] {
            assert_eq!(message(KCamera { up, ..good() }), along, "up {up:?}");
        }
        // The rule is relative: a short view or a short up is judged by its direction alone.
        let near = KCamera { pos: v3(0.0, 0.0, 0.0), target: v3(0.0, 0.0, 1e-3), ..good() };
        assert_eq!(message(KCamera { up: v3(0.0, 0.0, 1.0), ..near }), along);
        assert_eq!(check_camera(&KCamera { up: v3(0.0, 1.0, 0.0), ..near }), Ok(()));
        assert_eq!(message(KCamera { up: v3(0.0, 0.0, 1e-9), ..good() }), along);
        assert_eq!(check_camera(&KCamera { up: v3(1e-9, 0.0, 1e-9), ..good() }), Ok(()), "a tiny up at 45 degrees is fine");
        // Just inside the 1e-6 limit is refused, just outside is not.
        assert_eq!(message(KCamera { up: v3(0.0, 5e-7, 1.0), ..good() }), along);
        assert_eq!(check_camera(&KCamera { up: v3(0.0, 2e-6, 1.0), ..good() }), Ok(()));
        // The up is checked before the fov.
        assert_eq!(message(KCamera { up: v3(0.0, 0.0, 1.0), fov: 0.0, ..good() }), along);
    }

    #[test]
    fn fov_must_be_between_0_and_180_degrees() {
        let wrong = |fov: f32| message(KCamera { fov, ..good() });
        let text = |shown: &str| format!("draw3d.camera needs a fov between 0 and 180 degrees, not {shown}");
        assert_eq!(wrong(200.0), text("200"));
        assert_eq!(wrong(0.0), text("0"));
        assert_eq!(wrong(180.0), text("180"));
        assert_eq!(wrong(-5.0), text("-5"));
        assert_eq!(wrong(180.5), text("180.5"));
        assert_eq!(wrong(f32::NAN), text("NaN"));
        assert_eq!(wrong(f32::INFINITY), text("inf"));
        assert_eq!(wrong(f32::NEG_INFINITY), text("-inf"));
    }

    /// Whether the pass is in 3D.
    fn going() -> bool {
        PASS.get().in_3d
    }

    #[test]
    fn a_drawing_in_3d_needs_a_camera_first() {
        on_a_thread(|| {
            let wrong = enter_3d("draw3d.cube", false).unwrap_err();
            assert_eq!(wrong, "draw3d.cube needs a camera; call draw3d.camera first");
            // The name is the native that asked, and asking again changes nothing.
            assert_eq!(enter_3d("draw3d.mesh", false), Err("draw3d.mesh needs a camera; call draw3d.camera first".to_string()));
            // A 2D call first doesn't give a camera.
            enter_2d(false);
            assert!(enter_3d("draw3d.line", false).is_err());
            assert!(!going());
        });
    }

    #[test]
    fn a_camera_with_a_problem_is_refused_and_changes_nothing() {
        on_a_thread(|| {
            let wrong = set_camera(KCamera { target: good().pos, ..good() }, false).unwrap_err();
            assert_eq!(wrong, "draw3d.camera needs pos and target to differ");
            assert!(!going() && enter_3d("draw3d.cube", false).is_err(), "a refused camera was not kept, and 3D didn't start");
            // After a good one, a bad one leaves the good one set, and 3D going.
            set_camera(good(), false).unwrap();
            assert!(set_camera(KCamera { fov: 200.0, ..good() }, false).is_err());
            assert!(going());
            assert_eq!(PASS.get().camera, Some(good()));
        });
    }

    #[test]
    fn the_pass_goes_2d_to_3d_and_back() {
        on_a_thread(|| {
            assert!(!going(), "drawing starts in 2D");
            // The camera call starts 3D; the 3D natives that follow find it going.
            assert_eq!(set_camera(good(), false), Ok(()));
            assert!(going());
            assert_eq!(enter_3d("draw3d.cube", false), Ok(()));
            assert!(going());
            // A draw call goes back to 2D; the next 3D call begins 3D again, with no new camera.
            enter_2d(false);
            assert!(!going());
            assert_eq!(enter_3d("draw3d.sphere", false), Ok(()));
            assert!(going());
            // Ending 2D twice, or in 2D already, does nothing more.
            enter_2d(false);
            enter_2d(false);
            assert!(!going());
            assert_eq!(enter_3d("draw3d.line", false), Ok(()));
            assert!(going());
        });
    }

    #[test]
    fn the_camera_stays_across_frames_but_drawing_starts_in_2d() {
        on_a_thread(|| {
            set_camera(good(), false).unwrap();
            assert!(going());
            start_frame();
            assert!(!going(), "the new frame starts in 2D");
            // The camera it had is still set: a 3D call needs no new camera call.
            assert_eq!(enter_3d("draw3d.cube", false), Ok(()));
            assert!(going());
            start_frame();
            assert!(!going());
            assert_eq!(enter_3d("draw3d.cube", false), Ok(()));
            // A frame start in 2D already changes nothing.
            enter_2d(false);
            start_frame();
            assert!(!going());
        });
    }

    #[test]
    fn a_new_camera_replaces_the_old_one_and_starts_3d() {
        on_a_thread(|| {
            set_camera(good(), false).unwrap();
            let moved = KCamera { pos: v3(0.0, 0.0, -9.0), ..good() };
            // Set while 3D is going: the old pass ends and a new one starts with the new camera.
            set_camera(moved, false).unwrap();
            assert!(going());
            assert_eq!(PASS.get().camera, Some(moved));
            enter_2d(false);
            assert_eq!(enter_3d("draw3d.cube", false), Ok(()));
            assert_eq!(PASS.get().camera, Some(moved), "the later 3D call uses the newest camera");
        });
    }
}
