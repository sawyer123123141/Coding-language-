//! Making new strings: `trim`, `upper`, `lower`, `join` and `split`. Each result is a new string
//! in the current arena (`split` also makes the list), never a view into the source.

use std::borrow::Cow;

use crate::arena::{kestrel_rt_current_arena, List};
use crate::panic::{fail, of_file, FileName};
use crate::string::KStr;
use crate::string_find::find_bytes;

/// The text of `bytes` (strings are UTF-8 by construction; a stray byte becomes U+FFFD).
fn text(bytes: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

/// The string without whitespace (Unicode `White_Space`) at either end, as a new string.
///
/// # Safety
/// The string must be valid for reading its bytes unless the pointer is null; `out` must be
/// valid for writing 16 bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_trim(ptr: *const u8, len: i64, out: *mut KStr) {
    // SAFETY: the caller guarantees the string and `out`.
    unsafe {
        let bytes = KStr::bytes(ptr, len);
        match std::str::from_utf8(bytes) {
            Ok(s) => KStr::make(s.trim().as_bytes(), out),
            Err(_) => KStr::make(bytes, out),
        }
    }
}

/// The string in upper case (`upper` is 1) or lower case (0), by Unicode rules, as a new string.
///
/// # Safety
/// As `kestrel_rt_str_trim`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_case(ptr: *const u8, len: i64, upper: i8, out: *mut KStr) {
    // SAFETY: the caller guarantees the string and `out`.
    unsafe {
        let s = text(KStr::bytes(ptr, len));
        let changed = if upper != 0 { s.to_uppercase() } else { s.to_lowercase() };
        KStr::make(changed.as_bytes(), out);
    }
}

/// The `count` strings at `parts`, with `sep` between them, as a new string.
///
/// # Safety
/// `parts` must point to `count` valid strings (it may be null when `count` is 0); the others as
/// `kestrel_rt_str_trim`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_join(parts: *const KStr, count: i64, sep: *const u8, sep_len: i64, out: *mut KStr) {
    // SAFETY: the caller guarantees the strings and `out`.
    unsafe {
        let sep = KStr::bytes(sep, sep_len);
        let parts: &[KStr] = if parts.is_null() || count <= 0 { &[] } else { std::slice::from_raw_parts(parts, count as usize) };
        let mut joined = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                joined.extend_from_slice(sep);
            }
            joined.extend_from_slice(KStr::bytes(p.ptr, p.len));
        }
        KStr::make(&joined, out);
    }
}

/// The pieces of the string between the separators, as a new `list[string]` written at `out`;
/// each piece is a copy, and the list and its pieces are in the current arena. `"".split(",")`
/// is one empty piece. Panics when the separator is empty.
///
/// # Safety
/// As `kestrel_rt_str_trim`, with `out` valid for writing a `List`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_split(ptr: *const u8, len: i64, sep: *const u8, sep_len: i64, line: i32, file: *const FileName, out: *mut List) {
    // SAFETY: the caller guarantees the strings.
    let (bytes, sep) = unsafe { (KStr::bytes(ptr, len), KStr::bytes(sep, sep_len)) };
    if sep.is_empty() {
        fail(&format!(
            "`split` needs a separator that isn't empty; to go through each character, use `for c in s` on line {line}{}",
            of_file(file)
        ));
    }
    let mut ranges = Vec::new();
    let mut start = 0;
    while let Some(at) = find_bytes(&bytes[start..], sep) {
        // This piece and the last one after it must fit.
        if !crate::arena::items_fit(ranges.len() as i64 + 2) {
            crate::arena::list_too_long();
        }
        ranges.push(start..start + at);
        start += at + sep.len();
    }
    ranges.push(start..bytes.len());
    let size = ranges.len() * std::mem::size_of::<KStr>();
    // SAFETY: the current arena is alive while it's on the stack; `out` is the caller's.
    unsafe {
        let arena = kestrel_rt_current_arena();
        let items = (*arena).alloc_uninit(size, std::mem::align_of::<KStr>()).cast::<KStr>();
        for (k, r) in ranges.iter().enumerate() {
            KStr::make(&bytes[r.clone()], items.add(k));
        }
        out.write_unaligned(List { ptr: items.cast(), len: ranges.len() as i64, cap: ranges.len() as i64, arena });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_arena(f: impl FnOnce() + Send + 'static) {
        std::thread::spawn(f).join().unwrap();
    }

    fn string(s: &KStr) -> String {
        // SAFETY: made by `KStr::make`.
        String::from_utf8(unsafe { KStr::bytes(s.ptr, s.len) }.to_vec()).unwrap()
    }

    fn zero() -> KStr {
        KStr { ptr: std::ptr::null(), len: 0 }
    }

    fn trim(s: &str) -> String {
        let mut out = zero();
        // SAFETY: valid string; `out` is a `KStr`.
        unsafe { kestrel_rt_str_trim(s.as_ptr(), s.len() as i64, &mut out) };
        string(&out)
    }

    fn case(s: &str, upper: bool) -> String {
        let mut out = zero();
        // SAFETY: valid string; `out` is a `KStr`.
        unsafe { kestrel_rt_str_case(s.as_ptr(), s.len() as i64, upper as i8, &mut out) };
        string(&out)
    }

    fn join(parts: &[&str], sep: &str) -> String {
        let mut items = Vec::new();
        for p in parts {
            let mut s = zero();
            // SAFETY: valid string; `s` is a `KStr`.
            unsafe { KStr::make(p.as_bytes(), &mut s) };
            items.push(s);
        }
        let mut out = zero();
        // SAFETY: `items` holds `len` strings.
        unsafe { kestrel_rt_str_join(items.as_ptr(), items.len() as i64, sep.as_ptr(), sep.len() as i64, &mut out) };
        string(&out)
    }

    fn split(s: &str, sep: &str) -> Vec<String> {
        let mut list = List { ptr: std::ptr::null_mut(), len: 9, cap: 9, arena: std::ptr::null_mut() };
        // SAFETY: valid strings; `list` is a `List`.
        unsafe { kestrel_rt_str_split(s.as_ptr(), s.len() as i64, sep.as_ptr(), sep.len() as i64, 0, std::ptr::null(), &mut list) };
        assert_eq!(list.cap, list.len);
        assert_eq!(list.arena, kestrel_rt_current_arena());
        // SAFETY: the list holds `len` strings.
        let items = unsafe { std::slice::from_raw_parts(list.ptr.cast::<KStr>(), list.len as usize) };
        items.iter().map(string).collect()
    }

    #[test]
    fn trim_removes_unicode_whitespace_at_both_ends() {
        in_arena(|| {
            assert_eq!(trim("  hi \n"), "hi");
            assert_eq!(trim("\u{a0}\u{2003}é x\u{3000}"), "é x");
            assert_eq!(trim("a b"), "a b");
            assert_eq!(trim(""), "");
            assert_eq!(trim(" \t "), "");
        });
    }

    #[test]
    fn case_uses_unicode_rules() {
        in_arena(|| {
            assert_eq!(case("héllo", true), "HÉLLO");
            assert_eq!(case("HÉLLO", false), "héllo");
            assert_eq!(case("ß", true), "SS");
            assert_eq!(case("", true), "");
            assert_eq!(case("日本", true), "日本");
        });
    }

    #[test]
    fn join_puts_the_separator_between() {
        in_arena(|| {
            assert_eq!(join(&["a", "b", "c"], ", "), "a, b, c");
            assert_eq!(join(&["é"], "-"), "é");
            assert_eq!(join(&[], "-"), "");
            assert_eq!(join(&["", ""], "-"), "-");
            assert_eq!(join(&["a", "b"], ""), "ab");
            let mut out = KStr { ptr: std::ptr::dangling(), len: 5 };
            // SAFETY: a null array of 0 parts; `out` is a `KStr`.
            unsafe { kestrel_rt_str_join(std::ptr::null(), 0, std::ptr::null(), 0, &mut out) };
            assert!(out.ptr.is_null());
            assert_eq!(out.len, 0);
        });
    }

    #[test]
    fn split_copies_each_piece_into_the_current_arena() {
        in_arena(|| {
            assert_eq!(split("x,y,,z", ","), ["x", "y", "", "z"]);
            assert_eq!(split("", ","), [""]);
            assert_eq!(split(",", ","), ["", ""]);
            assert_eq!(split("héllo wörld", " "), ["héllo", "wörld"]);
            assert_eq!(split("a--b---c", "--"), ["a", "b", "-c"]);
            assert_eq!(split("é😀é", "😀"), ["é", "é"]);
            assert_eq!(split("abc", "abc"), ["", ""]);
            assert_eq!(split("abc", "x"), ["abc"]);
        });
    }
}
