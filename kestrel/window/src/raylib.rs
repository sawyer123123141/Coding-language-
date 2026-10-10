//! Hand-written declarations for the raylib 6.0 functions Kestrel calls.
//!
//! There is no bindgen on this machine, so each type here is copied from
//! `window/raylib/raylib.h` with its exact C layout (structs travel by value).
//! Add a declaration when a native needs one.
#![allow(non_snake_case)]

use std::ffi::{c_char, c_int, c_uint, c_void};

/// `Color`: four bytes, passed in one register.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// `Vector2`: two floats.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

/// `Rectangle`: top-left corner and size.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rectangle {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// `Image`: pixels in CPU memory. Free it with `UnloadImage`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Image {
    pub data: *mut c_void,
    pub width: c_int,
    pub height: c_int,
    pub mipmaps: c_int,
    pub format: c_int,
}

/// `Texture` (also `Texture2D`): an image in GPU memory.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Texture {
    pub id: c_uint,
    pub width: c_int,
    pub height: c_int,
    pub mipmaps: c_int,
    pub format: c_int,
}

/// `Font`: a glyph atlas. It is only passed back to raylib, so the two
/// pointers (`Rectangle *recs`, `GlyphInfo *glyphs`) are never read here.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Font {
    pub base_size: c_int,
    pub glyph_count: c_int,
    pub glyph_padding: c_int,
    pub texture: Texture,
    pub recs: *mut Rectangle,
    pub glyphs: *mut c_void,
}

// `ConfigFlags` (the `FLAG_*` values).
pub const FLAG_WINDOW_RESIZABLE: c_uint = 0x0000_0004;
pub const FLAG_MSAA_4X_HINT: c_uint = 0x0000_0020;
pub const FLAG_VSYNC_HINT: c_uint = 0x0000_0040;
pub const FLAG_WINDOW_HIDDEN: c_uint = 0x0000_0080;

/// `TraceLogLevel::LOG_ERROR`.
pub const LOG_ERROR: c_int = 5;

/// `KeyboardKey::KEY_NULL`: no key.
pub const KEY_NULL: c_int = 0;

extern "C" {
    // Window and frame loop (rcore.c).
    pub fn SetConfigFlags(flags: c_uint);
    pub fn SetTraceLogLevel(log_level: c_int);
    pub fn InitWindow(width: c_int, height: c_int, title: *const c_char);
    pub fn IsWindowReady() -> bool;
    pub fn SetExitKey(key: c_int);
    pub fn WindowShouldClose() -> bool;
    pub fn CloseWindow();
    pub fn BeginDrawing();
    pub fn EndDrawing();
    pub fn ClearBackground(color: Color);
    pub fn GetFrameTime() -> f32;
    pub fn GetTime() -> f64;
    pub fn GetScreenWidth() -> c_int;
    pub fn GetScreenHeight() -> c_int;
    pub fn GetFPS() -> c_int;

    // Input (rcore.c).
    pub fn IsKeyPressed(key: c_int) -> bool;
    pub fn IsKeyDown(key: c_int) -> bool;
    pub fn IsKeyReleased(key: c_int) -> bool;
    pub fn IsMouseButtonPressed(button: c_int) -> bool;
    pub fn IsMouseButtonDown(button: c_int) -> bool;
    pub fn IsMouseButtonReleased(button: c_int) -> bool;
    pub fn GetMousePosition() -> Vector2;
    pub fn GetMouseWheelMove() -> f32;

    // Shapes (rshapes.c).
    pub fn DrawRectangleV(position: Vector2, size: Vector2, color: Color);
    pub fn DrawRectangleLinesEx(rec: Rectangle, line_thick: f32, color: Color);
    pub fn DrawCircleSector(center: Vector2, radius: f32, start_angle: f32, end_angle: f32, segments: c_int, color: Color);
    pub fn DrawRing(
        center: Vector2,
        inner_radius: f32,
        outer_radius: f32,
        start_angle: f32,
        end_angle: f32,
        segments: c_int,
        color: Color,
    );
    pub fn DrawLineEx(start_pos: Vector2, end_pos: Vector2, thick: f32, color: Color);
    pub fn DrawTriangle(v1: Vector2, v2: Vector2, v3: Vector2, color: Color);

    // Text (rtext.c).
    pub fn GetFontDefault() -> Font;
    pub fn DrawTextEx(font: Font, text: *const c_char, position: Vector2, font_size: f32, spacing: f32, tint: Color);
    pub fn MeasureTextEx(font: Font, text: *const c_char, font_size: f32, spacing: f32) -> Vector2;

    // Images (rtextures.c).
    pub fn GenImageColor(width: c_int, height: c_int, color: Color) -> Image;
    pub fn ImageDrawPixel(dst: *mut Image, pos_x: c_int, pos_y: c_int, color: Color);
    pub fn ExportImage(image: Image, file_name: *const c_char) -> bool;
    pub fn LoadImage(file_name: *const c_char) -> Image;
    pub fn LoadImageFromScreen() -> Image;
    pub fn GetImageColor(image: Image, x: c_int, y: c_int) -> Color;
    pub fn UnloadImage(image: Image);

    // rlgl.h: flush the batch of shapes drawn so far.
    pub fn rlDrawRenderBatchActive();
}
