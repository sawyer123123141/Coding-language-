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

/// `draw3d.Camera`: three `vec3`s and a `float`, which the window crate reads as `KCamera`.
#[repr(C)]
struct Cam {
    pos: V3,
    target: V3,
    up: V3,
    fov: f32,
}

extern "C" fn k_cam_parts(c: *const Cam) -> f32 {
    // SAFETY: a pointer to a live `Cam`, laid out as above. Each field is read at its own offset,
    // so a layout that differs shows in the sum.
    unsafe { (*c).pos.x + 10.0 * (*c).target.y + 100.0 * (*c).up.z + 1000.0 * (*c).fov }
}
extern "C" fn k_cam_fovs(cs: *const KSeq) -> f32 {
    // SAFETY: a pointer to a live array header of `Cam`s. The elements are read 40 bytes apart.
    unsafe { elems::<Cam>((*cs).ptr, (*cs).len).iter().fold(0.0, |total, c| total + c.fov) }
}
extern "C" fn k_gap(a: *const V3, b: *const V3) -> f32 {
    // SAFETY: two pointers to live `vec3`s.
    unsafe { (*b).z - (*a).z }
}

#[test]
fn a_struct_of_three_vec3s_and_a_float_is_forty_bytes_and_vec3s_travel_by_pointer() {
    let src = "struct Cam {
    pos: vec3
    target: vec3
    up: vec3 = vec3(0, 1, 0)
    fov: float = 75
}
native cam_parts(c: Cam) -> float
native cam_fovs(cs: [Cam]) -> float
native gap(a: vec3, b: vec3) -> float
main() {
    let c = Cam(pos: vec3(1, 2, 3), target: vec3(4, 5, 6), up: vec3(0, 1, 2))
    let cams = [Cam(pos: vec3(0, 0, 0), target: vec3(0, 0, 1), fov: 10), Cam(pos: vec3(0, 0, 0), target: vec3(0, 0, 1), fov: 20), Cam(pos: vec3(0, 0, 0), target: vec3(0, 0, 1))]
    print(cam_parts(c), cam_fovs(cams), gap(vec3(0, 0, 1), vec3(0, 0, 6)))
}
";
    let extra = [
        ("kestrel_cam_parts", k_cam_parts as *const u8),
        ("kestrel_cam_fovs", k_cam_fovs as *const u8),
        ("kestrel_gap", k_gap as *const u8),
    ];
    let p = jit_natives(src, &extra).expect("codegen");
    // pos.x 1 + 10 * target.y 5 + 100 * up.z 2 + 1000 * the default fov 75; then 10 + 20 + 75; then 6 - 1.
    assert_eq!(run_captured(&p), "75251 105 5\n");
    assert_eq!(std::mem::size_of::<Cam>(), 40);
    assert_eq!(std::mem::offset_of!(Cam, fov), 36);
}

/// `draw3d.MeshData`: four `list[T]` fields, each an inline 32-byte header, so the struct is 128
/// bytes and a native gets a pointer to it. The window crate reads it as `KMeshData`.
#[repr(C)]
struct MeshData {
    vertices: KList,
    colors: KList,
    uvs: KList,
    indices: KList,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Rgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct V2 {
    x: f32,
    y: f32,
}

extern "C" fn k_mesh_lens(d: *const MeshData) -> i64 {
    // SAFETY: a pointer to a live `MeshData`, laid out as above.
    unsafe { (*d).vertices.len * 1_000_000 + (*d).colors.len * 10_000 + (*d).uvs.len * 100 + (*d).indices.len }
}
extern "C" fn k_mesh_picks(d: *const MeshData) -> f32 {
    // SAFETY: a pointer to a live `MeshData` whose lists hold `vec3`, `Color`, `vec2` and `int`
    // elements, read through each header's `ptr` and `len` only. An empty list may have a null `ptr`.
    unsafe {
        let d = &*d;
        let vertices = elems::<V3>(d.vertices.ptr, d.vertices.len);
        let colors = elems::<Rgba>(d.colors.ptr, d.colors.len);
        let uvs = elems::<V2>(d.uvs.ptr, d.uvs.len);
        let indices = elems::<i32>(d.indices.ptr, d.indices.len);
        let last = |n: usize| n.checked_sub(1);
        let z = last(vertices.len()).map_or(0.0, |i| vertices[i].z);
        let color = colors.first().map_or(0.0, |c| c.r + 2.0 * c.g + 4.0 * c.b + 8.0 * c.a);
        let uv = last(uvs.len()).map_or(0.0, |i| uvs[i].x + 2.0 * uvs[i].y);
        let index = last(indices.len()).map_or(0, |i| indices[i]);
        z + 10.0 * color + 1000.0 * uv + 100_000.0 * index as f32
    }
}

#[test]
fn a_struct_of_four_lists_reaches_a_native_as_four_32_byte_headers() {
    let src = "struct Color {
    r: float
    g: float
    b: float
    a: float = 1
}
struct MeshData {
    vertices: list[vec3] = []
    colors: list[Color] = []
    uvs: list[vec2] = []
    indices: list[int] = []
}
native mesh_lens(d: MeshData) -> i64
native mesh_picks(d: MeshData) -> float
main() {
    var d = MeshData()
    print(mesh_lens(d), mesh_picks(d))
    for i from 0 to 5 {
        d.vertices.add(vec3(0, 0, i))
    }
    d.colors.add(Color(1, 0, 1))
    d.colors.add(Color(0, 0, 0, 0))
    d.uvs.add(vec2(0, 0))
    d.uvs.add(vec2(0.5, 0.25))
    d.uvs.add(vec2(1, 2))
    for i from 0 to 4 {
        d.indices.add(i * 2)
    }
    print(mesh_lens(d), mesh_picks(d))
}
";
    let extra = [("kestrel_mesh_lens", k_mesh_lens as *const u8), ("kestrel_mesh_picks", k_mesh_picks as *const u8)];
    let p = jit_natives(src, &extra).expect("codegen");
    // Lengths 5, 2, 3, 4. The last vertex's z is 4; the first colour is 1 + 4 + 8 = 13; the last
    // uv is 1 + 2 * 2 = 5; the last index is 6: 4 + 130 + 5000 + 600000.
    assert_eq!(run_captured(&p), "0 0\n5020304 605134\n");
    assert_eq!(std::mem::size_of::<MeshData>(), 128);
    assert_eq!(std::mem::offset_of!(MeshData, indices), 96);
}
