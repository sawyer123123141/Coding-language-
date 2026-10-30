//! The `file` module's natives (`driver/std/file.kes`), each `kestrel_file_<name>`.
//!
//! A path arrives as a pointer to its string header and is found by `paths::resolve`: an absolute
//! path as given, a relative one next to the program. No native stops the program: a failure gives
//! `none` or `false` and records why (`error::set`), a success empties the record. The sentences
//! name the path as the program wrote it.
//!
//! Results that hold memory (the bytes, the text, the names) are made in the current arena, and
//! the checker never runs these calls in a scratch arena, so the caller keeps them.

use std::fs;
use std::io::Read;

use super::abi::{byte_list, elems, str_bytes, string_list, write_none, write_some, KOpt, KSeq};
use super::error;
use super::paths::resolve;
use crate::arena::List;
use crate::string::KStr;

/// A native's `bool`.
fn flag(ok: bool) -> u8 {
    u8::from(ok)
}

/// The bytes of the file at `path`, or `None` with the error recorded.
fn read_bytes(path: &[u8]) -> Option<Vec<u8>> {
    let mut file = match fs::File::open(resolve(path)) {
        Ok(file) => file,
        Err(e) => {
            error::set_io("open", path, &e);
            return None;
        }
    };
    let mut bytes = Vec::new();
    if let Err(e) = file.read_to_end(&mut bytes) {
        error::set_io("read", path, &e);
        return None;
    }
    error::clear();
    Some(bytes)
}

/// Replaces the file at `path` with `bytes`, recording the error on a failure.
fn write_bytes(path: &[u8], bytes: &[u8]) -> bool {
    match fs::write(resolve(path), bytes) {
        Ok(()) => {
            error::clear();
            true
        }
        Err(e) => {
            error::set_io("write", path, &e);
            false
        }
    }
}

/// The names in the folder at `path`, sorted bytewise, or `None` with the error recorded.
fn names_in(path: &[u8]) -> Option<Vec<String>> {
    let entries = match fs::read_dir(resolve(path)) {
        Ok(entries) => entries,
        Err(e) => {
            error::set_io("list", path, &e);
            return None;
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        match entry {
            // A name that isn't Unicode (possible on Linux) is shown with U+FFFD in its place.
            Ok(entry) => names.push(entry.file_name().to_string_lossy().into_owned()),
            Err(e) => {
                error::set_io("list", path, &e);
                return None;
            }
        }
    }
    // `String`'s order is its bytes' order.
    names.sort();
    error::clear();
    Some(names)
}

/// `file.read(path) -> list[u8]?`.
///
/// # Safety
/// `out` must be valid for writing a `list[u8]?` (40 bytes); `path` must point to a live string.
#[no_mangle]
pub unsafe extern "C" fn kestrel_file_read(out: *mut KOpt<List>, path: *const KStr) {
    match read_bytes(str_bytes(path)) {
        Some(bytes) => write_some(out, byte_list(&bytes)),
        None => write_none(out),
    }
}

/// `file.read_text(path) -> string?`: `none` when the file can't be read or isn't UTF-8.
///
/// # Safety
/// `out` must be valid for writing a `string?` (24 bytes); `path` must point to a live string.
#[no_mangle]
pub unsafe extern "C" fn kestrel_file_read_text(out: *mut KOpt<KStr>, path: *const KStr) {
    let path = str_bytes(path);
    let Some(bytes) = read_bytes(path) else {
        write_none(out);
        return;
    };
    if std::str::from_utf8(&bytes).is_err() {
        error::set(error::sentence("read", path, "it is not UTF-8 text"));
        write_none(out);
        return;
    }
    let mut text = KStr { ptr: std::ptr::null(), len: 0 };
    KStr::make(&bytes, &mut text);
    write_some(out, text);
}

/// `file.write(path, bytes: [u8]) -> bool`: the whole file replaced.
///
/// # Safety
/// `path` must point to a live string, `bytes` to a live `[u8]` or `list[u8]` header.
#[no_mangle]
pub unsafe extern "C" fn kestrel_file_write(path: *const KStr, bytes: *const KSeq) -> u8 {
    flag(write_bytes(str_bytes(path), elems::<u8>(bytes)))
}

/// `file.write_text(path, text) -> bool`: the whole file replaced with the text's UTF-8 bytes.
///
/// # Safety
/// `path` and `text` must point to live strings.
#[no_mangle]
pub unsafe extern "C" fn kestrel_file_write_text(path: *const KStr, text: *const KStr) -> u8 {
    flag(write_bytes(str_bytes(path), str_bytes(text)))
}

/// `file.exists(path) -> bool`: a file or a folder is there. It never fails.
///
/// # Safety
/// `path` must point to a live string.
#[no_mangle]
pub unsafe extern "C" fn kestrel_file_exists(path: *const KStr) -> u8 {
    error::clear();
    flag(resolve(str_bytes(path)).exists())
}

/// `file.list_dir(path) -> list[string]?`: the names in a folder, sorted.
///
/// # Safety
/// `out` must be valid for writing a `list[string]?` (40 bytes); `path` must point to a live string.
#[no_mangle]
pub unsafe extern "C" fn kestrel_file_list_dir(out: *mut KOpt<List>, path: *const KStr) {
    match names_in(str_bytes(path)) {
        Some(names) => write_some(out, string_list(&names)),
        None => write_none(out),
    }
}

/// `file.make_dir(path) -> bool`: the folder and any missing folders it sits in.
///
/// # Safety
/// `path` must point to a live string.
#[no_mangle]
pub unsafe extern "C" fn kestrel_file_make_dir(path: *const KStr) -> u8 {
    let path = str_bytes(path);
    match fs::create_dir_all(resolve(path)) {
        Ok(()) => {
            error::clear();
            1
        }
        Err(e) => {
            error::set_io("create", path, &e);
            0
        }
    }
}

/// `file.remove(path) -> bool`: a file, never a folder.
///
/// # Safety
/// `path` must point to a live string.
#[no_mangle]
pub unsafe extern "C" fn kestrel_file_remove(path: *const KStr) -> u8 {
    let path = str_bytes(path);
    let resolved = resolve(path);
    if resolved.is_dir() {
        error::set(error::sentence("remove", path, "it is a folder, and `file.remove` removes only files"));
        return 0;
    }
    match fs::remove_file(resolved) {
        Ok(()) => {
            error::clear();
            1
        }
        Err(e) => {
            error::set_io("remove", path, &e);
            0
        }
    }
}

/// `file.error() -> string`: why the last call of the module failed, or "".
///
/// # Safety
/// `out` must be valid for writing a string (16 bytes).
#[no_mangle]
pub unsafe extern "C" fn kestrel_file_error(out: *mut KStr) {
    KStr::make(error::get().as_bytes(), out);
}
