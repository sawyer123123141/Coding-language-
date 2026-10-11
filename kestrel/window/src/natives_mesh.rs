//! The `draw3d` module's mesh natives (`driver/std/draw3d.kes`): `make_mesh` turns a `MeshData`
//! into a mesh on the GPU, and `mesh` and `mesh_textured` draw one.
//!
//! **Memory.** A `MeshData` reaches `make_mesh` as a pointer to its four list headers
//! (`KMeshData`). The elements are the program's memory, borrowed for the call. raylib's
//! `UploadMesh` copies each CPU array a raylib `Mesh` names into a GPU buffer (OpenGL copies the
//! data before `glBufferData` returns), and `UnloadMesh` `free`s every CPU array a `Mesh` still
//! names. So, for each part:
//!
//! - its arrays point at the list memory (positions and uvs, when the mesh is one part as given),
//!   or at Rust `Vec`s made for the call (colours as bytes, indices as `u16`s, and every array of a
//!   split mesh), all alive until `UploadMesh` returns;
//! - right after `UploadMesh`, `gpu::without_cpu_pointers` nulls them all, so the part that is
//!   stored names only what raylib allocated itself (its vertex array and buffer ids), which
//!   `UnloadMesh` frees when the program's `Mesh` goes away (`gpu.rs`).
//!
//! An empty `MeshData` makes a mesh with no parts, and a headless window keeps only the counts: no
//! raylib call either way.
//!
//! **Drawing.** Kestrel's fronts are clockwise and OpenGL's are counter-clockwise, so a mesh is
//! drawn culling OpenGL's front faces. Outside a mesh draw, culling stays as raylib has it (on,
//! back faces culled), which raylib's cubes and spheres need. The 3D batch they are in is drawn
//! first, so they are culled their way and stay in drawing order. Each draw flushes once and sets
//! the cull face twice, whatever the number of parts, and allocates nothing.

use std::ffi::c_int;

use kestrel_runtime::panic::fail_at_call;

use crate::abi::{read, write, KColor, KMesh, KMeshData, KTexture, V2, V3};
use crate::gpu::{self, Object};
use crate::headless::{self, Arg};
use crate::material;
use crate::mesh_data::{self, MOST_PART_VERTICES};
use crate::natives_texture::lookup;
use crate::pass;
use crate::raylib as rl;
use crate::state::{self, Mode};

/// Copies `data` to the GPU (a headless window only checks and counts it) and writes the `Mesh`.
///
/// # Safety
/// `out` must be valid for writing a `Mesh`, `data` must point to a live `MeshData`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw3d_make_mesh(out: *mut KMesh, data: *const KMeshData) {
    // SAFETY: `data` points to a live `MeshData` (this function's contract), whose lists hold
    // `vec3`, `Color`, `vec2` and `int` elements, alive for the call.
    let lists = unsafe { Lists::of(&read(data)) };
    let mode = state::drawing("draw3d.make_mesh");
    let mesh = make_mesh_in(mode, &lists).unwrap_or_else(|msg| fail_at_call(&msg));
    if mode == Mode::Headless {
        headless::line("draw3d.make_mesh", &[lists.arg()]);
    }
    write(out, mesh);
}

/// Draws `m` moved by `pos`, its colours multiplied by `tint`.
///
/// # Safety
/// `m` must point to a live `Mesh`, `pos` to a live `vec3`, `tint` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw3d_mesh(m: *const KMesh, pos: *const V3, tint: *const KColor) {
    let (m, pos, tint) = (read(m), read(pos), read(tint));
    draw("draw3d.mesh", &[Arg::Mesh(m), Arg::Vec3(pos), Arg::Color(tint)], m, None, pos, tint);
}

/// `kestrel_draw3d_mesh`, with the colours multiplied by `tex` sampled at the mesh's uvs too.
///
/// # Safety
/// `m` must point to a live `Mesh`, `tex` to a live `Texture`, `pos` to a live `vec3`, `tint` to a live `Color`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_draw3d_mesh_textured(m: *const KMesh, tex: *const KTexture, pos: *const V3, tint: *const KColor) {
    let (m, tex, pos, tint) = (read(m), read(tex), read(pos), read(tint));
    let args = [Arg::Mesh(m), Arg::Texture(tex), Arg::Vec3(pos), Arg::Color(tint)];
    draw("draw3d.mesh_textured", &args, m, Some(tex), pos, tint);
}

/// A `MeshData`'s lists, as slices of the program's memory.
#[derive(Clone, Copy)]
pub(crate) struct Lists<'a> {
    pub vertices: &'a [V3],
    pub colors: &'a [KColor],
    pub uvs: &'a [V2],
    pub indices: &'a [i32],
}

impl<'a> Lists<'a> {
    /// The lists of `data`.
    ///
    /// # Safety
    /// Each list's elements must be live and of its element type for `'a`.
    pub(crate) unsafe fn of(data: &KMeshData) -> Lists<'a> {
        // SAFETY: this function's contract; `V3`, `KColor`, `V2` and `i32` have the elements' layouts.
        unsafe {
            Lists { vertices: data.vertices.elems(), colors: data.colors.elems(), uvs: data.uvs.elems(), indices: data.indices.elems() }
        }
    }

    /// The headless line's argument: the lengths.
    fn arg(&self) -> Arg<'static> {
        let (vertices, colors, uvs, indices) = (self.vertices.len(), self.colors.len(), self.uvs.len(), self.indices.len());
        Arg::MeshData { vertices, colors, uvs, indices }
    }
}

/// `draw3d.make_mesh` for these lists: checks them (headless too), uploads them when `mode` is
/// real, and registers the mesh. A mesh with no vertices has a uv for each of them, so
/// `mesh_textured` can draw an empty mesh (as nothing) whatever it was meant to be.
///
/// `mode` is what `state::drawing` gave on this thread: `Mode::Real` means a real window is open here.
pub(crate) fn make_mesh_in(mode: Mode, lists: &Lists) -> Result<KMesh, String> {
    let Lists { vertices, colors, uvs, indices } = *lists;
    mesh_data::check(vertices.len(), colors.len(), uvs.len(), indices)?;
    let corners = if indices.is_empty() { vertices.len() } else { indices.len() };
    let fits = "`check` keeps a mesh's counts within an int (`mesh_data::limits`)";
    let (vertex_count, triangle_count) = (i32::try_from(vertices.len()).expect(fits), i32::try_from(corners / 3).expect(fits));
    let parts = match mode {
        Mode::Headless => Vec::new(),
        // SAFETY: a real window is open, on this thread (the caller's `mode`), and the lists passed `check`.
        Mode::Real => unsafe { upload(lists) },
    };
    let indexed = !indices.is_empty();
    let object = Object::Mesh { parts, has_uvs: uvs.len() == vertices.len(), indexed };
    Ok(KMesh { id: gpu::add(object), vertex_count, triangle_count })
}

/// The mesh's parts on the GPU: none for no vertices; one, from the lists as they are, for a mesh
/// without indices or with at most `MOST_PART_VERTICES` vertices; otherwise `split`'s parts.
///
/// # Safety
/// A real window must be open, on this thread, and the lists must have passed `mesh_data::check`.
unsafe fn upload(l: &Lists) -> Vec<rl::Mesh> {
    if l.vertices.is_empty() {
        return Vec::new();
    }
    if l.indices.is_empty() || l.vertices.len() <= MOST_PART_VERTICES {
        // Each index is below the vertex count, which is at most 65,536, so it fits a `u16`.
        let indices: Vec<u16> = l.indices.iter().map(|&i| i as u16).collect();
        let colors = mesh_data::colour_bytes(l.colors);
        let arrays = Arrays { vertices: l.vertices, uvs: l.uvs, colors: &colors, indices: &indices };
        // SAFETY: the window is open, on this thread (this function's contract), and the arrays are
        // alive through the call: the lists for the native call, the vectors until this returns.
        return vec![unsafe { upload_part(arrays) }];
    }
    let mut gathered = Gathered::default();
    let parts = mesh_data::split(l.indices, MOST_PART_VERTICES);
    let upload_one = |part: &mesh_data::Part| {
        gathered.fill(l, &part.vertices);
        let arrays = Arrays { vertices: &gathered.vertices, uvs: &gathered.uvs, colors: &gathered.colors, indices: &part.indices };
        // SAFETY: as above; `gathered` and `part` outlive the call.
        unsafe { upload_part(arrays) }
    };
    parts.iter().map(upload_one).collect()
}

/// One part's CPU arrays: positions, and uvs, colours (four bytes each) and indices, each empty for none.
struct Arrays<'a> {
    vertices: &'a [V3],
    uvs: &'a [V2],
    colors: &'a [u8],
    indices: &'a [u16],
}

/// Uploads one part, and gives it back without its CPU arrays (`gpu::without_cpu_pointers`).
///
/// # Safety
/// A real window must be open, on this thread. The arrays hold at most `mesh_data::limits`' counts,
/// one uv and four colour bytes per vertex (or none), and three indices a triangle below the vertex count.
unsafe fn upload_part(a: Arrays) -> rl::Mesh {
    // SAFETY: every field of `Mesh` is an integer or a raw pointer, for which zero (null) is valid.
    let mut mesh: rl::Mesh = unsafe { std::mem::zeroed() };
    let corners = if a.indices.is_empty() { a.vertices.len() } else { a.indices.len() };
    // Within an int, and so are the arrays' sizes in bytes (`mesh_data::limits`).
    mesh.vertex_count = a.vertices.len() as c_int;
    mesh.triangle_count = (corners / 3) as c_int;
    mesh.vertices = array(a.vertices);
    mesh.texcoords = array(a.uvs);
    mesh.colors = array(a.colors);
    mesh.indices = array(a.indices);
    // SAFETY: the window is open, on this thread. `UploadMesh` reads `vertex_count` positions, uvs
    // and colours and `3 * triangle_count` indices, which the arrays hold, and writes only the vertex
    // array and buffer ids. A null array is one the mesh doesn't have: raylib gives the shader a
    // fixed value for it (white for colours).
    unsafe { rl::UploadMesh(&mut mesh, false) };
    gpu::without_cpu_pointers(mesh)
}

/// `items` as a raylib array pointer, or null for none (`as_ptr` of an empty slice isn't null).
/// raylib only reads the arrays it is given to upload, so the `*mut` is never written through.
fn array<T, U>(items: &[T]) -> *mut U {
    if items.is_empty() {
        std::ptr::null_mut()
    } else {
        items.as_ptr().cast_mut().cast()
    }
}

/// A split part's arrays, gathered from the mesh's lists. One is used for every part of a mesh,
/// so the vectors are allocated once.
#[derive(Default)]
struct Gathered {
    vertices: Vec<V3>,
    uvs: Vec<V2>,
    colors: Vec<u8>,
}

impl Gathered {
    /// Fills the arrays with the vertices `numbers` names, in order, and their uvs and colours if
    /// the mesh has them. The numbers are below the vertex count (`split` takes them from checked indices).
    fn fill(&mut self, l: &Lists, numbers: &[u32]) {
        let at = |n: &u32| *n as usize;
        self.vertices.clear();
        self.vertices.extend(numbers.iter().map(|n| l.vertices[at(n)]));
        self.uvs.clear();
        if !l.uvs.is_empty() {
            self.uvs.extend(numbers.iter().map(|n| l.uvs[at(n)]));
        }
        self.colors.clear();
        if !l.colors.is_empty() {
            self.colors.extend(numbers.iter().flat_map(|n| mesh_data::rgba(l.colors[at(n)])));
        }
    }
}

/// What a mesh draw found it can do, once `prepare` has checked everything.
pub(crate) enum Ready {
    /// Print the line.
    Headless,
    /// Draw with raylib, with this texture (`mesh_textured`) or the material's white one.
    Real(Option<rl::Texture>),
}

/// What `mesh` and `mesh_textured` check before drawing, in order: the window is open; the pass
/// can be in 3D, which needs a camera (it moves to 3D now); the mesh exists; for a texture, the
/// mesh was made with uvs and the texture exists.
pub(crate) fn prepare(name: &str, m: KMesh, tex: Option<KTexture>) -> Result<Ready, String> {
    let mode = state::check_drawing(name)?;
    pass::enter_3d(name, mode == Mode::Real)?;
    let has_uvs = gpu::with(m.id, |object| match object {
        Object::Mesh { has_uvs, .. } => Some(*has_uvs),
        Object::Texture { .. } => None,
    })
    .flatten()
    .ok_or_else(|| format!("{name} got an empty mesh; make one with draw3d.make_mesh"))?;
    let texture = match tex {
        None => None,
        Some(_) if !has_uvs => return Err(format!("{name} needs a mesh made with uvs")),
        Some(tex) => lookup(name, tex)?.texture,
    };
    Ok(match mode {
        Mode::Headless => Ready::Headless,
        Mode::Real => Ready::Real(texture),
    })
}

/// What the two drawing natives share: check (`prepare`), then print the line or draw.
fn draw(name: &str, args: &[Arg], m: KMesh, tex: Option<KTexture>, pos: V3, tint: KColor) {
    match prepare(name, m, tex).unwrap_or_else(|msg| fail_at_call(&msg)) {
        Ready::Headless => headless::line(name, args),
        // SAFETY: `prepare` found a real window open, on this thread, moved the pass to 3D, and
        // found `texture` (if any) to be a live texture of the window.
        Ready::Real(texture) => unsafe { draw_real(m.id, texture, pos, tint.to_raylib()) },
    }
}

/// Draws the parts of mesh `id` (which exists) at `pos`, tinted, with `texture` or the material's
/// white one. A mesh with no parts draws nothing and touches nothing.
///
/// # Safety
/// A real window must be open, on this thread, in a 3D pass, and `texture` must be a live texture of it.
unsafe fn draw_real(id: i64, texture: Option<rl::Texture>, pos: V3, tint: rl::Color) {
    gpu::with(id, |object| {
        let Object::Mesh { parts, indexed, .. } = object else { return };
        if parts.is_empty() {
            return;
        }
        // SAFETY (all three blocks below): the window is open, on this thread, in 3D (this function's contract),
        // and the parts are live meshes it uploaded. Drawing adds and frees no GPU object.
        let draw_with = |material| unsafe { draw_parts(parts, *indexed, material, pos) };
        match texture {
            Some(texture) => unsafe { material::with_texture(tint, texture, draw_with) },
            None => draw_with(unsafe { material::tinted(tint) }),
        }
    });
}

/// Draws each part with `material`, moved by `pos`, culling OpenGL's front faces (the module doc).
///
/// # Safety
/// A real window must be open, on this thread, in 3D; `parts` must be meshes it uploaded, with index
/// buffers when `indexed`.
unsafe fn draw_parts(parts: &[rl::Mesh], indexed: bool, material: rl::Material, pos: V3) {
    let transform = translate(pos);
    // SAFETY: this function's contract. The batch is drawn before the cull face changes, so the
    // shapes in it are culled as raylib winds them; it is empty during the mesh draws, which don't
    // use it, so nothing else is drawn with the front faces culled.
    unsafe {
        rl::rlDrawRenderBatchActive();
        rl::rlSetCullFace(rl::RL_CULL_FACE_FRONT);
        for &part in parts {
            let part = if indexed { rl::Mesh { indices: index_marker(), ..part } } else { part };
            rl::DrawMesh(part, material, transform);
        }
        rl::rlSetCullFace(rl::RL_CULL_FACE_BACK);
    }
}

/// What a stored indexed part's `indices` (null, so `UnloadMesh` frees nothing of ours) is replaced
/// with in the copy given to `DrawMesh`.
static INDEX_MARKER: u16 = 0;

/// Under OpenGL 3.3, `DrawMesh` draws an indexed mesh from the index buffer in the part's vertex
/// array, and reads `indices` only to test it for null: `glDrawElements` if it isn't, else
/// `glDrawArrays`. This gives it a pointer that isn't null, to a real `u16` that is never read,
/// written or freed.
fn index_marker() -> *mut u16 {
    std::ptr::addr_of!(INDEX_MARKER).cast_mut()
}

/// The matrix that moves a mesh by `pos`: raylib's `MatrixTranslate`. The translation is `m12`,
/// `m13` and `m14`, the last column (raylib lists the fields row by row).
fn translate(pos: V3) -> rl::Matrix {
    rl::Matrix {
        m0: 1.0,
        m4: 0.0,
        m8: 0.0,
        m12: pos.x,
        m1: 0.0,
        m5: 1.0,
        m9: 0.0,
        m13: pos.y,
        m2: 0.0,
        m6: 0.0,
        m10: 1.0,
        m14: pos.z,
        m3: 0.0,
        m7: 0.0,
        m11: 0.0,
        m15: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_is_raylibs_translation_matrix() {
        let m = translate(V3 { x: 1.5, y: -2.0, z: 3.0 });
        // The diagonal is 1 and the last column is the offset: a point (x, y, z, 1) moves by it.
        let rows = [[m.m0, m.m4, m.m8, m.m12], [m.m1, m.m5, m.m9, m.m13], [m.m2, m.m6, m.m10, m.m14], [m.m3, m.m7, m.m11, m.m15]];
        assert_eq!(rows, [[1.0, 0.0, 0.0, 1.5], [0.0, 1.0, 0.0, -2.0], [0.0, 0.0, 1.0, 3.0], [0.0, 0.0, 0.0, 1.0]]);
        let point = [10.0, 20.0, 30.0, 1.0];
        let moved: Vec<f32> = rows.iter().map(|row| row.iter().zip(point).map(|(a, b)| a * b).sum()).collect();
        assert_eq!(moved, [11.5, 18.0, 33.0, 1.0]);
    }

    #[test]
    fn an_empty_array_is_null_for_raylib() {
        let none: [u16; 0] = [];
        assert!(array::<u16, u16>(&none).is_null());
        let some = [V3 { x: 1.0, y: 2.0, z: 3.0 }];
        let p: *mut f32 = array(&some);
        assert_eq!(p.cast_const(), some.as_ptr().cast::<f32>());
        assert!(!index_marker().is_null());
    }

    #[test]
    fn a_split_part_gathers_its_vertices_uvs_and_colours() {
        let vertices: Vec<V3> = (0..5).map(|i| V3 { x: i as f32, y: 0.0, z: 0.0 }).collect();
        let uvs: Vec<V2> = (0..5).map(|i| V2 { x: i as f32, y: 1.0 }).collect();
        let colors: Vec<KColor> = (0..5).map(|i| KColor { r: i as f32 / 4.0, g: 0.0, b: 1.0, a: 1.0 }).collect();
        let l = Lists { vertices: &vertices, colors: &colors, uvs: &uvs, indices: &[] };
        let mut gathered = Gathered::default();
        gathered.fill(&l, &[4, 0, 2]);
        assert_eq!(gathered.vertices.iter().map(|v| v.x).collect::<Vec<_>>(), [4.0, 0.0, 2.0]);
        assert_eq!(gathered.uvs.iter().map(|v| v.x).collect::<Vec<_>>(), [4.0, 0.0, 2.0]);
        assert_eq!(gathered.colors, [255, 0, 255, 255, 0, 0, 255, 255, 128, 0, 255, 255]);
        // The next part starts afresh, and a mesh without uvs or colours gathers none.
        let bare = Lists { uvs: &[], colors: &[], ..l };
        gathered.fill(&bare, &[1]);
        assert_eq!(gathered.vertices.len(), 1);
        assert!(gathered.uvs.is_empty() && gathered.colors.is_empty());
    }
}
