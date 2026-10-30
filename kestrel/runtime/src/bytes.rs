//! The byte format of `to_bytes` (the I/O round's spec, decision 2). Compiled code writes the
//! bytes itself, field by field, into one `list[u8]` it sizes first; it comes here to give that
//! list its room and to copy a string's bytes in. `encode` is a reference encoder in Rust over
//! `RefValue`, for tests and tools.
//!
//! The format, little-endian throughout and with no padding: a header of the 4 bytes `KESB`
//! (`MAGIC`), the format version (`VERSION`) and the type's fingerprint as an `i64` (the
//! `checksum` of the type's description, see `fingerprint`), 13 bytes in all; then the value. A
//! `bool` is one byte, 0 or 1; a `u8` one byte; an `int` or `float` four; an `i64` or `f64` eight
//! (a float's raw bits, NaN payloads kept); a vector its lanes in order. A string is a `u32` byte
//! length, then its UTF-8 bytes. An array or list is a `u32` length, then its items; a map a `u32`
//! live count, then each live key and value in insertion order. An optional is a byte, 0 or 1,
//! then its payload when 1; an enum a `u32` variant number, then that variant's fields; a struct
//! its fields in order.

use crate::arena::{list_too_long, List, MAX_ITEMS};
use crate::checksum::{finish, mix_bytes, START};

/// The first four bytes of every value `to_bytes` writes.
pub const MAGIC: [u8; 4] = *b"KESB";

/// The format version, the header's fifth byte.
pub const VERSION: u8 = 1;

/// The header's length: the magic, the version and the fingerprint.
pub const HEADER: usize = 13;

/// The fingerprint of a type with this description (`struct Game{ball:vec2,score:int}`): the
/// `checksum` of the description as a string.
pub fn fingerprint(description: &str) -> i64 {
    finish(mix_bytes(START, description.as_bytes())) as i64
}

/// Gives the list at `list` room for at least `n` items of `elem_size` bytes: a list with less
/// room moves its items into a new buffer of exactly `n` items, from the list's own arena. The
/// length is unchanged. Fails when `n` is more than a list can hold.
///
/// # Safety
/// `list` must point to a valid `List` whose items are `elem_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_list_reserve(list: *mut List, n: i64, elem_size: i64, align: i64) {
    // SAFETY: the caller guarantees `list`.
    let l = unsafe { &mut *list };
    if n <= l.cap {
        return;
    }
    if n > MAX_ITEMS {
        list_too_long();
    }
    if l.arena.is_null() {
        l.arena = crate::arena::program_arena();
    }
    let size = (n * elem_size) as usize;
    // SAFETY: a list's arena outlives the list; inside a `par for`, other threads may be
    // allocating in the same arena. Room past the length is never read before it's written.
    let p = crate::par::shared_growth(l.arena, || unsafe { (*l.arena).alloc_uninit(size, align as usize) });
    if l.len > 0 {
        // SAFETY: the old buffer holds `len` items; the new one is fresh and bigger.
        unsafe { std::ptr::copy_nonoverlapping(l.ptr, p, (l.len * elem_size) as usize) };
    }
    l.ptr = p;
    l.cap = n;
}

/// Writes the string with these bytes at `dst`: its byte length as a `u32`, then the bytes. A null
/// pointer is the empty string. (Compiled code sizes the list first, and a list can't be longer
/// than `MAX_ITEMS` bytes, so the length fits.)
///
/// # Safety
/// `dst` must be valid for writing `4 + len` bytes, and `ptr` for reading `len` bytes unless it is
/// null.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_bytes_write_str(dst: *mut u8, ptr: *const u8, len: i64) {
    // SAFETY: the caller guarantees `ptr` for `len` bytes.
    let bytes = unsafe { crate::string::KStr::bytes(ptr, len) };
    let count = (bytes.len() as u32).to_le_bytes();
    // SAFETY: the caller guarantees room for the length and the bytes; the string's bytes are
    // never inside the list being written.
    unsafe {
        std::ptr::copy_nonoverlapping(count.as_ptr(), dst, 4);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.add(4), bytes.len());
    }
}

/// A value for the reference encoder: the shape the byte format sees. A vector is a `Struct` of
/// its lanes; an array is a `List`.
#[derive(Clone, Debug, PartialEq)]
pub enum RefValue {
    Bool(bool),
    U8(u8),
    Int(i32),
    I64(i64),
    Float(f32),
    F64(f64),
    Str(String),
    List(Vec<RefValue>),
    Map(Vec<(RefValue, RefValue)>),
    Opt(Option<Box<RefValue>>),
    /// The variant number and that variant's fields.
    Enum(u32, Vec<RefValue>),
    Struct(Vec<RefValue>),
}

/// The bytes `to_bytes` writes for `value` of a type whose fingerprint is `fingerprint`.
pub fn encode(value: &RefValue, fingerprint: i64) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&fingerprint.to_le_bytes());
    write_value(value, &mut out);
    out
}

/// `value`'s bytes, without a header, added to `out`.
pub fn write_value(value: &RefValue, out: &mut Vec<u8>) {
    let count = |n: usize, out: &mut Vec<u8>| out.extend_from_slice(&(n as u32).to_le_bytes());
    match value {
        RefValue::Bool(b) => out.push(u8::from(*b)),
        RefValue::U8(n) => out.push(*n),
        RefValue::Int(n) => out.extend_from_slice(&n.to_le_bytes()),
        RefValue::I64(n) => out.extend_from_slice(&n.to_le_bytes()),
        RefValue::Float(x) => out.extend_from_slice(&x.to_bits().to_le_bytes()),
        RefValue::F64(x) => out.extend_from_slice(&x.to_bits().to_le_bytes()),
        RefValue::Str(s) => {
            count(s.len(), out);
            out.extend_from_slice(s.as_bytes());
        }
        RefValue::List(items) => {
            count(items.len(), out);
            items.iter().for_each(|v| write_value(v, out));
        }
        RefValue::Map(entries) => {
            count(entries.len(), out);
            for (k, v) in entries {
                write_value(k, out);
                write_value(v, out);
            }
        }
        RefValue::Opt(None) => out.push(0),
        RefValue::Opt(Some(v)) => {
            out.push(1);
            write_value(v, out);
        }
        RefValue::Enum(variant, fields) => {
            out.extend_from_slice(&variant.to_le_bytes());
            fields.iter().for_each(|v| write_value(v, out));
        }
        RefValue::Struct(fields) => fields.iter().for_each(|v| write_value(v, out)),
    }
}

#[cfg(test)]
#[path = "bytes_test.rs"]
mod tests;
