use super::*;
use crate::arena::{kestrel_rt_arena_free, kestrel_rt_arena_new, kestrel_rt_arena_pop, kestrel_rt_arena_push};

fn s(text: &str) -> RefValue {
    RefValue::Str(text.to_string())
}

#[test]
fn the_header_is_the_magic_the_version_and_the_fingerprint() {
    let bytes = encode(&RefValue::Bool(true), 0x0102_0304_0506_0708);
    assert_eq!(&bytes[..4], b"KESB");
    assert_eq!(bytes[4], 1);
    assert_eq!(&bytes[5..13], &[8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(&bytes[13..], &[1]);
    assert_eq!(bytes.len(), HEADER + 1);
}

#[test]
fn values_are_little_endian_without_padding() {
    let cases: Vec<(RefValue, Vec<u8>)> = vec![
        (RefValue::Bool(false), vec![0]),
        (RefValue::U8(200), vec![200]),
        (RefValue::Int(-2), vec![254, 255, 255, 255]),
        (RefValue::I64(258), vec![2, 1, 0, 0, 0, 0, 0, 0]),
        (RefValue::Float(1.5), vec![0, 0, 192, 63]),
        (RefValue::F64(0.5), vec![0, 0, 0, 0, 0, 0, 224, 63]),
        (s("h\u{e9}"), vec![3, 0, 0, 0, 104, 195, 169]),
        (RefValue::List(vec![RefValue::U8(1), RefValue::U8(2)]), vec![2, 0, 0, 0, 1, 2]),
        (RefValue::Map(vec![(s("a"), RefValue::U8(9))]), vec![1, 0, 0, 0, 1, 0, 0, 0, 97, 9]),
        (RefValue::Opt(None), vec![0]),
        (RefValue::Opt(Some(Box::new(RefValue::U8(5)))), vec![1, 5]),
        (RefValue::Enum(2, vec![RefValue::U8(3), RefValue::U8(4)]), vec![2, 0, 0, 0, 3, 4]),
        (RefValue::Struct(vec![RefValue::U8(1), RefValue::Int(1)]), vec![1, 1, 0, 0, 0]),
    ];
    for (value, want) in cases {
        let mut out = Vec::new();
        write_value(&value, &mut out);
        assert_eq!(out, want, "{value:?}");
    }
}

#[test]
fn a_nan_keeps_its_payload() {
    let mut out = Vec::new();
    write_value(&RefValue::Float(f32::from_bits(0x7FC0_0001)), &mut out);
    assert_eq!(out, vec![1, 0, 192, 127]);
}

#[test]
fn the_fingerprint_is_the_checksum_of_the_description() {
    let desc = "struct Rng{seed:i64}";
    assert_eq!(fingerprint(desc), finish(mix_bytes(START, desc.as_bytes())) as i64);
    assert_ne!(fingerprint("struct P{a:int,b:int}"), fingerprint("struct P{b:int,a:int}"));
}

#[test]
fn reserving_gives_exactly_the_room_asked_for_and_keeps_the_items() {
    // SAFETY: the arena is pushed while the list is used and freed after.
    unsafe {
        let arena = kestrel_rt_arena_new();
        kestrel_rt_arena_push(arena);
        let mut l = List { ptr: std::ptr::null_mut(), len: 0, cap: 0, arena };
        kestrel_rt_list_reserve(&mut l, 3, 4, 4);
        assert_eq!((l.len, l.cap), (0, 3));
        l.ptr.cast::<i32>().write(7);
        l.len = 1;
        kestrel_rt_list_reserve(&mut l, 13, 4, 4);
        assert_eq!((l.len, l.cap, l.ptr.cast::<i32>().read()), (1, 13, 7));
        // Enough room already: nothing changes.
        let before = l.ptr;
        kestrel_rt_list_reserve(&mut l, 5, 4, 4);
        assert_eq!((l.cap, l.ptr), (13, before));
        kestrel_rt_arena_pop();
        kestrel_rt_arena_free(arena);
    }
}

#[test]
fn a_string_is_written_as_its_length_then_its_bytes() {
    let mut buf = [0xAAu8; 9];
    let text = "h\u{e9}";
    // SAFETY: `buf` has room for the length and the three bytes.
    unsafe { kestrel_rt_bytes_write_str(buf.as_mut_ptr().add(1), text.as_ptr(), text.len() as i64) };
    assert_eq!(buf, [0xAA, 3, 0, 0, 0, 104, 195, 169, 0xAA]);
    // A null pointer is the empty string.
    // SAFETY: as above.
    unsafe { kestrel_rt_bytes_write_str(buf.as_mut_ptr(), std::ptr::null(), 0) };
    assert_eq!(&buf[..5], &[0, 0, 0, 0, 0]);
}
