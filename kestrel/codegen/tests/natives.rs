//! `native` functions: bodyless declarations that import Rust `extern "C"`
//! functions, called with Kestrel's own calling convention.

use std::sync::atomic::{AtomicI32, Ordering};

use kestrel_codegen::{emit_object, Options};
use kestrel_syntax::{parse, LineIndex};

mod common;

use common::{jit_natives, run_captured};

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
