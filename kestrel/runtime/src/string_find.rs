//! Searching and cutting strings: `find`, `starts_with`, `ends_with` and `slice`. Positions
//! the program sees count characters (see `string`); these functions turn them into byte offsets.

use crate::panic::{fail, of_file, FileName};
use crate::string::{char_offset, chars, KStr};

/// The byte offset of the first match of `sub` in `bytes`, or `None`. An empty `sub` matches at 0.
/// UTF-8 can't match in the middle of a character, so a byte search finds only whole ones.
pub fn find_bytes(bytes: &[u8], sub: &[u8]) -> Option<usize> {
    if sub.is_empty() {
        return Some(0);
    }
    bytes.windows(sub.len()).position(|w| w == sub)
}

/// The character index of the first match of `sub`, or -1.
///
/// # Safety
/// Both strings must be valid for reading their bytes unless the pointer is null.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_find(ptr: *const u8, len: i64, sub: *const u8, sub_len: i64) -> i64 {
    // SAFETY: the caller guarantees both strings.
    let (bytes, sub) = unsafe { (KStr::bytes(ptr, len), KStr::bytes(sub, sub_len)) };
    match find_bytes(bytes, sub) {
        Some(at) => chars(&bytes[..at]),
        None => -1,
    }
}

/// Whether the string starts with `sub` (1 or 0).
///
/// # Safety
/// As `kestrel_rt_str_find`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_starts(ptr: *const u8, len: i64, sub: *const u8, sub_len: i64) -> i8 {
    // SAFETY: the caller guarantees both strings.
    let (bytes, sub) = unsafe { (KStr::bytes(ptr, len), KStr::bytes(sub, sub_len)) };
    bytes.starts_with(sub) as i8
}

/// Whether the string ends with `sub` (1 or 0).
///
/// # Safety
/// As `kestrel_rt_str_find`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_ends(ptr: *const u8, len: i64, sub: *const u8, sub_len: i64) -> i8 {
    // SAFETY: the caller guarantees both strings.
    let (bytes, sub) = unsafe { (KStr::bytes(ptr, len), KStr::bytes(sub, sub_len)) };
    bytes.ends_with(sub) as i8
}

/// A new string of the characters `from..to`, written at `out`. Panics unless
/// `0 <= from <= to <= chars`.
///
/// # Safety
/// As `kestrel_rt_str_find`; `out` must be valid for writing 16 bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_slice(ptr: *const u8, len: i64, from: i64, to: i64, line: i32, file: *const FileName, out: *mut KStr) {
    // SAFETY: the caller guarantees the string.
    let bytes = unsafe { KStr::bytes(ptr, len) };
    let n = chars(bytes);
    if from < 0 || to < from || to > n {
        fail(&format!("slice {from} to {to} is out of bounds for a string of {n} characters on line {line}{}", of_file(file)));
    }
    // A character index equal to the length is the end of the string.
    let at = |i: i64| char_offset(bytes, i).unwrap_or(bytes.len());
    // SAFETY: the caller guarantees `out`.
    unsafe { KStr::make(&bytes[at(from)..at(to)], out) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_arena(f: impl FnOnce() + Send + 'static) {
        std::thread::spawn(f).join().unwrap();
    }

    fn find(s: &str, sub: &str) -> i64 {
        // SAFETY: valid strings.
        unsafe { kestrel_rt_str_find(s.as_ptr(), s.len() as i64, sub.as_ptr(), sub.len() as i64) }
    }

    #[test]
    fn find_gives_a_character_index() {
        assert_eq!(find("héllo", "ll"), 2);
        assert_eq!(find("héllo", "é"), 1);
        assert_eq!(find("héllo", "z"), -1);
        assert_eq!(find("a😀b", "b"), 2);
        assert_eq!(find("abcabc", "c"), 2);
    }

    #[test]
    fn find_with_empty_inputs() {
        assert_eq!(find("", ""), 0);
        assert_eq!(find("abc", ""), 0);
        assert_eq!(find("", "a"), -1);
        // SAFETY: null pointers are empty strings.
        assert_eq!(unsafe { kestrel_rt_str_find(std::ptr::null(), 0, std::ptr::null(), 0) }, 0);
    }

    #[test]
    fn starts_and_ends() {
        let (s, p, q, e) = ("héllo", "hé", "lo", "");
        // SAFETY: valid strings.
        unsafe {
            let (s_ptr, s_len) = (s.as_ptr(), s.len() as i64);
            assert_eq!(kestrel_rt_str_starts(s_ptr, s_len, p.as_ptr(), p.len() as i64), 1);
            assert_eq!(kestrel_rt_str_starts(s_ptr, s_len, q.as_ptr(), q.len() as i64), 0);
            assert_eq!(kestrel_rt_str_ends(s_ptr, s_len, q.as_ptr(), q.len() as i64), 1);
            assert_eq!(kestrel_rt_str_ends(s_ptr, s_len, p.as_ptr(), p.len() as i64), 0);
            assert_eq!(kestrel_rt_str_starts(s_ptr, s_len, e.as_ptr(), 0), 1);
            assert_eq!(kestrel_rt_str_ends(s_ptr, s_len, e.as_ptr(), 0), 1);
            assert_eq!(kestrel_rt_str_ends(std::ptr::null(), 0, e.as_ptr(), 0), 1);
            assert_eq!(kestrel_rt_str_ends(std::ptr::null(), 0, q.as_ptr(), 2), 0);
        }
    }

    fn slice(s: &str, from: i64, to: i64) -> String {
        let mut out = KStr { ptr: std::ptr::null(), len: 0 };
        // SAFETY: valid string; `out` is a `KStr`.
        unsafe { kestrel_rt_str_slice(s.as_ptr(), s.len() as i64, from, to, 0, std::ptr::null(), &mut out) };
        // SAFETY: made by `KStr::make`.
        String::from_utf8(unsafe { KStr::bytes(out.ptr, out.len) }.to_vec()).unwrap()
    }

    #[test]
    fn slice_counts_characters() {
        in_arena(|| {
            assert_eq!(slice("héllo", 2, 4), "ll");
            assert_eq!(slice("héllo", 1, 2), "é");
            assert_eq!(slice("héllo", 3, 3), "");
            assert_eq!(slice("héllo", 0, 5), "héllo");
            assert_eq!(slice("héllo", 5, 5), "");
            assert_eq!(slice("a😀b", 1, 3), "😀b");
            assert_eq!(slice("", 0, 0), "");
        });
    }

    #[test]
    fn an_empty_slice_is_the_zero_string() {
        in_arena(|| {
            let mut out = KStr { ptr: std::ptr::dangling(), len: 9 };
            // SAFETY: valid string; `out` is a `KStr`.
            unsafe { kestrel_rt_str_slice(b"abc".as_ptr(), 3, 1, 1, 0, std::ptr::null(), &mut out) };
            assert!(out.ptr.is_null());
            assert_eq!(out.len, 0);
        });
    }
}
