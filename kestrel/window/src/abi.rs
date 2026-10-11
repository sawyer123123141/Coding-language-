//! How compiled Kestrel code hands values to a native, and how a native reads them.
//!
//! A native follows Kestrel's own calling convention: scalars travel in
//! registers, and everything bigger travels by pointer. A `string` is a pointer
//! to a 16-byte header, a `vec2` or `Color` a pointer to its floats, an enum a
//! pointer to its tag byte. An aggregate result is written through a pointer
//! that comes first. The pointers may aim at the caller's own variables, so a
//! native only ever reads through an argument.
//!
//! The compiled code's stack slots are not promised to be aligned for their
//! type, so every access here is unaligned.

use crate::raylib;

/// A `string`: bytes and a length. A zero-filled or empty string has a null
/// `ptr` and a `len` of 0.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KStr {
    pub ptr: *const u8,
    pub len: i64,
}

/// A `vec2`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V2 {
    pub x: f32,
    pub y: f32,
}

/// A `vec3`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// `draw3d.Camera`: `pos`, `target` and `up` (three `vec3`s) then `fov` in degrees, 40 bytes with
/// the alignment of a float.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KCamera {
    pub pos: V3,
    pub target: V3,
    pub up: V3,
    pub fov: f32,
}

/// `draw.Color`: four channels from 0 to 1.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// `draw.Rect`: a top-left corner and a size.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KRect {
    pub pos: V2,
    pub size: V2,
}

/// `draw.Texture`: the id of its GPU object (0 for none) and its size at the time it was made.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KTexture {
    pub id: i64,
    pub width: i32,
    pub height: i32,
}

/// The first 16 bytes of an array or list header, which the two share: where the elements
/// are, and how many.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KSeq {
    pub ptr: *const u8,
    pub len: i64,
}

impl V2 {
    pub fn to_raylib(self) -> raylib::Vector2 {
        raylib::Vector2 { x: self.x, y: self.y }
    }
}

impl V3 {
    pub fn to_raylib(self) -> raylib::Vector3 {
        raylib::Vector3 { x: self.x, y: self.y, z: self.z }
    }
}

impl KCamera {
    /// A perspective camera with raylib's near and far planes (0.05 and 4,000), and `fov` as the
    /// vertical field of view, which is what raylib's `fovy` is.
    pub fn to_raylib(self) -> raylib::Camera3D {
        raylib::Camera3D {
            position: self.pos.to_raylib(),
            target: self.target.to_raylib(),
            up: self.up.to_raylib(),
            fovy: self.fov,
            projection: raylib::CAMERA_PERSPECTIVE,
        }
    }
}

impl KColor {
    /// The 8-bit colour raylib draws with. Each channel is clamped to 0..=1.
    pub fn to_raylib(self) -> raylib::Color {
        raylib::Color { r: channel(self.r), g: channel(self.g), b: channel(self.b), a: channel(self.a) }
    }
}

/// One channel, 0..=1 to 0..=255, rounded. Values outside 0..=1 are clamped.
pub fn channel(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Reads the value `p` points at.
///
/// # Safety
/// `p` must point to a live `T`.
pub unsafe fn read<T: Copy>(p: *const T) -> T {
    p.read_unaligned()
}

/// Writes `v` where `out` points: the result pointer of an aggregate return.
///
/// # Safety
/// `out` must be valid for writing a `T`.
pub unsafe fn write<T>(out: *mut T, v: T) {
    out.write_unaligned(v)
}

/// The bytes of the string `s` points at. A null `s`, a null `ptr` or a
/// non-positive `len` gives an empty slice; `from_raw_parts` never sees null.
///
/// # Safety
/// `s` must be null or point to a string header whose `ptr` addresses `len`
/// readable bytes that outlive the returned slice.
pub unsafe fn str_bytes<'a>(s: *const KStr) -> &'a [u8] {
    if s.is_null() {
        return &[];
    }
    let s = read(s);
    if s.ptr.is_null() || s.len <= 0 {
        return &[];
    }
    std::slice::from_raw_parts(s.ptr, s.len as usize)
}

/// The elements of the array or list `s` points at. A null `s`, a null `ptr` or a non-positive
/// `len` gives an empty slice; `from_raw_parts` never sees null.
///
/// # Safety
/// `s` must be null or point to an array or list header whose `ptr` addresses `len` live `T`s
/// that outlive the returned slice. Kestrel allocates the elements aligned for their type, so
/// a slice of them is aligned for `T` as long as `T` has the layout of the element type.
pub unsafe fn elems<'a, T>(s: *const KSeq) -> &'a [T] {
    if s.is_null() {
        return &[];
    }
    let s = read(s);
    if s.ptr.is_null() || s.len <= 0 {
        return &[];
    }
    std::slice::from_raw_parts(s.ptr.cast::<T>(), s.len as usize)
}

/// A NUL-terminated copy of `bytes` for raylib. C strings end at the first NUL,
/// so the copy does too.
pub fn c_string(bytes: &[u8]) -> std::ffi::CString {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    std::ffi::CString::new(&bytes[..end]).expect("no NUL bytes remain before `end`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_conversion() {
        let c = KColor { r: 1.0, g: 0.3, b: -1.0, a: 2.0 };
        assert_eq!(c.to_raylib(), raylib::Color { r: 255, g: 77, b: 0, a: 255 });
        let mid = KColor { r: 0.5, g: 0.0, b: 1.0, a: 0.0 };
        assert_eq!(mid.to_raylib(), raylib::Color { r: 128, g: 0, b: 255, a: 0 });
        // Not a number reads as 0 rather than a random colour.
        assert_eq!(channel(f32::NAN), 0);
    }

    #[test]
    fn strings_read_as_bytes_and_never_dereference_null() {
        let text = "héllo";
        let s = KStr { ptr: text.as_ptr(), len: text.len() as i64 };
        // SAFETY: `s` describes `text`, which outlives the slice.
        unsafe {
            assert_eq!(str_bytes(&s), text.as_bytes());
            assert_eq!(str_bytes(&KStr { ptr: std::ptr::null(), len: 0 }), b"");
            assert_eq!(str_bytes(&KStr { ptr: std::ptr::null(), len: 5 }), b"", "null wins over a length");
            assert_eq!(str_bytes(&KStr { ptr: text.as_ptr(), len: 0 }), b"");
            assert_eq!(str_bytes(std::ptr::null()), b"");
        }
    }

    #[test]
    fn arrays_read_as_slices_and_never_dereference_null() {
        let colors = [KColor { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }, KColor { r: 0.0, g: 0.0, b: 1.0, a: 1.0 }];
        let header = KSeq { ptr: colors.as_ptr().cast(), len: 2 };
        // SAFETY: `header` describes `colors`, which outlives the slices.
        unsafe {
            assert_eq!(elems::<KColor>(&header), colors);
            assert_eq!(elems::<KColor>(&KSeq { ptr: colors.as_ptr().cast(), len: 1 }), &colors[..1]);
            assert!(elems::<KColor>(&KSeq { ptr: std::ptr::null(), len: 0 }).is_empty());
            assert!(elems::<KColor>(&KSeq { ptr: std::ptr::null(), len: 5 }).is_empty(), "null wins over a length");
            assert!(elems::<KColor>(&KSeq { ptr: colors.as_ptr().cast(), len: 0 }).is_empty());
            assert!(elems::<KColor>(&KSeq { ptr: colors.as_ptr().cast(), len: -1 }).is_empty());
            assert!(elems::<KColor>(std::ptr::null()).is_empty());
        }
    }

    #[test]
    fn the_layouts_are_kestrels() {
        use std::mem::{align_of, offset_of, size_of};
        // `Texture { id: GpuHandle, width: int, height: int }`: an i64 then two i32s.
        assert_eq!((size_of::<KTexture>(), align_of::<KTexture>()), (16, 8));
        assert_eq!((offset_of!(KTexture, id), offset_of!(KTexture, width), offset_of!(KTexture, height)), (0, 8, 12));
        // `Rect { pos: vec2, size: vec2 }`: four floats.
        assert_eq!(size_of::<KRect>(), 16);
        assert_eq!((offset_of!(KRect, pos), offset_of!(KRect, size)), (0, 8));
        // The first two words of an array or list header.
        #[cfg(target_pointer_width = "64")]
        assert_eq!((size_of::<KSeq>(), offset_of!(KSeq, len)), (16, 8));
        // A `vec3` is three floats with no padding.
        assert_eq!((size_of::<V3>(), align_of::<V3>()), (12, 4));
        assert_eq!((offset_of!(V3, x), offset_of!(V3, y), offset_of!(V3, z)), (0, 4, 8));
        // `Camera { pos: vec3, target: vec3, up: vec3, fov: float }`: 40 bytes, and a `[Camera]`
        // steps by 40. (`codegen/tests/natives_arrays.rs` checks that compiled code agrees.)
        assert_eq!((size_of::<KCamera>(), align_of::<KCamera>()), (40, 4));
        let offsets = (offset_of!(KCamera, pos), offset_of!(KCamera, target), offset_of!(KCamera, up), offset_of!(KCamera, fov));
        assert_eq!(offsets, (0, 12, 24, 36));
    }

    #[test]
    fn a_camera_converts_to_a_perspective_raylib_camera() {
        let cam = KCamera {
            pos: V3 { x: 1.0, y: 2.0, z: 3.0 },
            target: V3 { x: 4.0, y: 5.0, z: 6.0 },
            up: V3 { x: 0.0, y: 1.0, z: 0.0 },
            fov: 60.0,
        };
        let c = cam.to_raylib();
        let at = |x, y, z| raylib::Vector3 { x, y, z };
        assert_eq!((c.position, c.target, c.up), (at(1.0, 2.0, 3.0), at(4.0, 5.0, 6.0), at(0.0, 1.0, 0.0)));
        assert_eq!((c.fovy, c.projection), (60.0, raylib::CAMERA_PERSPECTIVE));
    }

    #[test]
    fn c_strings_end_at_the_first_nul() {
        assert_eq!(c_string(b"abc").as_bytes(), b"abc");
        assert_eq!(c_string(b"ab\0cd").as_bytes(), b"ab");
        assert_eq!(c_string(b"").as_bytes(), b"");
    }

    #[test]
    fn unaligned_reads_and_writes() {
        // A `vec2` at an odd offset, as a compiled stack slot might be.
        let mut buf = [0u8; 12];
        // SAFETY: bytes 1..9 are inside `buf`.
        unsafe {
            write(buf.as_mut_ptr().add(1).cast::<V2>(), V2 { x: 1.5, y: -2.0 });
            assert_eq!(read(buf.as_ptr().add(1).cast::<V2>()), V2 { x: 1.5, y: -2.0 });
        }
    }
}
