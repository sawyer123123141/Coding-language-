//! The native log: each logged native's id byte, then its result in the byte format of
//! `to_bytes` (`crate::bytes`): a `bool` one byte, an `int` four, an `i64` eight, a `float` its
//! four raw bits, an `f64` its eight, a `vec2` its two floats, and a pair of ints (a texture's or
//! a mesh's plain fields) its two ints.

use crate::bytes::{write_value, RefValue};

use super::NativeId;

/// A native's result, as the native log holds it.
pub trait LogValue: Copy + PartialEq {
    /// How many bytes it takes in the log.
    const SIZE: usize;
    /// Appends its bytes to `out`.
    fn put(self, out: &mut Vec<u8>);
    /// Reads it back from exactly `SIZE` bytes.
    fn get(bytes: &[u8]) -> Self;
}

/// The `N` bytes at the start of `bytes`.
fn first<const N: usize>(bytes: &[u8]) -> [u8; N] {
    bytes[..N].try_into().expect("the log holds the value's bytes")
}

impl LogValue for bool {
    const SIZE: usize = 1;
    fn put(self, out: &mut Vec<u8>) {
        write_value(&RefValue::Bool(self), out);
    }
    fn get(bytes: &[u8]) -> bool {
        bytes[0] != 0
    }
}

impl LogValue for i32 {
    const SIZE: usize = 4;
    fn put(self, out: &mut Vec<u8>) {
        write_value(&RefValue::Int(self), out);
    }
    fn get(bytes: &[u8]) -> i32 {
        i32::from_le_bytes(first(bytes))
    }
}

impl LogValue for i64 {
    const SIZE: usize = 8;
    fn put(self, out: &mut Vec<u8>) {
        write_value(&RefValue::I64(self), out);
    }
    fn get(bytes: &[u8]) -> i64 {
        i64::from_le_bytes(first(bytes))
    }
}

impl LogValue for f32 {
    const SIZE: usize = 4;
    fn put(self, out: &mut Vec<u8>) {
        write_value(&RefValue::Float(self), out);
    }
    fn get(bytes: &[u8]) -> f32 {
        f32::from_bits(u32::from_le_bytes(first(bytes)))
    }
}

impl LogValue for f64 {
    const SIZE: usize = 8;
    fn put(self, out: &mut Vec<u8>) {
        write_value(&RefValue::F64(self), out);
    }
    fn get(bytes: &[u8]) -> f64 {
        f64::from_bits(u64::from_le_bytes(first(bytes)))
    }
}

/// A `vec2`.
impl LogValue for [f32; 2] {
    const SIZE: usize = 8;
    fn put(self, out: &mut Vec<u8>) {
        self[0].put(out);
        self[1].put(out);
    }
    fn get(bytes: &[u8]) -> [f32; 2] {
        [f32::get(bytes), f32::get(&bytes[4..])]
    }
}

/// Two ints: an `ivec2`, or a texture's or mesh's plain fields.
impl LogValue for [i32; 2] {
    const SIZE: usize = 8;
    fn put(self, out: &mut Vec<u8>) {
        self[0].put(out);
        self[1].put(out);
    }
    fn get(bytes: &[u8]) -> [i32; 2] {
        [i32::get(bytes), i32::get(&bytes[4..])]
    }
}

/// Where a replay has got to in the native log.
#[derive(Default)]
pub struct Cursor {
    pub at: usize,
}

/// The name of the native whose entry starts with `byte`, for messages.
fn named(byte: u8) -> &'static str {
    NativeId::from_u8(byte).map_or("a native this Kestrel doesn't log", NativeId::name)
}

impl Cursor {
    /// The next logged result, which must be `id`'s, or the call-order message for frame `frame`.
    pub fn next<T: LogValue>(&mut self, log: &[u8], id: NativeId, frame: u64) -> Result<T, String> {
        let differs = |has: &str| format!("replay differs at frame {frame}: the program called {} where the recording has {has}", id.name());
        let Some(&byte) = log.get(self.at) else { return Err(differs("ended")) };
        if byte != id as u8 {
            return Err(differs(named(byte)));
        }
        let start = self.at + 1;
        let Some(bytes) = log.get(start..start + T::SIZE) else { return Err(differs("ended")) };
        self.at = start + T::SIZE;
        Ok(T::get(bytes))
    }

    /// The message for a program that ended (or failed) at frame `frame` while the log has
    /// entries left, or `None` when it was all used.
    pub fn left_over(&self, log: &[u8], frame: u64) -> Option<String> {
        let &byte = log.get(self.at)?;
        Some(format!("replay differs at frame {frame}: the program ended where the recording has {}", named(byte)))
    }
}
