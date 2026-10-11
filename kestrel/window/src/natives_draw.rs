//! The `draw` module's natives (`driver/std/draw.kes`).
//!
//! Each one is `kestrel_draw_<name>`. A `vec2`, `Color` or `string` argument is a
//! pointer to the value, read once before anything else. Every native checks
//! that the window is open first, then either prints its line (headless) or
//! draws with raylib.

use crate::abi::{c_string, elems, read, str_bytes, KColor, KSeq, KStr, V2};
use crate::bulk::{self, span};
use crate::headless::{self, Arg};
use crate::raylib as rl;
use crate::state::{self, Mode};

/// Runs `real` to draw with raylib, or prints the call's line in headless mode.
fn draw(name: &'static str, args: &[Arg], real: impl FnOnce()) {
    match state::drawing(name) {
        Mode::Headless => headless::line(name, args),
        Mode::Real => real(),
    }
}

/// The spacing raylib's default font looks right with.
fn spacing(size: f32) -> f32 {
    size / 10.0
}

/// # Safety
/// `c` must point to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_clear(c: *const KColor) {
    let c = read(c);
    // SAFETY (for every `unsafe` block below): the window is open, on this thread.
    draw("draw.clear", &[Arg::Color(c)], || unsafe { rl::ClearBackground(c.to_raylib()) });
}

/// A negative width or height covers the rectangle the size spans back from `pos` (`bulk::span`).
/// The headless line shows the size as given.
///
/// # Safety
/// `pos` and `size` must point to live `vec2`s, `c` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_rect(pos: *const V2, size: *const V2, c: *const KColor) {
    let (pos, size, c) = (read(pos), read(size), read(c));
    let args = [Arg::Vec2(pos), Arg::Vec2(size), Arg::Color(c)];
    draw("draw.rect", &args, || {
        let (corner, size) = span(pos, size);
        unsafe { rl::DrawRectangleV(corner.to_raylib(), size.to_raylib(), c.to_raylib()) }
    });
}

/// The border is drawn inside the rectangle, `width` thick. A negative size spans back from
/// `pos`, as for `rect`.
///
/// # Safety
/// `pos` and `size` must point to live `vec2`s, `c` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_rect_outline(pos: *const V2, size: *const V2, width: f32, c: *const KColor) {
    let (pos, size, c) = (read(pos), read(size), read(c));
    let args = [Arg::Vec2(pos), Arg::Vec2(size), Arg::Float(width), Arg::Color(c)];
    draw("draw.rect_outline", &args, || {
        let (corner, size) = span(pos, size);
        let rec = rl::Rectangle { x: corner.x, y: corner.y, width: size.x, height: size.y };
        unsafe { rl::DrawRectangleLinesEx(rec, width, c.to_raylib()) }
    });
}

/// One rectangle of `size` at each of `positions`, in order, all in `c`: what a `rect` for each
/// would draw, in one GPU draw call (`bulk.rs`). The headless line shows the positions' count.
///
/// # Safety
/// `positions` must point to a live array or list header of `vec2`s, `size` to a live `vec2`,
/// `c` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_rects(positions: *const KSeq, size: *const V2, c: *const KColor) {
    let (positions, size, c) = (elems::<V2>(positions), read(size), read(c));
    let args = [Arg::Count(positions.len() as i64), Arg::Vec2(size), Arg::Color(c)];
    // SAFETY: as for every block here; and every drawing native draws in 2D.
    draw("draw.rects", &args, || unsafe { bulk::draw(positions, size, c.to_raylib()) });
}

/// # Safety
/// `center` must point to a live `vec2`, `c` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_circle(center: *const V2, radius: f32, c: *const KColor) {
    let (center, c) = (read(center), read(c));
    let args = [Arg::Vec2(center), Arg::Float(radius), Arg::Color(c)];
    draw("draw.circle", &args, || {
        if radius > 0.0 {
            // A full sector with `segments` 0: raylib picks enough segments for the radius.
            // (`DrawCircleV` is this with a fixed 36, which shows corners on a big circle.)
            unsafe { rl::DrawCircleSector(center.to_raylib(), radius, 0.0, 360.0, 0, c.to_raylib()) }
        }
    });
}

/// The ring is drawn inside `radius`, `width` thick, like `rect_outline`.
///
/// # Safety
/// `center` must point to a live `vec2`, `c` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_circle_outline(center: *const V2, radius: f32, width: f32, c: *const KColor) {
    let (center, c) = (read(center), read(c));
    let args = [Arg::Vec2(center), Arg::Float(radius), Arg::Float(width), Arg::Color(c)];
    draw("draw.circle_outline", &args, || {
        if radius > 0.0 && width > 0.0 {
            // A ring from `radius - width` to `radius`. When `width` reaches the centre raylib
            // fills the disc.
            let inner = (radius - width).max(0.0);
            unsafe { rl::DrawRing(center.to_raylib(), inner, radius, 0.0, 360.0, 0, c.to_raylib()) }
        }
    });
}

/// # Safety
/// `a` and `b` must point to live `vec2`s, `c` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_line(a: *const V2, b: *const V2, width: f32, c: *const KColor) {
    let (a, b, c) = (read(a), read(b), read(c));
    let args = [Arg::Vec2(a), Arg::Vec2(b), Arg::Float(width), Arg::Color(c)];
    draw("draw.line", &args, || unsafe { rl::DrawLineEx(a.to_raylib(), b.to_raylib(), width, c.to_raylib()) });
}

/// Whether raylib draws the triangle `a`, `b`, `c` in this order. It culls the
/// back face, so it shows only a triangle whose vertices run counter-clockwise
/// on the screen (y points down, which makes that a negative cross product).
fn faces_front(a: V2, b: V2, c: V2) -> bool {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x) < 0.0
}

/// Draws in either vertex order.
///
/// # Safety
/// `a`, `b` and `c3` must point to live `vec2`s, `c` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_triangle(a: *const V2, b: *const V2, c3: *const V2, c: *const KColor) {
    let (a, b, c3, c) = (read(a), read(b), read(c3), read(c));
    let args = [Arg::Vec2(a), Arg::Vec2(b), Arg::Vec2(c3), Arg::Color(c)];
    draw("draw.triangle", &args, || {
        // raylib wants one winding; hand it the vertices in that order.
        let (b, c3) = if faces_front(a, b, c3) { (b, c3) } else { (c3, b) };
        unsafe { rl::DrawTriangle(a.to_raylib(), b.to_raylib(), c3.to_raylib(), c.to_raylib()) }
    });
}

/// # Safety
/// `text` must point to a live string header, `pos` to a live `vec2`, `c` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_text(text: *const KStr, pos: *const V2, size: f32, c: *const KColor) {
    let (text, pos, c) = (str_bytes(text), read(pos), read(c));
    let args = [Arg::Str(text), Arg::Vec2(pos), Arg::Float(size), Arg::Color(c)];
    draw("draw.text", &args, || {
        if !text.is_empty() {
            let text = c_string(text);
            unsafe {
                let font = rl::GetFontDefault();
                rl::DrawTextEx(font, text.as_ptr(), pos.to_raylib(), size, spacing(size), c.to_raylib());
            }
        }
    });
}

/// How wide `draw.text` would draw `text` at `size`. An empty text is 0 wide.
///
/// # Safety
/// `text` must point to a live string header.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_text_width(text: *const KStr, size: f32) -> f32 {
    let text = str_bytes(text);
    match state::drawing("draw.text_width") {
        Mode::Headless => {
            headless::line("draw.text_width", &[Arg::Str(text), Arg::Float(size)]);
            headless::text_width(text, size)
        }
        Mode::Real if text.is_empty() => 0.0,
        Mode::Real => {
            let text = c_string(text);
            // SAFETY: the window is open, so the default font is loaded.
            unsafe { rl::MeasureTextEx(rl::GetFontDefault(), text.as_ptr(), size, spacing(size)).x }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_winding_of_a_triangle_faces_front() {
        let a = V2 { x: 0.0, y: 0.0 };
        let b = V2 { x: 10.0, y: 0.0 };
        let c = V2 { x: 0.0, y: 10.0 };
        // a -> b -> c goes right, then down-left: clockwise on a y-down screen, so raylib would cull it.
        // (Checked against a real window: the swapped order is the one that shows.)
        assert!(!faces_front(a, b, c));
        assert!(faces_front(a, c, b));
        // Rotating the vertices keeps the winding.
        assert!(faces_front(c, b, a));
        assert!(faces_front(b, a, c));
        // A degenerate triangle faces neither way, and draws nothing anyway.
        assert!(!faces_front(a, a, c));
    }
}
