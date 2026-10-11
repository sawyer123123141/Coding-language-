//! `draw.rects` and negative-size rectangles, called the way compiled code calls them, in headless mode.

use super::{headless, headless_lines_of, kstr, rgba, v2};
use crate::abi::{KSeq, V2};
use crate::natives_draw::*;
use crate::natives_window::*;

/// An array header over `points`, as compiled code passes a `[vec2]` or a `list[vec2]`.
fn seq(points: &[V2]) -> KSeq {
    KSeq { ptr: points.as_ptr().cast(), len: points.len() as i64 }
}

#[test]
fn headless_rects_lines_print_the_positions_as_their_count() {
    let lines = headless_lines_of(|| {
        let title = kstr("t");
        let (red, blue) = (rgba(1.0, 0.0, 0.0, 1.0), rgba(0.0, 0.0, 1.0, 1.0));
        let dots = [v2(1.0, 1.0), v2(5.0, 1.0), v2(9.0, 1.0)];
        // SAFETY: every pointer is to a live local, as compiled code passes them.
        unsafe {
            open_with(&headless(1), 64, 64, &title);
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw_rects(&seq(&dots), &v2(2.0, 2.0), &red);
            kestrel_draw_rects(&seq(&dots[..2]), &v2(-4.0, 3.0), &blue);
            // An empty list, and a zero-filled header (a null element pointer).
            kestrel_draw_rects(&seq(&[]), &v2(2.0, 2.0), &red);
            kestrel_draw_rects(&KSeq { ptr: std::ptr::null(), len: 0 }, &v2(2.0, 2.0), &red);
        }
    });
    assert_eq!(
        lines,
        [
            "window.open 64 64 \"t\"",
            "draw.rects 3 vec2(x: 2, y: 2) Color(r: 1, g: 0, b: 0, a: 1)",
            "draw.rects 2 vec2(x: -4, y: 3) Color(r: 0, g: 0, b: 1, a: 1)",
            "draw.rects 0 vec2(x: 2, y: 2) Color(r: 1, g: 0, b: 0, a: 1)",
            "draw.rects 0 vec2(x: 2, y: 2) Color(r: 1, g: 0, b: 0, a: 1)",
        ]
    );
}

#[test]
fn a_negative_size_prints_as_the_call_gave_it() {
    let lines = headless_lines_of(|| {
        let title = kstr("t");
        let yellow = rgba(1.0, 1.0, 0.0, 1.0);
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&headless(1), 64, 64, &title);
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw_rect(&v2(30.0, 30.0), &v2(-10.0, -10.0), &yellow);
            kestrel_draw_rect_outline(&v2(14.0, 14.0), &v2(-12.0, 6.0), 2.0, &yellow);
        }
    });
    assert_eq!(
        lines,
        [
            "window.open 64 64 \"t\"",
            "draw.rect vec2(x: 30, y: 30) vec2(x: -10, y: -10) Color(r: 1, g: 1, b: 0, a: 1)",
            "draw.rect_outline vec2(x: 14, y: 14) vec2(x: -12, y: 6) 2 Color(r: 1, g: 1, b: 0, a: 1)",
        ]
    );
}
