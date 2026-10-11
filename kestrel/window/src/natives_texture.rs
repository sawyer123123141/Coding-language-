//! The `draw` module's texture natives (`driver/std/draw.kes`): making a texture, and
//! drawing one.
//!
//! A `Texture` reaches a native as a pointer to its 16 bytes (`KTexture`): the id of a GPU
//! object (`gpu.rs`) and the size it had when it was made. The object is what counts. It
//! holds the real raylib texture and the real size, because a program may assign to a
//! `Texture`'s plain `width` field. A native that makes a texture writes it through the
//! pointer that comes first.
//!
//! A texture is drawn with nearest filtering, so pixels stay crisp, and coordinates outside
//! it repeat it. A headless window reads image files (for their size and their errors) and
//! checks pixel counts like a real one, but keeps no GPU texture.

use std::path::Path;

use kestrel_runtime::panic::fail_at_call;

use crate::abi::{elems, read, span, str_bytes, write, KColor, KRect, KSeq, KStr, KTexture, V2};
use crate::gpu::{self, Object};
use crate::headless::{self, Arg};
use crate::image_file;
use crate::pass;
use crate::paths;
use crate::raylib as rl;
use crate::state::{self, Mode};

/// Reads the image file `path` into a texture. A relative path means the program's folder.
///
/// # Safety
/// `out` must be valid for writing a `Texture`, `path` must point to a live string header.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_load_texture(out: *mut KTexture, path: *const KStr) {
    let path = str_bytes(path);
    let mode = state::drawing("draw.load_texture");
    let texture = load_texture_in(mode, path, &paths::resolve(path)).unwrap_or_else(|msg| fail_at_call(&msg));
    if mode == Mode::Headless {
        headless::line("draw.load_texture", &[Arg::Str(path)]);
    }
    write(out, texture);
}

/// Makes a texture from `pixels`, given row by row from the top-left.
///
/// # Safety
/// `out` must be valid for writing a `Texture`, `pixels` must point to a live array or list of `Color`s.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_make_texture(out: *mut KTexture, width: i32, height: i32, pixels: *const KSeq) {
    let colors = elems::<KColor>(pixels);
    let mode = state::drawing("draw.make_texture");
    let texture = make_texture_in(mode, width, height, colors).unwrap_or_else(|msg| fail_at_call(&msg));
    if mode == Mode::Headless {
        headless::line("draw.make_texture", &[Arg::Int(width), Arg::Int(height), Arg::Count(colors.len() as i64)]);
    }
    write(out, texture);
}

/// Draws the whole texture at its natural size, its top-left corner at `pos`.
///
/// # Safety
/// `tex` must point to a live `Texture`, `pos` to a live `vec2`, `tint` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_texture(tex: *const KTexture, pos: *const V2, tint: *const KColor) {
    let (tex, pos, tint) = (read(tex), read(pos), read(tint));
    let args = [Arg::Texture(tex), Arg::Vec2(pos), Arg::Color(tint)];
    sprite("draw.texture", &args, tex, tint, |w, h| {
        place(pos, V2 { x: w as f32, y: h as f32 }, whole(w, h))
    });
}

/// Stretches the whole texture over the rectangle `pos`/`size`.
///
/// # Safety
/// `tex` must point to a live `Texture`, `pos` and `size` to live `vec2`s, `tint` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_texture_rect(tex: *const KTexture, pos: *const V2, size: *const V2, tint: *const KColor) {
    let (tex, pos, size, tint) = (read(tex), read(pos), read(size), read(tint));
    let args = [Arg::Texture(tex), Arg::Vec2(pos), Arg::Vec2(size), Arg::Color(tint)];
    sprite("draw.texture_rect", &args, tex, tint, |w, h| place(pos, size, whole(w, h)));
}

/// Stretches the part `region` of the texture (in texture pixels) over the rectangle `pos`/`size`.
///
/// # Safety
/// `tex` must point to a live `Texture`, `region` to a live `Rect`, `pos` and `size` to live
/// `vec2`s, `tint` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw_texture_region(
    tex: *const KTexture,
    region: *const KRect,
    pos: *const V2,
    size: *const V2,
    tint: *const KColor,
) {
    let (tex, region, pos, size, tint) = (read(tex), read(region), read(pos), read(size), read(tint));
    let args = [Arg::Texture(tex), Arg::Rect(region), Arg::Vec2(pos), Arg::Vec2(size), Arg::Color(tint)];
    sprite("draw.texture_region", &args, tex, tint, |_, _| place(pos, size, region));
}

/// What the registry holds for a `Texture`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Found {
    /// The raylib texture. A headless window has none.
    pub texture: Option<rl::Texture>,
    pub width: i32,
    pub height: i32,
}

/// The GPU object `tex` stands for. An id of 0 (a zero-filled `Texture`) or of an object that
/// was freed gives the error for `name`, the native that got it.
pub(crate) fn lookup(name: &str, tex: KTexture) -> Result<Found, String> {
    gpu::with(tex.id, |object| match object {
        Object::Texture { texture, width, height } => Some(Found { texture: *texture, width: *width, height: *height }),
        Object::Mesh { .. } => None,
    })
    .flatten()
    .ok_or_else(|| format!("{name} got an empty texture; make one with draw.load_texture or draw.make_texture"))
}

/// What the three drawing natives share: check the window, end a 3D pass if one is going (a sprite
/// is 2D), check the texture, then print the call's line (headless) or draw. `layout` gives the
/// source and destination rectangles for a texture of the width and height given.
fn sprite(
    name: &'static str,
    args: &[Arg],
    tex: KTexture,
    tint: KColor,
    layout: impl FnOnce(i32, i32) -> (rl::Rectangle, rl::Rectangle),
) {
    let mode = state::drawing(name);
    pass::enter_2d(mode == Mode::Real);
    let found = lookup(name, tex).unwrap_or_else(|msg| fail_at_call(&msg));
    match mode {
        Mode::Headless => headless::line(name, args),
        Mode::Real => {
            // A real window only ever registers textures that raylib made.
            if let Some(texture) = found.texture {
                let (source, dest) = layout(found.width, found.height);
                let origin = rl::Vector2 { x: 0.0, y: 0.0 };
                // SAFETY: the window is open, on this thread, and `texture` is a live texture of it.
                unsafe { rl::DrawTexturePro(texture, source, dest, origin, 0.0, tint.to_raylib()) }
            }
        }
    }
}

/// The whole of a `width` by `height` texture.
fn whole(width: i32, height: i32) -> KRect {
    KRect { pos: V2 { x: 0.0, y: 0.0 }, size: V2 { x: width as f32, y: height as f32 } }
}

/// Where `DrawTexturePro` reads and where it draws, as `(source, dest)`.
///
/// - A negative `size` covers the rectangle it spans from `pos`, mirrored: `dest` starts at
///   `pos + size` and is as wide as `size` is long.
/// - `region` is the part read, from its `pos` and as big as its `size` is long. A negative
///   `region` size mirrors that part.
/// - Mirroring is a negative source width or height, which raylib flips. The two mirrorings
///   cancel each other when a side is negative in both.
pub(crate) fn place(pos: V2, size: V2, region: KRect) -> (rl::Rectangle, rl::Rectangle) {
    let (corner, extent) = span(pos, size);
    let flip = |drawn: f32, read: f32| (drawn < 0.0) != (read < 0.0);
    let signed = |length: f32, flipped: bool| if flipped { -length.abs() } else { length.abs() };
    let source = rl::Rectangle {
        x: region.pos.x,
        y: region.pos.y,
        width: signed(region.size.x, flip(size.x, region.size.x)),
        height: signed(region.size.y, flip(size.y, region.size.y)),
    };
    (source, rl::Rectangle { x: corner.x, y: corner.y, width: extent.x, height: extent.y })
}

/// `draw.load_texture` for the file `resolved`, which `path` (as the program wrote it) stands for.
/// A real window uploads the image; a headless one only reads it.
pub(crate) fn load_texture_in(mode: Mode, path: &[u8], resolved: &Path) -> Result<KTexture, String> {
    // A missing file says so before anything is decoded, so it isn't told it isn't an image.
    let image = image_file::load(resolved).map_err(|failure| {
        format!(
            "draw.load_texture couldn't read {} (looked for {}): {}",
            String::from_utf8_lossy(path),
            paths::display(resolved),
            failure.reason()
        )
    })?;
    let texture = match mode {
        Mode::Headless => Ok(None),
        Mode::Real => upload(image).map(Some).ok_or_else(|| {
            too_big(&format!("draw.load_texture couldn't use {}", String::from_utf8_lossy(path)))
        }),
    };
    // SAFETY: `image` was loaded above and is freed once. A texture keeps its own copy of the pixels.
    unsafe { rl::UnloadImage(image) };
    Ok(register(texture?, image.width, image.height))
}

/// `draw.make_texture` for these `colors`.
pub(crate) fn make_texture_in(mode: Mode, width: i32, height: i32, colors: &[KColor]) -> Result<KTexture, String> {
    if width <= 0 || height <= 0 {
        return Err(format!("draw.make_texture needs a width and height above 0, not {width} by {height}"));
    }
    // In 64 bits, so two big sizes can't wrap round to a count that matches.
    let wanted = i64::from(width) * i64::from(height);
    if wanted != colors.len() as i64 {
        return Err(format!(
            "draw.make_texture needs width \u{d7} height pixels: {width} by {height} is {wanted}, not {}",
            colors.len()
        ));
    }
    let texture = match mode {
        Mode::Headless => None,
        Mode::Real => {
            let message = || too_big(&format!("draw.make_texture couldn't make a {width} by {height} texture"));
            Some(upload_pixels(width, height, colors).ok_or_else(message)?)
        }
    };
    Ok(register(texture, width, height))
}

/// The message for a texture raylib could not make, which means the driver refused it.
fn too_big(what: &str) -> String {
    format!("{what}; the image may be too big for this graphics driver")
}

/// Puts `texture` (none in a headless window) in the registry; gives the `Texture` value for it.
fn register(texture: Option<rl::Texture>, width: i32, height: i32) -> KTexture {
    KTexture { id: gpu::add(Object::Texture { texture, width, height }), width, height }
}

/// A texture of `colors`, converted to RGBA bytes row by row. The bytes are Rust's, and are freed
/// by Rust: raylib only copies them to the GPU.
fn upload_pixels(width: i32, height: i32, colors: &[KColor]) -> Option<rl::Texture> {
    let mut bytes: Vec<rl::Color> = colors.iter().map(|c| c.to_raylib()).collect();
    let image = rl::Image {
        data: bytes.as_mut_ptr().cast(),
        width,
        height,
        mipmaps: 1,
        format: rl::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8,
    };
    upload(image)
}

/// Sends `image` to the GPU as a texture that is drawn with nearest filtering and repeats outside
/// itself. `None` if raylib made no texture (its id is 0).
fn upload(image: rl::Image) -> Option<rl::Texture> {
    // SAFETY: only called while a real window is open, on its thread. `image` holds pixel data
    // that stays alive through the call; `LoadTextureFromImage` copies it.
    unsafe {
        let texture = rl::LoadTextureFromImage(image);
        if texture.id == 0 {
            return None;
        }
        rl::SetTextureFilter(texture, rl::TEXTURE_FILTER_POINT);
        rl::SetTextureWrap(texture, rl::TEXTURE_WRAP_REPEAT);
        Some(texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v2(x: f32, y: f32) -> V2 {
        V2 { x, y }
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> rl::Rectangle {
        rl::Rectangle { x, y, width, height }
    }

    /// The part of a 16 by 16 texture the sprite calls read: its top-left quarter.
    fn quarter() -> KRect {
        KRect { pos: v2(0.0, 0.0), size: v2(8.0, 8.0) }
    }

    #[test]
    fn a_positive_size_and_region_are_used_as_given() {
        let (source, dest) = place(v2(10.0, 20.0), v2(32.0, 16.0), quarter());
        assert_eq!(source, rect(0.0, 0.0, 8.0, 8.0));
        assert_eq!(dest, rect(10.0, 20.0, 32.0, 16.0));
    }

    #[test]
    fn a_negative_size_covers_the_rectangle_it_spans_and_mirrors() {
        // x only: the destination reaches back from `pos`, the source is read right to left.
        let (source, dest) = place(v2(64.0, 32.0), v2(-32.0, 16.0), quarter());
        assert_eq!(dest, rect(32.0, 32.0, 32.0, 16.0));
        assert_eq!(source, rect(0.0, 0.0, -8.0, 8.0));
        // y only.
        let (source, dest) = place(v2(10.0, 20.0), v2(32.0, -16.0), quarter());
        assert_eq!(dest, rect(10.0, 4.0, 32.0, 16.0));
        assert_eq!(source, rect(0.0, 0.0, 8.0, -8.0));
        // Both.
        let (source, dest) = place(v2(10.0, 20.0), v2(-4.0, -6.0), quarter());
        assert_eq!(dest, rect(6.0, 14.0, 4.0, 6.0));
        assert_eq!(source, rect(0.0, 0.0, -8.0, -8.0));
    }

    #[test]
    fn a_negative_region_mirrors_the_part_read_in_place() {
        let region = KRect { pos: v2(4.0, 2.0), size: v2(-8.0, 6.0) };
        let (source, dest) = place(v2(10.0, 20.0), v2(32.0, 16.0), region);
        // The part is still the one at (4, 2), 8 wide; it is only read the other way round.
        assert_eq!(source, rect(4.0, 2.0, -8.0, 6.0));
        assert_eq!(dest, rect(10.0, 20.0, 32.0, 16.0));
        let region = KRect { pos: v2(4.0, 2.0), size: v2(8.0, -6.0) };
        let (source, _) = place(v2(10.0, 20.0), v2(32.0, 16.0), region);
        assert_eq!(source, rect(4.0, 2.0, 8.0, -6.0));
    }

    #[test]
    fn two_mirrorings_of_a_side_cancel() {
        let region = KRect { pos: v2(4.0, 2.0), size: v2(-8.0, -6.0) };
        let (source, dest) = place(v2(64.0, 32.0), v2(-32.0, 16.0), region);
        assert_eq!(dest, rect(32.0, 32.0, 32.0, 16.0));
        // x is negative in both, so it reads left to right again; y is negative only in the region.
        assert_eq!(source, rect(4.0, 2.0, 8.0, -6.0));
        let (source, dest) = place(v2(0.0, 0.0), v2(-5.0, -5.0), region);
        assert_eq!(dest, rect(-5.0, -5.0, 5.0, 5.0));
        assert_eq!(source, rect(4.0, 2.0, 8.0, 6.0), "both sides cancel: nothing is mirrored");
    }

    #[test]
    fn a_zero_size_stays_zero() {
        let (source, dest) = place(v2(3.0, 4.0), v2(0.0, 0.0), KRect { pos: v2(1.0, 1.0), size: v2(0.0, 0.0) });
        assert_eq!(dest, rect(3.0, 4.0, 0.0, 0.0));
        assert_eq!(source, rect(1.0, 1.0, 0.0, 0.0));
    }

    #[test]
    fn the_whole_texture_is_its_full_size_from_the_corner() {
        assert_eq!(whole(4, 2), KRect { pos: v2(0.0, 0.0), size: v2(4.0, 2.0) });
    }
}
