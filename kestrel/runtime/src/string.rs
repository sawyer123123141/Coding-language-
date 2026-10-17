//! Strings, as compiled code lays them out: a pointer and a length in bytes. The text is UTF-8
//! (it comes only from source text, interpolation and these functions), and lengths and indexes
//! seen by the program count characters. A null pointer with length 0 is the empty string
//! everywhere, as in a zero-filled string. Every string made here is new, in the current arena.

use crate::arena::kestrel_rt_dup;
use crate::panic::{kestrel_rt_panic_index, FileName};

/// A string value.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KStr {
    pub ptr: *const u8,
    pub len: i64,
}

impl KStr {
    /// The bytes. A null pointer is the empty string whatever the length says.
    ///
    /// # Safety
    /// `ptr` must be valid for reading `len` bytes unless it is null.
    pub unsafe fn bytes<'a>(ptr: *const u8, len: i64) -> &'a [u8] {
        if ptr.is_null() || len <= 0 {
            return &[];
        }
        // SAFETY: the caller guarantees `ptr` is valid for `len` bytes.
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }

    /// A copy of `bytes` in the current arena, written at `out`. No bytes give a null pointer and 0,
    /// as a zero-filled string is.
    ///
    /// # Safety
    /// `out` must be valid for writing 16 bytes (not necessarily aligned).
    pub unsafe fn make(bytes: &[u8], out: *mut KStr) {
        let ptr = if bytes.is_empty() {
            std::ptr::null()
        } else {
            // SAFETY: `bytes` is a valid slice.
            unsafe { kestrel_rt_dup(bytes.as_ptr(), bytes.len() as i64, 1) }.cast_const()
        };
        // SAFETY: the caller guarantees `out`; compiled code's slot may not be 8-aligned.
        unsafe { out.write_unaligned(KStr { ptr, len: bytes.len() as i64 }) };
    }
}

/// Whether `b` continues a character that started earlier (`10xxxxxx`).
fn is_continuation(b: u8) -> bool {
    b & 0xC0 == 0x80
}

/// The byte length of the character that starts with the lead byte `b`. A stray continuation
/// byte (not valid UTF-8) counts as one.
fn width(b: u8) -> usize {
    match b {
        0xF0.. => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}

/// The number of characters in `bytes`.
pub fn chars(bytes: &[u8]) -> i64 {
    bytes.iter().filter(|&&b| !is_continuation(b)).count() as i64
}

/// The byte offset where character `i` starts, or `None` when `i` is not in `0..chars`.
pub fn char_offset(bytes: &[u8], i: i64) -> Option<usize> {
    if i < 0 {
        return None;
    }
    let (mut at, mut seen) = (0, 0);
    while at < bytes.len() {
        if seen == i {
            return Some(at);
        }
        at = (at + width(bytes[at])).min(bytes.len());
        seen += 1;
    }
    None
}

/// The number of characters in the string.
///
/// # Safety
/// `ptr` must be valid for reading `len` bytes unless it is null.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_chars(ptr: *const u8, len: i64) -> i64 {
    // SAFETY: the caller guarantees the string.
    chars(unsafe { KStr::bytes(ptr, len) })
}

/// A new one-character string, character `i` of the string, written at `out`. Panics unless
/// `0 <= i < chars`.
///
/// # Safety
/// As `kestrel_rt_str_chars`; `out` must be valid for writing 16 bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_at(ptr: *const u8, len: i64, i: i64, line: i32, file: *const FileName, out: *mut KStr) {
    // SAFETY: the caller guarantees the string.
    let bytes = unsafe { KStr::bytes(ptr, len) };
    let Some(at) = char_offset(bytes, i) else {
        kestrel_rt_panic_index(i, chars(bytes), line, file);
    };
    let end = (at + width(bytes[at])).min(bytes.len());
    // SAFETY: the caller guarantees `out`.
    unsafe { KStr::make(&bytes[at..end], out) };
}

/// The byte length of the character that starts at byte `pos`. Only called with `pos < len`.
///
/// # Safety
/// As `kestrel_rt_str_chars`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_char_len(ptr: *const u8, len: i64, pos: i64) -> i64 {
    // SAFETY: the caller guarantees the string.
    let bytes = unsafe { KStr::bytes(ptr, len) };
    match bytes.get(pos as usize) {
        Some(&b) => width(b).min(bytes.len() - pos as usize) as i64,
        None => 0,
    }
}

/// A new string of the bytes `pos..pos + n`, written at `out`. The range is clamped to the string.
///
/// # Safety
/// As `kestrel_rt_str_chars`; `out` must be valid for writing 16 bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_piece(ptr: *const u8, len: i64, pos: i64, n: i64, out: *mut KStr) {
    // SAFETY: the caller guarantees the string.
    let bytes = unsafe { KStr::bytes(ptr, len) };
    let start = (pos.max(0) as usize).min(bytes.len());
    let end = start.saturating_add(n.max(0) as usize).min(bytes.len());
    // SAFETY: the caller guarantees `out`.
    unsafe { KStr::make(&bytes[start..end], out) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `f` on a thread of its own: the strings it makes go in that thread's program arena,
    /// which goes with the thread.
    fn in_arena(f: impl FnOnce() + Send + 'static) {
        std::thread::spawn(f).join().unwrap();
    }

    fn text(s: &KStr) -> String {
        // SAFETY: the strings here were made by `KStr::make`.
        String::from_utf8(unsafe { KStr::bytes(s.ptr, s.len) }.to_vec()).unwrap()
    }

    fn empty() -> KStr {
        KStr { ptr: std::ptr::null(), len: 0 }
    }

    #[test]
    fn characters_are_counted_not_bytes() {
        for (s, n) in [("", 0), ("hello", 5), ("héllo", 5), ("日本語", 3), ("a😀b", 3)] {
            // SAFETY: `s` is a valid string.
            assert_eq!(unsafe { kestrel_rt_str_chars(s.as_ptr(), s.len() as i64) }, n, "{s}");
        }
        // SAFETY: a null pointer is the empty string.
        assert_eq!(unsafe { kestrel_rt_str_chars(std::ptr::null(), 0) }, 0);
    }

    #[test]
    fn each_character_is_found_by_its_index() {
        in_arena(|| {
            let s = "aé日😀";
            let mut got = Vec::new();
            for i in 0..4 {
                let mut out = empty();
                // SAFETY: valid string, `out` is a `KStr`.
                unsafe { kestrel_rt_str_at(s.as_ptr(), s.len() as i64, i, 0, std::ptr::null(), &mut out) };
                got.push(text(&out));
            }
            assert_eq!(got, ["a", "é", "日", "😀"]);
        });
    }

    #[test]
    fn walking_by_character_length_covers_the_string() {
        in_arena(|| {
            let s = "héllo😀";
            let (mut pos, mut got) = (0i64, Vec::new());
            while pos < s.len() as i64 {
                // SAFETY: valid string, and `pos < len`.
                let n = unsafe { kestrel_rt_str_char_len(s.as_ptr(), s.len() as i64, pos) };
                let mut out = empty();
                // SAFETY: as above.
                unsafe { kestrel_rt_str_piece(s.as_ptr(), s.len() as i64, pos, n, &mut out) };
                got.push(text(&out));
                pos += n;
            }
            assert_eq!(got.concat(), s);
            assert_eq!(got, ["h", "é", "l", "l", "o", "😀"]);
        });
    }

    #[test]
    fn an_empty_piece_is_the_zero_string() {
        in_arena(|| {
            let mut out = KStr { ptr: std::ptr::dangling(), len: 9 };
            // SAFETY: a null pointer is the empty string; `out` is a `KStr`.
            unsafe { kestrel_rt_str_piece(std::ptr::null(), 0, 0, 0, &mut out) };
            assert!(out.ptr.is_null());
            assert_eq!(out.len, 0);
        });
    }

    #[test]
    fn offsets_outside_the_string_are_none() {
        assert_eq!(char_offset("héllo".as_bytes(), 2), Some(3));
        assert_eq!(char_offset("héllo".as_bytes(), 5), None);
        assert_eq!(char_offset("héllo".as_bytes(), -1), None);
        assert_eq!(char_offset(b"", 0), None);
    }
}
