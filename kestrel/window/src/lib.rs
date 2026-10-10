//! Kestrel's window, drawing and input natives, built on raylib.
//!
//! A staticlib of this crate holds `kestrel-runtime`'s `kestrel_rt_*` symbols as
//! well as raylib's, so a built program links this one library.

// Naming the crate makes rustc link it, so its `kestrel_rt_*` exports land in
// this crate's staticlib.
pub use kestrel_runtime;
pub mod raylib;

use std::ffi::CString;
use std::path::Path;

use raylib::{GetImageColor, LoadImage, UnloadImage};

/// Reads the pixel at `(x, y)` of the image file at `path` as `[r, g, b, a]`.
/// No window is needed. Gives `None` if the file can't be read as an image or
/// the point is outside it.
pub fn png_pixel(path: &Path, x: i32, y: i32) -> Option<[u8; 4]> {
    let c_path = CString::new(path.to_str()?).ok()?;
    // SAFETY: `c_path` is a valid C string. `image` is read only while loaded
    // and is unloaded before returning.
    unsafe {
        let image = LoadImage(c_path.as_ptr());
        if image.data.is_null() {
            return None;
        }
        let inside = (0..image.width).contains(&x) && (0..image.height).contains(&y);
        let pixel = inside.then(|| GetImageColor(image, x, y));
        UnloadImage(image);
        pixel.map(|c| [c.r, c.g, c.b, c.a])
    }
}

#[cfg(test)]
mod tests {
    use super::raylib::{Color, ExportImage, GenImageColor, ImageDrawPixel, UnloadImage};
    use super::*;

    #[test]
    fn png_round_trip() {
        let black = Color { r: 0, g: 0, b: 0, a: 255 };
        let red = Color { r: 255, g: 0, b: 0, a: 255 };
        let path = std::env::temp_dir().join(format!("kestrel_window_{}.png", std::process::id()));
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        // SAFETY: a fresh image, drawn within its bounds, exported and freed once.
        unsafe {
            let mut image = GenImageColor(4, 4, black);
            ImageDrawPixel(&mut image, 2, 1, red);
            assert!(ExportImage(image, c_path.as_ptr()));
            UnloadImage(image);
        }

        assert_eq!(png_pixel(&path, 2, 1), Some([255, 0, 0, 255]));
        assert_eq!(png_pixel(&path, 0, 0), Some([0, 0, 0, 255]));
        assert_eq!(png_pixel(&path, 4, 0), None, "outside the image");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(png_pixel(&path, 0, 0), None, "missing file");
    }
}
