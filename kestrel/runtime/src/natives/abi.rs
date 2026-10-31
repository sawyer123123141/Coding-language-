//! How compiled Kestrel code hands values to a native, and how a native hands its result back.
//!
//! A native follows Kestrel's own calling convention: scalars travel in registers, and everything
//! bigger travels by pointer. A `string` is a pointer to a 16-byte header, an array or list a
//! pointer to its header (an array's `{ptr, len}` is the start of a list's). An aggregate result
//! (a struct, a string, a list, an optional) is written through a pointer that comes first. The
//! pointers may aim at the caller's own variables, so a native only ever reads through an argument.
//!
//! The compiled code's stack slots are not promised to be aligned for their type, so every access
//! here is unaligned.
//!
//! A result made here lives in the current arena, where the caller's own values are made: a
//! string's bytes, and a list's items with `cap == len` and the current arena as the list's own.
//! An optional `T?` is a tag byte at offset 0 (1 for a value, 0 for `none`) and the `T` at the
//! offset of `T`'s alignment (`KOpt`), as `kir::layout::optional_layout` lays it out.

use crate::arena::{kestrel_rt_current_arena, items_fit, list_too_long, List};
use crate::string::KStr;

/// The first 16 bytes of an array or list header, which the two share: where the elements
/// are, and how many.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KSeq {
    pub ptr: *const u8,
    pub len: i64,
}

/// An optional `T?` as compiled code lays it out: the tag byte, then the value at `T`'s alignment.
/// (`repr(C)` puts `value` at the first offset aligned for `T`, which is Kestrel's rule too.)
#[repr(C)]
pub struct KOpt<T> {
    pub tag: u8,
    pub value: T,
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
    KStr::bytes(s.ptr, s.len)
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

/// A new list in the current arena with room for `n` `T`s and length `n`, its items not written
/// yet. Its capacity is its length.
fn unwritten_list<T>(n: usize) -> List {
    if !items_fit(n as i64) {
        list_too_long();
    }
    let arena = kestrel_rt_current_arena();
    let ptr = if n == 0 {
        std::ptr::null_mut()
    } else {
        // SAFETY: the current arena is alive while it is on the stack.
        unsafe { (*arena).alloc_uninit(n * std::mem::size_of::<T>(), std::mem::align_of::<T>()) }
    };
    List { ptr, len: n as i64, cap: n as i64, arena }
}

/// A new list in the current arena holding `n` items that `fill` writes, one at a time through
/// the pointer to each (`fill(k, at)`). Its capacity is its length.
///
/// # Safety
/// `fill` must write a valid `T` at every pointer it is given.
unsafe fn list_of<T>(n: usize, mut fill: impl FnMut(usize, *mut T)) -> List {
    let list = unwritten_list::<T>(n);
    let items = list.ptr.cast::<T>();
    for k in 0..n {
        // SAFETY: `items` has room for `n` aligned `T`s.
        fill(k, unsafe { items.add(k) });
    }
    list
}

/// A `list[u8]` of a copy of `bytes`, in the current arena, copied whole.
pub fn byte_list(bytes: &[u8]) -> List {
    let list = unwritten_list::<u8>(bytes.len());
    if !bytes.is_empty() {
        // SAFETY: the list's fresh memory has room for exactly these bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), list.ptr, bytes.len()) };
    }
    list
}

/// A `list[string]` of copies of `items`, in the current arena (the strings' bytes too).
pub fn string_list<S: AsRef<[u8]>>(items: &[S]) -> List {
    // SAFETY: each pointer `list_of` gives is room for one string header, which `make` writes.
    unsafe { list_of(items.len(), |k, at: *mut KStr| KStr::make(items[k].as_ref(), at)) }
}

/// Writes `value` as an optional's value at `out`: zeros over the whole optional (so its padding
/// is defined, as `write_none` leaves it), then tag 1 and `value`.
///
/// # Safety
/// `out` must be valid for writing a `KOpt<T>` (not necessarily aligned).
pub unsafe fn write_some<T>(out: *mut KOpt<T>, value: T) {
    std::ptr::write_bytes(out.cast::<u8>(), 0, std::mem::size_of::<KOpt<T>>());
    out.cast::<u8>().write(1);
    std::ptr::addr_of_mut!((*out).value).write_unaligned(value);
}

/// Writes `none` at `out`: tag 0, and zeros where a value would be.
///
/// # Safety
/// `out` must be valid for writing a `KOpt<T>`.
pub unsafe fn write_none<T>(out: *mut KOpt<T>) {
    std::ptr::write_bytes(out.cast::<u8>(), 0, std::mem::size_of::<KOpt<T>>());
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let xs = [1.5f32, -2.0, 3.0];
        let header = KSeq { ptr: xs.as_ptr().cast(), len: 3 };
        // SAFETY: `header` describes `xs`, which outlives the slices.
        unsafe {
            assert_eq!(elems::<f32>(&header), xs);
            assert_eq!(elems::<f32>(&KSeq { ptr: xs.as_ptr().cast(), len: 1 }), &xs[..1]);
            assert!(elems::<f32>(&KSeq { ptr: std::ptr::null(), len: 5 }).is_empty(), "null wins over a length");
            assert!(elems::<f32>(&KSeq { ptr: xs.as_ptr().cast(), len: -1 }).is_empty());
            assert!(elems::<f32>(std::ptr::null()).is_empty());
        }
    }

    #[test]
    fn unaligned_reads_and_writes() {
        // Two floats at an odd offset, as a compiled stack slot might be.
        let mut buf = [0u8; 12];
        // SAFETY: bytes 1..9 are inside `buf`.
        unsafe {
            write(buf.as_mut_ptr().add(1).cast::<[f32; 2]>(), [1.5, -2.0]);
            assert_eq!(read(buf.as_ptr().add(1).cast::<[f32; 2]>()), [1.5, -2.0]);
        }
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn optionals_are_laid_out_as_kestrel_lays_them_out() {
        use std::mem::{offset_of, size_of};
        // `list[u8]?`: the 32-byte list at offset 8; `string?`: the 16-byte string at offset 8.
        assert_eq!((size_of::<KOpt<List>>(), offset_of!(KOpt<List>, value)), (40, 8));
        assert_eq!((size_of::<KOpt<KStr>>(), offset_of!(KOpt<KStr>, value)), (24, 8));
        assert_eq!((size_of::<KOpt<u8>>(), offset_of!(KOpt<u8>, value)), (2, 1));
        assert_eq!((size_of::<KOpt<f32>>(), offset_of!(KOpt<f32>, value)), (8, 4));
    }

    #[test]
    fn some_and_none_write_the_tag_first() {
        let mut buf = [0xAAu8; 25];
        // SAFETY: bytes 1..25 hold a `KOpt<KStr>`; the string's bytes are never read.
        unsafe {
            let out = buf.as_mut_ptr().add(1).cast::<KOpt<KStr>>();
            write_some(out, KStr { ptr: 8 as *const u8, len: 3 });
            assert_eq!(buf[1], 1);
            assert_eq!(i64::from_le_bytes(buf[17..25].try_into().unwrap()), 3);
            write_none(out);
            assert!(buf[1..].iter().all(|&b| b == 0));
            assert_eq!(buf[0], 0xAA, "nothing before `out` is touched");
        }
    }

    #[test]
    fn lists_are_made_in_the_current_arena_with_no_spare_room() {
        let bytes = byte_list(&[1, 2, 3]);
        assert_eq!((bytes.len, bytes.cap, bytes.arena), (3, 3, kestrel_rt_current_arena()));
        // SAFETY: the list holds 3 bytes in the current arena.
        assert_eq!(unsafe { std::slice::from_raw_parts(bytes.ptr, 3) }, [1, 2, 3]);
        let empty = byte_list(&[]);
        assert_eq!((empty.len, empty.cap, empty.ptr.is_null()), (0, 0, true));
        let names = string_list(&["a", "", "bc"]);
        assert_eq!((names.len, names.cap), (3, 3));
        // SAFETY: the list holds 3 strings in the current arena.
        let items = unsafe { std::slice::from_raw_parts(names.ptr.cast::<KStr>(), 3) };
        // SAFETY: each string was made by `KStr::make`.
        let texts: Vec<&[u8]> = items.iter().map(|s| unsafe { KStr::bytes(s.ptr, s.len) }).collect();
        assert_eq!(texts, [&b"a"[..], b"", b"bc"]);
    }
}
