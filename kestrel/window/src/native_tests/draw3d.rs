//! The `draw3d` natives, and the switch between 2D and 3D that every `draw` native takes part in,
//! called the way compiled code calls them, in headless mode.
//!
//! The pass state is per thread, so each test runs on a fresh thread (`headless_lines_of`).

use super::{headless, headless_lines_of, kstr, rgba, v2, NO_STRING};
use crate::abi::{KCamera, KRect, KSeq, KTexture, V3};
use crate::natives_3d::*;
use crate::natives_draw::*;
use crate::natives_texture::*;
use crate::natives_window::*;
use crate::pass;

fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

/// A camera three units behind the origin, looking at it.
fn camera() -> KCamera {
    KCamera { pos: v3(0.0, 0.0, -3.0), target: v3(0.0, 0.0, 0.0), up: v3(0.0, 1.0, 0.0), fov: 75.0 }
}

const CAMERA_LINE: &str =
    "draw3d.camera Camera(pos: vec3(x: 0, y: 0, z: -3), target: vec3(x: 0, y: 0, z: 0), up: vec3(x: 0, y: 1, z: 0), fov: 75)";

#[test]
fn headless_draw3d_lines() {
    let lines = headless_lines_of(|| {
        let title = kstr("t");
        let (red, blue, white) = (rgba(1.0, 0.0, 0.0, 1.0), rgba(0.0, 0.0, 1.0, 1.0), rgba(1.0, 1.0, 1.0, 0.5));
        // SAFETY: every pointer is to a live local, as compiled code passes them.
        unsafe {
            open_with(&headless(2), 64, 48, &title);
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw3d_camera(&camera());
            kestrel_draw3d_cube(&v3(0.0, 0.5, 0.0), &v3(1.0, 2.0, 3.0), &red);
            kestrel_draw3d_sphere(&v3(2.0, 0.0, 0.0), 0.5, &blue);
            kestrel_draw3d_line(&v3(-1.0, -1.0, 0.0), &v3(1.0, 1.0, -0.25), &white);
            kestrel_draw_text(&title, &v2(2.0, 2.0), 10.0, &white);

            // The next frame starts in 2D with the same camera, so a cube needs no camera call.
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw3d_cube(&v3(0.0, 0.0, 0.0), &v3(1.0, 1.0, 1.0), &red);
            // A sphere with no size, and a cube with negative sizes, still print what they were given.
            kestrel_draw3d_sphere(&v3(0.0, 0.0, 0.0), 0.0, &blue);
            kestrel_draw3d_cube(&v3(0.0, 0.0, 0.0), &v3(-1.0, 1.0, -2.0), &blue);
        }
    });
    assert_eq!(
        lines,
        [
            "window.open 64 48 \"t\"",
            CAMERA_LINE,
            "draw3d.cube vec3(x: 0, y: 0.5, z: 0) vec3(x: 1, y: 2, z: 3) Color(r: 1, g: 0, b: 0, a: 1)",
            "draw3d.sphere vec3(x: 2, y: 0, z: 0) 0.5 Color(r: 0, g: 0, b: 1, a: 1)",
            "draw3d.line vec3(x: -1, y: -1, z: 0) vec3(x: 1, y: 1, z: -0.25) Color(r: 1, g: 1, b: 1, a: 0.5)",
            "draw.text \"t\" vec2(x: 2, y: 2) 10 Color(r: 1, g: 1, b: 1, a: 0.5)",
            "draw3d.cube vec3(x: 0, y: 0, z: 0) vec3(x: 1, y: 1, z: 1) Color(r: 1, g: 0, b: 0, a: 1)",
            "draw3d.sphere vec3(x: 0, y: 0, z: 0) 0 Color(r: 0, g: 0, b: 1, a: 1)",
            "draw3d.cube vec3(x: 0, y: 0, z: 0) vec3(x: -1, y: 1, z: -2) Color(r: 0, g: 0, b: 1, a: 1)",
        ]
    );
}

/// An array header over `colors`, as compiled code passes a `[Color]`.
fn seq<T>(items: &[T]) -> KSeq {
    KSeq { ptr: items.as_ptr().cast(), len: items.len() as i64 }
}

/// Ruling 6: every drawing `draw` native ends a 3D pass, the texture ones and `rects` included.
#[test]
fn every_draw_native_goes_back_to_2d() {
    headless_lines_of(|| {
        let title = kstr("t");
        let white = rgba(1.0, 1.0, 1.0, 1.0);
        let mut tex = KTexture { id: 0, width: 0, height: 0 };
        let part = KRect { pos: v2(0.0, 0.0), size: v2(1.0, 1.0) };
        // SAFETY: every pointer is to a live local, as compiled code passes them.
        unsafe {
            open_with(&headless(1), 64, 64, &title);
            kestrel_draw_make_texture(&mut tex, 1, 1, &seq(&[white]));
            assert_eq!(kestrel_window_frame(), 1);
            let natives: [(&str, &dyn Fn()); 12] = [
                ("draw.clear", &|| kestrel_draw_clear(&white)),
                ("draw.rect", &|| kestrel_draw_rect(&v2(0.0, 0.0), &v2(2.0, 2.0), &white)),
                ("draw.rect_outline", &|| kestrel_draw_rect_outline(&v2(0.0, 0.0), &v2(2.0, 2.0), 1.0, &white)),
                ("draw.rects", &|| kestrel_draw_rects(&seq(&[v2(1.0, 1.0)]), &v2(2.0, 2.0), &white)),
                ("draw.circle", &|| kestrel_draw_circle(&v2(5.0, 5.0), 2.0, &white)),
                ("draw.circle_outline", &|| kestrel_draw_circle_outline(&v2(5.0, 5.0), 2.0, 1.0, &white)),
                ("draw.line", &|| kestrel_draw_line(&v2(0.0, 0.0), &v2(5.0, 5.0), 1.0, &white)),
                ("draw.triangle", &|| kestrel_draw_triangle(&v2(0.0, 0.0), &v2(5.0, 0.0), &v2(0.0, 5.0), &white)),
                ("draw.text", &|| kestrel_draw_text(&title, &v2(0.0, 0.0), 10.0, &white)),
                ("draw.texture", &|| kestrel_draw_texture(&tex, &v2(0.0, 0.0), &white)),
                ("draw.texture_rect", &|| kestrel_draw_texture_rect(&tex, &v2(0.0, 0.0), &v2(4.0, 4.0), &white)),
                ("draw.texture_region", &|| kestrel_draw_texture_region(&tex, &part, &v2(0.0, 0.0), &v2(4.0, 4.0), &white)),
            ];
            for (name, call) in natives {
                kestrel_draw3d_camera(&camera());
                assert!(pass::in_3d(), "{name}: the camera call starts 3D");
                call();
                assert!(!pass::in_3d(), "{name} ends the 3D pass");
                // And the next 3D native starts it again with the same camera.
                kestrel_draw3d_cube(&v3(0.0, 0.0, 0.0), &v3(1.0, 1.0, 1.0), &white);
                assert!(pass::in_3d(), "{name}: a 3D native after it is in 3D");
            }
        }
    });
}

#[test]
fn text_width_is_a_query_that_leaves_a_3d_pass_alone() {
    headless_lines_of(|| {
        let title = kstr("t");
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&headless(1), 64, 64, &title);
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw3d_camera(&camera());
            assert_eq!(kestrel_draw_text_width(&title, 10.0), 5.0);
            assert_eq!(kestrel_draw_text_width(&NO_STRING, 10.0), 0.0);
            assert!(pass::in_3d());
        }
    });
}

#[test]
fn a_screenshot_ends_a_3d_pass_so_its_drawing_is_in_the_image() {
    headless_lines_of(|| {
        let (title, shot) = (kstr("t"), kstr("shot.png"));
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&headless(1), 64, 64, &title);
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw3d_camera(&camera());
            assert!(pass::in_3d());
            kestrel_window_screenshot(&shot);
            assert!(!pass::in_3d());
            // A 3D native afterwards begins 3D again, with the camera it had.
            kestrel_draw3d_line(&v3(0.0, 0.0, 0.0), &v3(1.0, 0.0, 0.0), &rgba(1.0, 1.0, 1.0, 1.0));
            assert!(pass::in_3d());
        }
    });
}

#[test]
fn each_frame_starts_in_2d_and_keeps_the_camera() {
    let lines = headless_lines_of(|| {
        let title = kstr("t");
        let green = rgba(0.0, 1.0, 0.0, 1.0);
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&headless(3), 64, 64, &title);
            assert!(!pass::in_3d(), "a new window draws in 2D");
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw3d_camera(&camera());
            assert!(pass::in_3d());
            assert_eq!(kestrel_window_frame(), 1);
            assert!(!pass::in_3d(), "the next frame is in 2D");
            kestrel_draw3d_line(&v3(0.0, 0.0, 0.0), &v3(0.0, 1.0, 0.0), &green);
            assert!(pass::in_3d(), "and a 3D native needs no new camera");
            assert_eq!(kestrel_window_frame(), 1);
            assert!(!pass::in_3d());
        }
    });
    assert_eq!(lines.len(), 3, "{lines:?}");
}
