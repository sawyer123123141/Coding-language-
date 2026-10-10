//! `native` functions: bodyless declarations that import Rust `extern "C"`
//! functions, called with Kestrel's own calling convention.

use std::sync::atomic::{AtomicI32, Ordering};

use kestrel_codegen::{emit_object, Options};
use kestrel_syntax::{parse, LineIndex};

mod common;

use common::{jit_natives, jit_natives_with, run_captured};

/// A string argument: a pointer to this 16-byte header.
#[repr(C)]
struct KStr {
    ptr: *const u8,
    len: i64,
}
#[repr(C)]
struct V2 {
    x: f32,
    y: f32,
}
#[repr(C)]
struct Rgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

extern "C" fn k_twice(x: i32) -> i32 {
    x * 2
}
extern "C" fn k_flip(b: u8) -> u8 {
    (b == 0) as u8
}
extern "C" fn k_len(s: *const KStr) -> i32 {
    // SAFETY: Kestrel passes a pointer to a live string header.
    let s = unsafe { &*s };
    if s.ptr.is_null() {
        0
    } else {
        s.len as i32
    }
}
extern "C" fn k_sum(v: *const V2) -> f32 {
    // SAFETY: a pointer to a live `vec2`.
    let v = unsafe { &*v };
    v.x + v.y
}
extern "C" fn k_swap(out: *mut V2, v: *const V2) {
    // SAFETY: the result pointer comes first; both point at live `vec2`s.
    unsafe { *out = V2 { x: (*v).y, y: (*v).x } }
}
extern "C" fn k_alpha(c: *const Rgba) -> f32 {
    // SAFETY: a pointer to a live struct of four floats.
    unsafe { (*c).a }
}
extern "C" fn k_tag(e: *const u8) -> i32 {
    // SAFETY: an enum is passed by pointer, its tag byte first.
    unsafe { *e as i32 }
}

const SYMBOLS: &[(&str, *const u8)] = &[
    ("kestrel_twice", k_twice as *const u8),
    ("kestrel_flip", k_flip as *const u8),
    ("kestrel_len", k_len as *const u8),
    ("kestrel_sum", k_sum as *const u8),
    ("kestrel_swap", k_swap as *const u8),
    ("kestrel_alpha", k_alpha as *const u8),
    ("kestrel_tag", k_tag as *const u8),
];

const PROGRAM: &str = "enum Dir {
    North
    East
    South
}
struct Rgba {
    r: float
    g: float
    b: float
    a: float = 1
}
native twice(x: int) -> int
native flip(b: bool) -> bool
native len(s: string) -> int
native sum(v: vec2) -> float
native swap(v: vec2) -> vec2
native alpha(c: Rgba) -> float
native tag(d: Dir) -> int
main() {
    let n = 3
    print(twice(21), flip(true), len(\"abc{n}\"), len(\"\"), sum(vec2(1.5, 2)), swap(vec2(1, 2)), alpha(Rgba(1, 0, 0)), tag(Dir.South))
}
";

#[test]
fn natives_use_kestrels_calling_convention() {
    let p = jit_natives(PROGRAM, SYMBOLS).expect("codegen");
    assert_eq!(run_captured(&p), "42 false 4 0 3.5 vec2(x: 2, y: 1) 1 2\n");
}

static BUMPS: AtomicI32 = AtomicI32::new(0);

extern "C" fn k_bump() {
    BUMPS.fetch_add(1, Ordering::SeqCst);
}
extern "C" fn k_mix(a: i64, b: f64, c: f32, d: u8, e: u8) -> f64 {
    a as f64 + b + c as f64 + d as f64 * 100.0 + e as f64 * 1000.0
}
extern "C" fn k_origin(out: *mut f32) {
    // SAFETY: a `vec3` result is three floats written through the first pointer.
    unsafe { out.copy_from_nonoverlapping([7.0, 8.0, 9.0].as_ptr(), 3) }
}
extern "C" fn k_big(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) -> i32 {
    a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f + 7 * g + 8 * h
}

#[test]
fn a_native_with_no_result_is_called_not_inlined_away() {
    let src = "native bump()\nmain() {\n bump()\n bump()\n bump()\n}\n";
    let p = jit_natives(src, &[("kestrel_bump", k_bump as *const u8)]).expect("codegen");
    run_captured(&p);
    assert_eq!(BUMPS.load(Ordering::SeqCst), 3);
}

#[test]
fn scalars_of_every_width_and_vec3_results() {
    let src = "pure native mix(a: i64, b: f64, c: float, d: u8, e: bool) -> f64
native origin() -> vec3
native big(a: int, b: int, c: int, d: int, e: int, f: int, g: int, h: int) -> int
main() {
    let x: u8 = 7
    print(mix(5, 0.5, 0.25, x, true))
    print(origin())
    print(big(1, 1, 1, 1, 1, 1, 1, 1))
}
";
    let extra = [
        ("kestrel_mix", k_mix as *const u8),
        ("kestrel_origin", k_origin as *const u8),
        ("kestrel_big", k_big as *const u8),
    ];
    let p = jit_natives(src, &extra).expect("codegen");
    assert_eq!(run_captured(&p), "1705.75\nvec3(x: 7, y: 8, z: 9)\n36\n");
}

#[test]
fn a_native_in_the_entry_file_is_never_exported() {
    let p = jit_natives("pub native twice(x: int) -> int\npub keep() -> int { 1 }\nmain() {}\n", &[("kestrel_twice", k_twice as *const u8)])
        .expect("codegen");
    let names: Vec<String> = p.exports().into_iter().map(|(info, _)| info.name).collect();
    assert_eq!(names, ["keep"]);
}

#[test]
fn a_missing_implementation_is_an_error_not_a_crash() {
    let e = jit_natives("native twice(x: int) -> int\nmain() {\n print(twice(1))\n}\n", &[]).err().expect("must fail");
    assert!(e.contains("twice") && e.contains("kestrel_twice"), "{e}");
}

#[test]
fn objects_import_natives_by_their_symbol() {
    let src = "native twice(x: int) -> int\nmain() {\n print(twice(21))\n}\n";
    let (p, _) = parse(src);
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty());
    let mut m = kestrel_kir::lower(&p, &t.unwrap(), &LineIndex::new(src));
    kestrel_kir::inline::inline_small(&mut m);
    let obj = emit_object(&m, Options::default()).expect("object");
    let has = |needle: &[u8]| obj.windows(needle.len()).any(|w| w == needle);
    assert!(has(b"kestrel_twice"), "the object imports the native's symbol");
    assert!(!has(b"k_twice"), "nothing is defined for the native");
}

/// Where allocation goes right now, as a number: tells the program's own arena from a scratch one.
extern "C" fn k_where_am_i() -> i64 {
    kestrel_runtime::arena::kestrel_rt_current_arena() as i64
}
extern "C" fn k_where_in(_s: *const KStr) -> i64 {
    k_where_am_i()
}
extern "C" fn k_where_pair(_s: *const KStr, _inner: i64) -> i64 {
    k_where_am_i()
}
/// A `vec2` result and a text argument together: (the text's length, 2).
extern "C" fn k_measure(out: *mut V2, s: *const KStr) {
    // SAFETY: the result pointer comes first; `s` points at a live string header.
    unsafe { *out = V2 { x: (*s).len as f32, y: 2.0 } }
}

#[test]
fn an_interpolated_argument_is_built_in_an_arena_that_is_gone_after_the_call() {
    let src = "native where_am_i() -> i64
native where_in(s: string) -> i64
native where_pair(s: string, inner: i64) -> i64
native measure(s: string) -> vec2
early(c: bool) -> int {
    let a = where_in(\"a{if c { return 1 } else { 2 }}\")
    return 0
}
main() {
    let outer = where_am_i()
    let n = 42
    print(where_in(\"n {n}\") != outer, where_in(\"plain\") == outer)
    // One scratch arena inside another: the inner call pops back to the outer one.
    print(where_pair(\"a{n}\", where_in(\"b{n}\")) != outer, where_am_i() == outer)
    print(where_am_i() == outer, measure(\"n {n}\"))
    print(early(true), early(false), where_am_i() == outer)
    var i = 0
    while i < 4 {
        i += 1
        let a = where_in(\"c{if i == 2 { continue } else { i }}\")
        let b = where_in(\"d{if i == 3 { break } else { i }}\")
    }
    print(i, where_am_i() == outer)
}
";
    let extra = [
        ("kestrel_where_am_i", k_where_am_i as *const u8),
        ("kestrel_where_in", k_where_in as *const u8),
        ("kestrel_where_pair", k_where_pair as *const u8),
        ("kestrel_measure", k_measure as *const u8),
    ];
    let p = jit_natives(src, &extra).expect("codegen");
    // `i` stops at 3: `break` leaves the loop on the third round, and every early exit
    // has popped its arena, so the current arena is the program's again.
    assert_eq!(run_captured(&p), "true true\ntrue true\ntrue vec2(x: 4, y: 2)\n1 0 true\n3 true\n");
}

/// Reads its argument as a whole 32-bit register, which is what a C function declared with an
/// `unsigned`/`int` parameter does. Declared `u8` in Kestrel, so only the zero-extension the
/// caller adds keeps the register's upper bits from showing.
extern "C" fn k_wide(b: u32) -> i32 {
    b as i32
}

#[test]
fn small_arguments_are_zero_extended() {
    // `u8(xs[i])` with checks off truncates, and nothing clears the register's upper bits (the
    // values come from memory, so they can't be folded into constants): 74565 is 0x12345.
    let src = "native wide(b: u8) -> int
main() {
    let xs = [74565, 74566, 255, 256]
    var i = 0
    while i < xs.len {
        print(wide(u8(xs[i])))
        i += 1
    }
}
";
    let release = Options { checked_arith: false };
    let p = jit_natives_with(src, &[("kestrel_wide", k_wide as *const u8)], release).expect("codegen");
    assert_eq!(run_captured(&p), "69\n70\n255\n0\n");
}

#[test]
fn a_bool_argument_is_zero_extended_too() {
    // `a != b` is a byte (0 or 1) that nothing else has widened.
    let src = "native wide(b: bool) -> int
main() {
    let xs = [74565, 74566]
    var i = 0
    while i < xs.len {
        print(wide(xs[i] != 74566), wide(xs[i] == 74566))
        i += 1
    }
}
";
    let p = jit_natives_with(src, &[("kestrel_wide", k_wide as *const u8)], Options { checked_arith: false }).expect("codegen");
    assert_eq!(run_captured(&p), "1 0\n0 1\n");
}

/// Kestrel's `Mix`, as C would lay it out: declaration order, natural alignment.
#[repr(C)]
struct Mix {
    a: u8,
    b: u8,
    c: i64,
    s: KStr,
    v: [f32; 3],
}

/// A different decimal place for each field, so a wrong offset shows in the digits.
extern "C" fn k_checksum(m: *const Mix) -> i64 {
    // SAFETY: Kestrel passes a pointer to a live `Mix`, whose string points at `len` bytes.
    let m = unsafe { &*m };
    let bytes = unsafe { std::slice::from_raw_parts(m.s.ptr, m.s.len as usize) };
    let byte_sum: i64 = bytes.iter().map(|&b| b as i64).sum();
    let v = (m.v[0] + 10.0 * m.v[1] + 100.0 * m.v[2]) as i64;
    m.a as i64 + 1_000 * m.b as i64 + 10_000 * m.s.len + 1_000_000 * byte_sum + 10_000_000_000 * v + m.c
}

#[test]
fn a_struct_of_mixed_widths_has_the_layout_c_expects() {
    let src = "struct Mix {
    a: u8
    b: bool
    c: i64
    s: string
    v: vec3
}
native checksum(m: Mix) -> i64
main() {
    let big: i64 = 7000000000000000
    let m = Mix(200, true, big, \"kes\", vec3(1, 2, 3))
    print(checksum(m))
}
";
    let p = jit_natives(src, &[("kestrel_checksum", k_checksum as *const u8)]).expect("codegen");
    // a 200, b 1,000, the length 3 * 10,000, the bytes 323 * 1,000,000, v 321 * 10,000,000,000, c 7e15.
    assert_eq!(run_captured(&p), "7003210323031200\n");
}

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
