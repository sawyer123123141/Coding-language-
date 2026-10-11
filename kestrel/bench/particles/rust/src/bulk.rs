//! The bulk drawing path, copied from `kestrel/window/src/bulk.rs` and `material.rs` (those modules
//! are private to `kestrel-window`): one dynamic raylib mesh, the vertices of every dot written
//! into a reused `Vec<f32>` (6 vertices of `x, y, z` a dot), `UpdateMeshBuffer`, then `DrawMesh`
//! with raylib's default material, culling off around the draw.
//!
//! The one difference: `draw` is generic over where the positions come from, so the fill can read
//! them straight from the particles (what a Rust programmer would write) or from a separate
//! `Vec<V2>` (what the Kestrel demo does). It times the fill, the upload and the `DrawMesh` call.

use std::ffi::c_int;
use std::time::{Duration, Instant};

use kestrel_window::raylib as rl;

use crate::sim::V2;

/// The floats one rectangle takes: two triangles of three vertices, each `x, y, z`.
const FLOATS_PER_RECT: usize = 18;
/// The depth of every vertex (the window crate explains the -0.5).
const Z: f32 = -0.5;
const FIRST_CAPACITY: usize = 16;
/// The most dots one draw takes. The window crate draws more in parts; the bench never needs it.
const MOST_PER_DRAW: usize = 1 << 20;

const IDENTITY: rl::Matrix = rl::Matrix {
    m0: 1.0,
    m4: 0.0,
    m8: 0.0,
    m12: 0.0,
    m1: 0.0,
    m5: 1.0,
    m9: 0.0,
    m13: 0.0,
    m2: 0.0,
    m6: 0.0,
    m10: 1.0,
    m14: 0.0,
    m3: 0.0,
    m7: 0.0,
    m11: 0.0,
    m15: 1.0,
};

/// Time spent in each part of the draws since `take`.
#[derive(Default, Clone, Copy)]
pub struct Spent {
    pub fill: Duration,
    pub upload: Duration,
    pub draw_mesh: Duration,
    /// How many times the thread was on a different logical CPU after the fill and after the upload.
    pub moves: u32,
}

/// The logical CPU this thread is on right now.
fn cpu() -> u32 {
    extern "system" {
        fn GetCurrentProcessorNumber() -> u32;
    }
    // SAFETY: a plain Win32 query.
    unsafe { GetCurrentProcessorNumber() }
}

/// What happens to the vertices after the fill. The bench's default is what the window crate does
/// (`Gl`); the others take the GPU out, to see what the fill and the upload cost without it.
#[derive(Clone, Copy, Default, PartialEq)]
pub enum Upload {
    /// `UpdateMeshBuffer`, then `DrawMesh`.
    #[default]
    Gl,
    /// `UpdateMeshBuffer` only: no `DrawMesh`.
    GlNoDraw,
    /// `DrawMesh` only: it draws the vertices uploaded the first frame.
    GlDrawOnly,
    /// A `memcpy` of the vertices into a second `Vec` (no GL call at all).
    Memcpy,
    /// Nothing: the vertices are written and never read.
    Skip,
}

#[derive(Default)]
pub struct Bulk {
    upload: Upload,
    sink: Vec<f32>,
    drawn: u64,
    /// The mesh on the GPU, once made, and how many dots its vertex buffer holds.
    mesh: Option<(rl::Mesh, usize)>,
    material: Option<rl::Material>,
    vertices: Vec<f32>,
    spent: Spent,
}

/// Writes the vertices of a `size` rectangle at each item's position into `buf`, and gives them.
/// The same loop as `fill_into` in the window crate, for a positive `size`.
fn fill_into<'a, T>(buf: &'a mut Vec<f32>, items: &[T], position: impl Fn(&T) -> V2, size: V2) -> &'a [f32] {
    let len = items.len() * FLOATS_PER_RECT;
    if buf.len() < len {
        buf.resize(len, 0.0);
    }
    let out = &mut buf[..len];
    for (rect, item) in out.as_chunks_mut::<FLOATS_PER_RECT>().0.iter_mut().zip(items) {
        let p = position(item);
        let (left, top) = (p.x, p.y);
        let (right, bottom) = (left + size.x, top + size.y);
        // Top-left, top-right, bottom-right; then top-left, bottom-right, bottom-left.
        *rect = [
            left, top, Z, right, top, Z, right, bottom, Z, left, top, Z, right, bottom, Z, left, bottom, Z,
        ];
    }
    out
}

/// The capacity a mesh that holds `capacity` dots grows to when it needs `needed`: doubled until
/// they fit.
fn grown(capacity: usize, needed: usize) -> usize {
    let mut grown = capacity.max(FIRST_CAPACITY);
    while grown < needed {
        grown *= 2;
    }
    grown
}

impl Bulk {
    pub fn new(upload: Upload) -> Bulk {
        Bulk { upload, ..Bulk::default() }
    }

    /// The mesh, first made or grown if it holds fewer than `n` dots (the old one is not freed:
    /// the bench grows it a few times at the start and then exits).
    ///
    /// # Safety
    /// A real window must be open, on this thread.
    unsafe fn mesh_for(&mut self, n: usize) -> rl::Mesh {
        let held = match self.mesh {
            Some((mesh, capacity)) if capacity >= n => return mesh,
            Some((_, capacity)) => capacity,
            None => 0,
        };
        let capacity = grown(held, n);
        // SAFETY: every field of `Mesh` is an integer or a raw pointer, for which zero is valid.
        let mut mesh: rl::Mesh = unsafe { std::mem::zeroed() };
        mesh.vertex_count = (6 * capacity) as c_int;
        mesh.triangle_count = (2 * capacity) as c_int;
        // SAFETY: the window is open, on this thread. With every CPU array null, `UploadMesh`
        // makes a dynamic position buffer for `vertex_count` vertices without reading memory.
        unsafe { rl::UploadMesh(&mut mesh, true) };
        self.mesh = Some((mesh, capacity));
        mesh
    }

    /// Draws a `size` rectangle at each item's position, in `color`, as one mesh.
    ///
    /// # Safety
    /// A real window must be open, on this thread, drawing in 2D.
    pub unsafe fn draw<T>(&mut self, items: &[T], position: impl Fn(&T) -> V2, size: V2, color: rl::Color) {
        assert!(items.len() <= MOST_PER_DRAW);
        // SAFETY: the window is open, on this thread (this function's contract). The mesh holds
        // at least `items.len()` dots, so the upload stays inside its vertex buffer and the draw
        // reads only vertices just written. `maps` has the diffuse map, which lives forever.
        unsafe {
            let material = *self.material.get_or_insert_with(|| rl::LoadMaterialDefault());
            (*material.maps.add(rl::MATERIAL_MAP_DIFFUSE)).color = color;
            rl::rlDrawRenderBatchActive();
            rl::rlDisableBackfaceCulling();
            let mesh = self.mesh_for(items.len());
            let (t0, cpu0) = (Instant::now(), cpu());
            let vertices = fill_into(&mut self.vertices, items, position, size);
            let (t1, cpu1) = (Instant::now(), cpu());
            let (n, bytes) = (items.len() as c_int, std::mem::size_of_val(vertices) as c_int);
            match self.upload {
                Upload::Gl | Upload::GlNoDraw => rl::UpdateMeshBuffer(mesh, rl::MESH_BUFFER_POSITIONS, vertices.as_ptr().cast(), bytes, 0),
                Upload::GlDrawOnly if self.drawn == 0 => {
                    rl::UpdateMeshBuffer(mesh, rl::MESH_BUFFER_POSITIONS, vertices.as_ptr().cast(), bytes, 0)
                }
                Upload::GlDrawOnly => {}
                Upload::Memcpy => {
                    self.sink.resize(vertices.len(), 0.0);
                    self.sink.copy_from_slice(vertices);
                    std::hint::black_box(self.sink.as_ptr());
                }
                Upload::Skip => {
                    std::hint::black_box(vertices.as_ptr());
                }
            }
            let (t2, cpu2) = (Instant::now(), cpu());
            self.drawn += 1;
            if matches!(self.upload, Upload::Gl | Upload::GlDrawOnly) {
                rl::DrawMesh(
                    rl::Mesh {
                        vertex_count: 6 * n,
                        triangle_count: 2 * n,
                        ..mesh
                    },
                    material,
                    IDENTITY,
                );
            }
            let t3 = Instant::now();
            rl::rlEnableBackfaceCulling();
            self.spent.fill += t1 - t0;
            self.spent.upload += t2 - t1;
            self.spent.draw_mesh += t3 - t2;
            self.spent.moves += (cpu0 != cpu1) as u32 + (cpu1 != cpu2) as u32;
        }
    }

    /// The time spent since the last call.
    pub fn take(&mut self) -> Spent {
        std::mem::take(&mut self.spent)
    }
}
