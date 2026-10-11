//! Reading and writing image files without raylib's own file functions.
//!
//! raylib opens files with C's `fopen`, which on Windows takes the ANSI code page: a path with a
//! character outside it (a folder called `café`) can't be opened, and raylib only says that it
//! could not load the image. So Rust reads and writes the bytes, and raylib decodes and encodes
//! them in memory.

use std::ffi::c_int;
use std::io;
use std::path::Path;

use crate::abi::c_string;
use crate::raylib as rl;

/// The file types a texture can be loaded from, by extension (the dot is part of it).
const LOADABLE: [&str; 4] = [".png", ".jpg", ".jpeg", ".bmp"];

/// Why an image file could not be loaded.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The file isn't there.
    Missing,
    /// The file couldn't be read; this is the operating system's reason.
    Unreadable(String),
    /// The file was read, but it isn't a PNG, JPG or BMP image.
    NotAnImage,
}

impl Failure {
    /// The reason in the words an error message uses after a colon.
    pub fn reason(&self) -> String {
        match self {
            Failure::Missing => "no such file".to_string(),
            Failure::Unreadable(why) => why.clone(),
            Failure::NotAnImage => "it isn't a PNG, JPG or BMP image".to_string(),
        }
    }
}

/// The type raylib decodes `path` as: its extension with a dot, in lower case, if it is a
/// PNG, JPG or BMP file's. `None` for any other extension, or none.
pub fn file_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    LOADABLE.into_iter().find(|file_type| file_type[1..] == extension)
}

/// Reads and decodes the image file `path`. The pixels belong to raylib: free them with `UnloadImage`.
pub fn load(path: &Path) -> Result<rl::Image, Failure> {
    let bytes = std::fs::read(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => Failure::Missing,
        _ => Failure::Unreadable(os_reason(&e)),
    })?;
    let file_type = file_type(path).ok_or(Failure::NotAnImage)?;
    decode(&bytes, file_type).ok_or(Failure::NotAnImage)
}

/// The operating system's words for `error`, without the "(os error 5)" Rust adds or a final full stop.
fn os_reason(error: &io::Error) -> String {
    let text = error.to_string();
    let text = text.split(" (os error ").next().unwrap_or(&text);
    text.trim_end_matches('.').to_string()
}

/// Decodes `bytes`, the contents of a file of `file_type` (an extension with its dot, in lower case).
/// `None` if they aren't such an image.
fn decode(bytes: &[u8], file_type: &str) -> Option<rl::Image> {
    let size = c_int::try_from(bytes.len()).ok()?;
    let file_type = c_string(file_type.as_bytes());
    // SAFETY: `file_type` is a NUL-terminated string and `bytes` a live slice of `size` bytes; both
    // outlive the call, and `LoadImageFromMemory` copies what it needs. It needs no window.
    let image = unsafe {
        // raylib's loader logs a line to stdout for each image, and a warning for one it can't read.
        rl::SetTraceLogLevel(rl::LOG_ERROR);
        rl::LoadImageFromMemory(file_type.as_ptr(), bytes.as_ptr(), size)
    };
    (!image.data.is_null()).then_some(image)
}

/// The bytes of `image` as a PNG file. `None` if raylib could not encode it.
pub fn encode_png(image: rl::Image) -> Option<Vec<u8>> {
    let mut size: c_int = 0;
    // SAFETY: `image` is a loaded image. On success raylib gives a buffer of `size` bytes, which is
    // copied and then released with `MemFree` once, as raylib asks.
    unsafe {
        let data = rl::ExportImageToMemory(image, c".png".as_ptr(), &mut size);
        if data.is_null() {
            return None;
        }
        let bytes = (size > 0).then(|| std::slice::from_raw_parts(data, size as usize).to_vec());
        rl::MemFree(data.cast());
        bytes
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A folder of its own under the system's temporary one, with a non-ASCII letter in its name.
    fn accented_folder(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kestrel_caf\u{e9}_{label}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn checker() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs/assets/checker.png")
    }

    #[test]
    fn only_png_jpg_and_bmp_extensions_decode_whatever_their_case() {
        for (name, want) in [
            ("a.png", Some(".png")),
            ("a.PNG", Some(".png")),
            ("dir/a.b.Jpg", Some(".jpg")),
            ("a.jpeg", Some(".jpeg")),
            ("a.JPEG", Some(".jpeg")),
            ("a.bmp", Some(".bmp")),
            ("a.gif", None),
            ("a.tga", None),
            ("a.png.txt", None),
            ("a", None),
            ("png", None),
            (".png", None),
        ] {
            assert_eq!(file_type(Path::new(name)), want, "{name}");
        }
    }

    #[test]
    fn an_os_reason_has_no_error_number_or_full_stop() {
        let denied = io::Error::from_raw_os_error(if cfg!(windows) { 5 } else { 13 });
        let reason = os_reason(&denied);
        assert!(!reason.contains("os error") && !reason.ends_with('.') && !reason.is_empty(), "{reason}");
        // An error that isn't from the OS has just its message.
        assert_eq!(os_reason(&io::Error::other("it broke.")), "it broke");
    }

    #[test]
    fn an_image_loads_from_a_folder_with_a_non_ascii_name() {
        let dir = accented_folder("load");
        let path = dir.join("checker.png");
        std::fs::copy(checker(), &path).unwrap();
        let image = load(&path).expect("the checker loads");
        assert_eq!((image.width, image.height), (4, 4));
        // SAFETY: loaded just above and freed once.
        unsafe { rl::UnloadImage(image) };
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn loading_says_why_it_failed() {
        let dir = accented_folder("fail");
        assert_eq!(load(&dir.join("missing.png")).unwrap_err(), Failure::Missing);

        let text = dir.join("text.png");
        std::fs::write(&text, "this is not a PNG").unwrap();
        assert_eq!(load(&text).unwrap_err(), Failure::NotAnImage, "the right extension, the wrong contents");
        let empty = dir.join("empty.png");
        std::fs::write(&empty, "").unwrap();
        assert_eq!(load(&empty).unwrap_err(), Failure::NotAnImage, "an empty file");
        let png = std::fs::read(checker()).unwrap();
        let gif = dir.join("checker.gif");
        std::fs::write(&gif, png).unwrap();
        assert_eq!(load(&gif).unwrap_err(), Failure::NotAnImage, "a PNG under another extension");

        // A folder can't be read as a file; the reason is the system's, and isn't "no such file".
        match load(&dir).unwrap_err() {
            Failure::Unreadable(why) => assert!(!why.is_empty() && !why.contains("os error"), "{why}"),
            other => panic!("a folder gave {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_png_is_encoded_to_bytes_and_decodes_back() {
        let path = checker();
        let image = load(&path).unwrap();
        let bytes = encode_png(image).expect("a loaded image encodes");
        // SAFETY: loaded above and freed once.
        unsafe { rl::UnloadImage(image) };
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");

        // Written to, and read back from, a folder raylib's own file functions can't open.
        let dir = accented_folder("encode");
        let copy = dir.join("copy.png");
        std::fs::write(&copy, &bytes).unwrap();
        let again = load(&copy).unwrap();
        assert_eq!((again.width, again.height), (4, 4));
        // SAFETY: loaded just above and freed once.
        unsafe { rl::UnloadImage(again) };
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
