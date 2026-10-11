//! The mesh natives, called the way compiled code calls them, in headless mode. A headless mesh
//! is checked and registered like a real one, but holds no GPU parts.
//!
//! GPU ids count up for the whole test process, so the tests compare ids with each other, never
//! with fixed numbers.

use super::{headless, headless_run, kstr, rgba, v2};
use crate::abi::{KCamera, KColor, KList, KMesh, KMeshData, KSeq, KTexture, V2, V3};
use crate::gpu::{self, Object};
use crate::natives_3d::*;
use crate::natives_draw::*;
use crate::natives_mesh::*;
use crate::natives_texture::*;
use crate::natives_window::*;
use crate::pass;
use crate::state::Mode;

fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

/// A list field's header over `items`, as a struct holding a `list[T]` has it.
fn list<T>(items: &[T]) -> KList {
    KList { ptr: items.as_ptr().cast(), len: items.len() as i64, cap: items.len() as i64, arena: std::ptr::null() }
}

/// A `MeshData` whose lists are these slices.
fn data(vertices: &[V3], colors: &[KColor], uvs: &[V2], indices: &[i32]) -> KMeshData {
    KMeshData { vertices: list(vertices), colors: list(colors), uvs: list(uvs), indices: list(indices) }
}

/// The lists `make_mesh_in` takes, from slices.
fn lists<'a>(vertices: &'a [V3], colors: &'a [KColor], uvs: &'a [V2], indices: &'a [i32]) -> Lists<'a> {
    Lists { vertices, colors, uvs, indices }
}

/// An empty list field, as a zero-filled `MeshData` has: a null `ptr`.
const NO_LIST: KList = KList { ptr: std::ptr::null(), len: 0, cap: 0, arena: std::ptr::null() };
const EMPTY: KMeshData = KMeshData { vertices: NO_LIST, colors: NO_LIST, uvs: NO_LIST, indices: NO_LIST };
const NO_MESH: KMesh = KMesh { id: 0, vertex_count: 0, triangle_count: 0 };
const NO_TEXTURE: KTexture = KTexture { id: 0, width: 0, height: 0 };

/// The corners of `MeshData.quad(vec3(-1, -1, 0), vec3(1, -1, 0), vec3(1, 1, 0), vec3(-1, 1, 0), ..)`.
const SQUARE: [V3; 4] = [
    V3 { x: -1.0, y: -1.0, z: 0.0 },
    V3 { x: 1.0, y: -1.0, z: 0.0 },
    V3 { x: 1.0, y: 1.0, z: 0.0 },
    V3 { x: -1.0, y: 1.0, z: 0.0 },
];
const QUAD: [i32; 6] = [0, 1, 2, 0, 2, 3];

fn white() -> KColor {
    rgba(1.0, 1.0, 1.0, 1.0)
}

fn camera() -> KCamera {
    KCamera { pos: v3(0.0, 0.0, -3.0), target: v3(0.0, 0.0, 0.0), up: v3(0.0, 1.0, 0.0), fov: 75.0 }
}

/// What the registry holds for mesh `id`: `(parts, has_uvs, indexed)`.
fn registered(id: i64) -> Option<(usize, bool, bool)> {
    gpu::with(id, |object| match object {
        Object::Mesh { parts, has_uvs, indexed } => Some((parts.len(), *has_uvs, *indexed)),
        Object::Texture { .. } => None,
    })
    .flatten()
}

#[test]
fn headless_mesh_lines() {
    let (lines, (quad, empty, textured, pair)) = headless_run(|| {
        let title = kstr("t");
        let red = [rgba(1.0, 0.0, 0.0, 1.0); 4];
        let uvs = [v2(1.0, 1.0), v2(0.0, 1.0), v2(0.0, 0.0), v2(1.0, 0.0)];
        let (mut quad, mut empty, mut textured, mut pair) = (NO_MESH, NO_MESH, NO_MESH, NO_TEXTURE);
        let pixels = [rgba(1.0, 0.0, 0.0, 1.0), rgba(0.0, 0.0, 1.0, 1.0)];
        // SAFETY: every pointer is to a live local, as compiled code passes them.
        unsafe {
            open_with(&headless(1), 64, 64, &title);
            kestrel_draw3d_make_mesh(&mut quad, &data(&SQUARE, &red, &[], &QUAD));
            kestrel_draw3d_make_mesh(&mut empty, &EMPTY);
            kestrel_draw3d_make_mesh(&mut textured, &data(&SQUARE, &[], &uvs, &QUAD));
            kestrel_draw_make_texture(&mut pair, 2, 1, &KSeq { ptr: pixels.as_ptr().cast(), len: 2 });
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw3d_camera(&camera());
            kestrel_draw3d_mesh(&quad, &v3(1.0, 2.0, 3.0), &white());
            kestrel_draw3d_mesh(&empty, &v3(0.0, 0.0, 0.0), &rgba(0.5, 0.5, 0.5, 1.0));
            kestrel_draw3d_mesh_textured(&textured, &pair, &v3(0.0, -1.0, 0.0), &white());
        }
        (quad, empty, textured, pair)
    });
    assert_eq!((quad.vertex_count, quad.triangle_count), (4, 2));
    assert_eq!((empty.vertex_count, empty.triangle_count), (0, 0));
    assert_eq!((textured.vertex_count, textured.triangle_count), (4, 2));
    assert!(0 < quad.id && quad.id < empty.id && empty.id < textured.id, "{quad:?} {empty:?} {textured:?}");
    let (q, e, t, p) = (quad.id, empty.id, textured.id, pair.id);
    assert_eq!(
        lines,
        [
            "window.open 64 64 \"t\"".to_string(),
            "draw3d.make_mesh MeshData(vertices: 4, colors: 4, uvs: 0, indices: 6)".to_string(),
            "draw3d.make_mesh MeshData(vertices: 0, colors: 0, uvs: 0, indices: 0)".to_string(),
            "draw3d.make_mesh MeshData(vertices: 4, colors: 0, uvs: 4, indices: 6)".to_string(),
            "draw.make_texture 2 1 2".to_string(),
            "draw3d.camera Camera(pos: vec3(x: 0, y: 0, z: -3), target: vec3(x: 0, y: 0, z: 0), up: vec3(x: 0, y: 1, z: 0), fov: 75)"
                .to_string(),
            format!("draw3d.mesh Mesh(id: {q}, vertex_count: 4, triangle_count: 2) vec3(x: 1, y: 2, z: 3) Color(r: 1, g: 1, b: 1, a: 1)"),
            format!(
                "draw3d.mesh Mesh(id: {e}, vertex_count: 0, triangle_count: 0) vec3(x: 0, y: 0, z: 0) Color(r: 0.5, g: 0.5, b: 0.5, a: 1)"
            ),
            format!(
                "draw3d.mesh_textured Mesh(id: {t}, vertex_count: 4, triangle_count: 2) Texture(id: {p}, width: 2, height: 1) \
                 vec3(x: 0, y: -1, z: 0) Color(r: 1, g: 1, b: 1, a: 1)"
            ),
        ]
    );
}

#[test]
fn a_headless_mesh_keeps_its_counts_and_flags_but_no_parts() {
    headless_run(|| {
        let title = kstr("t");
        let loose = [v3(0.0, 0.0, 0.0), v3(1.0, 0.0, 0.0), v3(0.0, 1.0, 0.0)];
        let uvs = [v2(0.0, 0.0); 4];
        // SAFETY: every pointer is to a live local.
        unsafe { open_with(&headless(1), 8, 8, &title) };
        let indexed = make_mesh_in(Mode::Headless, &lists(&SQUARE, &[], &uvs, &QUAD)).unwrap();
        assert_eq!(registered(indexed.id), Some((0, true, true)));
        assert_eq!((indexed.vertex_count, indexed.triangle_count), (4, 2));
        let unindexed = make_mesh_in(Mode::Headless, &lists(&loose, &[], &[], &[])).unwrap();
        assert_eq!(registered(unindexed.id), Some((0, false, false)));
        assert_eq!((unindexed.vertex_count, unindexed.triangle_count), (3, 1));
        // An empty mesh has a uv for each of its (no) vertices, so mesh_textured draws it: nothing.
        let empty = make_mesh_in(Mode::Headless, &lists(&[], &[], &[], &[])).unwrap();
        assert_eq!(registered(empty.id), Some((0, true, false)));
        assert_eq!((empty.vertex_count, empty.triangle_count), (0, 0));
    });
}

#[test]
fn make_mesh_checks_the_data_and_registers_nothing_when_it_is_wrong() {
    headless_run(|| {
        let title = kstr("t");
        // SAFETY: every pointer is to a live local.
        unsafe { open_with(&headless(1), 8, 8, &title) };
        let before = gpu::live();
        let two = [white(); 2];
        let wrong = make_mesh_in(Mode::Headless, &lists(&SQUARE, &two, &[], &QUAD));
        assert_eq!(wrong, Err("draw3d.make_mesh needs one color per vertex or none: 4 vertices, 2 colors".to_string()));
        let wrong = make_mesh_in(Mode::Headless, &lists(&SQUARE, &[], &[], &[0, 1, 4]));
        assert_eq!(wrong, Err("draw3d.make_mesh got index 4, but there are only 4 vertices".to_string()));
        let wrong = make_mesh_in(Mode::Headless, &lists(&SQUARE, &[], &[], &[]));
        assert_eq!(wrong, Err("draw3d.make_mesh needs a multiple of 3 vertices when there are no indices, not 4".to_string()));
        assert_eq!(gpu::live(), before);
    });
}

#[test]
fn drawing_checks_the_camera_then_the_mesh_then_its_uvs_then_the_texture() {
    headless_run(|| {
        let title = kstr("t");
        let uvs = [v2(0.0, 0.0); 4];
        let mut tex = NO_TEXTURE;
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&headless(1), 8, 8, &title);
            kestrel_draw_make_texture(&mut tex, 1, 1, &KSeq { ptr: [white()].as_ptr().cast(), len: 1 });
        }
        let plain = make_mesh_in(Mode::Headless, &lists(&SQUARE, &[], &[], &QUAD)).unwrap();
        let textured = make_mesh_in(Mode::Headless, &lists(&SQUARE, &[], &uvs, &QUAD)).unwrap();
        let empty = make_mesh_in(Mode::Headless, &lists(&[], &[], &[], &[])).unwrap();

        assert_eq!(prepare("draw3d.mesh", NO_MESH, None).err().unwrap(), "draw3d.mesh needs a camera; call draw3d.camera first");
        // SAFETY: a pointer to a live local.
        unsafe { kestrel_draw3d_camera(&camera()) };
        assert!(prepare("draw3d.mesh", plain, None).is_ok());
        assert!(prepare("draw3d.mesh", empty, None).is_ok());
        let empty_mesh = |name: &str| format!("{name} got an empty mesh; make one with draw3d.make_mesh");
        assert_eq!(prepare("draw3d.mesh", NO_MESH, None).err().unwrap(), empty_mesh("draw3d.mesh"));
        let textured_name = "draw3d.mesh_textured";
        assert_eq!(prepare(textured_name, NO_MESH, Some(NO_TEXTURE)).err().unwrap(), empty_mesh(textured_name));
        let no_uvs = "draw3d.mesh_textured needs a mesh made with uvs";
        assert_eq!(prepare(textured_name, plain, Some(NO_TEXTURE)).err().unwrap(), no_uvs);
        assert_eq!(prepare(textured_name, plain, Some(tex)).err().unwrap(), no_uvs);
        let empty_texture = "draw3d.mesh_textured got an empty texture; make one with draw.load_texture or draw.make_texture";
        assert_eq!(prepare(textured_name, textured, Some(NO_TEXTURE)).err().unwrap(), empty_texture);
        assert!(prepare(textured_name, textured, Some(tex)).is_ok());
        assert!(prepare(textured_name, empty, Some(tex)).is_ok(), "an empty mesh draws nothing, textured or not");
        // A texture's id is not a mesh.
        let not_a_mesh = KMesh { id: tex.id, vertex_count: 4, triangle_count: 2 };
        assert_eq!(prepare("draw3d.mesh", not_a_mesh, None).err().unwrap(), empty_mesh("draw3d.mesh"));
    });
}

#[test]
fn make_mesh_leaves_the_pass_alone_and_the_mesh_natives_draw_in_3d() {
    headless_run(|| {
        let title = kstr("t");
        let mut m = NO_MESH;
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&headless(1), 8, 8, &title);
            assert_eq!(kestrel_window_frame(), 1);
            // No camera yet: making a mesh draws nothing, so it needs none.
            kestrel_draw3d_make_mesh(&mut m, &data(&SQUARE, &[], &[], &QUAD));
            assert!(!pass::in_3d());
            kestrel_draw3d_camera(&camera());
            kestrel_draw3d_make_mesh(&mut m, &data(&SQUARE, &[], &[], &QUAD));
            assert!(pass::in_3d(), "and it doesn't end a 3D pass");
            kestrel_draw_rect(&v2(0.0, 0.0), &v2(1.0, 1.0), &white());
            assert!(!pass::in_3d());
            kestrel_draw3d_mesh(&m, &v3(0.0, 0.0, 0.0), &white());
            assert!(pass::in_3d(), "draw3d.mesh begins 3D again with the camera");
        }
    });
}
