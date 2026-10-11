//! `native` functions that take arrays, lists and struct fields holding them, and return
//! structs. (The rest of the native tests are in `natives.rs`.)

mod common;

use common::{jit_natives, run_captured};

/// An array or list argument: a pointer to a header that starts `{ptr, len}`.
#[repr(C)]
struct KSeq {
    ptr: *const u8,
    len: i64,
}
/// A `list[T]` stored in a struct: the whole 32-byte header.
#[repr(C)]
struct KList {
    ptr: *const u8,
    len: i64,
    cap: i64,
    arena: *const u8,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct V3 {
    x: f32,
    y: f32,
    z: f32,
}
#[repr(C)]
struct Data {
    xs: KList,
    ps: KSeq,
}
#[repr(C)]
struct Pair {
    n: i32,
    at: V3,
    ok: u8,
}

/// The elements of an array or list header; empty when `ptr` is null.
unsafe fn elems<'a, T>(ptr: *const u8, len: i64) -> &'a [T] {
    if ptr.is_null() || len <= 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr.cast::<T>(), len as usize)
    }
}
extern "C" fn k_sum_ys(vs: *const KSeq) -> f32 {
    // SAFETY: Kestrel passes a pointer to a live array or list header of `vec3`s.
    // (A fold from +0.0: `Iterator::sum` starts from -0.0, which prints as `-0` for an empty array.)
    unsafe { elems::<V3>((*vs).ptr, (*vs).len).iter().fold(0.0, |total, v| total + v.y) }
}
extern "C" fn k_count(d: *const Data) -> i32 {
    // SAFETY: a pointer to a live `Data`, laid out as above.
    unsafe { ((*d).xs.len * 100 + (*d).ps.len) as i32 }
}
extern "C" fn k_pair(out: *mut Pair, n: i32) {
    // SAFETY: the result pointer comes first and has room for a `Pair`.
    unsafe { out.write_unaligned(Pair { n, at: V3 { x: 1.0, y: 2.0, z: 3.0 }, ok: 1 }) }
}

#[test]
fn natives_read_arrays_lists_and_struct_fields_and_return_structs() {
    let src = "struct Data {
    xs: list[int] = []
    ps: [vec2] = []
}
struct Pair {
    n: int
    at: vec3
    ok: bool
}
native sum_ys(vs: [vec3]) -> float
native count(d: Data) -> int
native pair(n: int) -> Pair
main() {
    var grown = list[vec3]()
    for i from 0 to 100 {
        grown.add(vec3(0, i, 0))
    }
    let fixed = [vec3(0, 1, 0), vec3(0, 2, 0)]
    let empty: [vec3] = []
    print(sum_ys(fixed), sum_ys(grown), sum_ys(empty), sum_ys([vec3(0, 5, 0)]))
    var d = Data()
    d.xs.add(1)
    d.xs.add(2)
    print(count(d), count(Data(ps: [vec2(1, 1)])))
    print(pair(7))
}
";
    let extra = [
        ("kestrel_sum_ys", k_sum_ys as *const u8),
        ("kestrel_count", k_count as *const u8),
        ("kestrel_pair", k_pair as *const u8),
    ];
    let p = jit_natives(src, &extra).expect("codegen");
    assert_eq!(run_captured(&p), "3 4950 0 5\n200 1\nPair(n: 7, at: vec3(x: 1, y: 2, z: 3), ok: true)\n");
}
