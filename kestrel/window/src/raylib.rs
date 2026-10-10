//! Hand-written declarations for the raylib 6.0 functions Kestrel calls.
//!
//! There is no bindgen on this machine, so each type here is copied from
//! `window/raylib/raylib.h` with its exact C layout (structs travel by value).
//! Add a declaration when a native needs one.
#![allow(non_snake_case)]

use std::ffi::{c_char, c_int, c_void};

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

extern "C" {
    pub fn GenImageColor(width: c_int, height: c_int, color: Color) -> Image;
    pub fn ImageDrawPixel(dst: *mut Image, pos_x: c_int, pos_y: c_int, color: Color);
    pub fn ExportImage(image: Image, file_name: *const c_char) -> bool;
    pub fn LoadImage(file_name: *const c_char) -> Image;
    pub fn GetImageColor(image: Image, x: c_int, y: c_int) -> Color;
    pub fn UnloadImage(image: Image);
}
