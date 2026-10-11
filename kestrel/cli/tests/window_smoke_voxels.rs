//! The real-window test for the voxel viewer's world: its faces drawn on the GPU, seen from above,
//! under both `kestrel run` (the JIT) and `kestrel build` (an exe). The shared machinery is in
//! `common/smoke.rs`; the winding rule itself is checked in `window_smoke_mesh.rs`, and the world's
//! faces are checked in pure Kestrel in `voxels_demo.rs`. This one checks that they meet: the
//! world's top faces are wound so that, seen from above, they are drawn, in full brightness.
//!
//! The window is hidden (`KESTREL_HIDDEN=1`), so no window appears while these run (see
//! `common/smoke.rs` for `KESTREL_SMOKE_VISIBLE`).

mod common;

use common::smoke::{smoke, Case, How, BLACK};

/// The snow band's colour at full brightness: `world.kes`'s `SNOW`, `Color(0.93, 0.95, 0.97)`, as
/// bytes. A top face is drawn in exactly this; a side is 0.8 or 0.7 of it.
const SNOW: [u8; 4] = [237, 242, 247, 255];

/// One column of the world, the four chunks at x and z 0, seen from straight above: the camera is
/// at (16, 300, 16), over the column's middle, looking straight down. A camera looking down
/// lines up with the usual up (+y), which `draw3d.camera` refuses, so up is +z here.
///
/// The column covers 32 by 32 blocks. From 300 blocks up with the default 75-degree field of view,
/// the window's half-height is `(300 - h) * tan(37.5°)` blocks at height h, so at h = 100 it is 153
/// blocks, the 64 pixels cover 307 blocks (4.8 a pixel), and the column covers only 6.7 pixels
/// across the middle: the screenshot shows it at x 29 to 35 and y 28 to 34. The window's corners
/// show the background.
///
/// The top faces are what the camera sees, so if the world's top faces were wound the wrong way
/// they would be culled and the column would show no top at all. Seen from above, a pixel whose
/// samples all land on tops is exactly a height band's colour at full brightness, where a side
/// would be 0.8 or 0.7 of one. Pixel (32, 30) is such a pixel: the snow band's top faces (the
/// peaks), with snow-coloured pixels above, below and to its right.
const VOXEL_COLUMN: Case = Case {
    name: "voxel_column",
    program: r#"use window
use draw
use draw3d
use { Camera, Mesh from draw3d }
use { chunk_mesh, LAYERS from world }

main() {
    window.open(64, 64, "voxels")
    var meshes = list[Mesh]()
    for cy from 0 to LAYERS {
        meshes.add(draw3d.make_mesh(chunk_mesh(0, cy, 0, 1337)))
    }
    // Straight down from high above the column's middle; up is +z, since +y lines up with the view.
    let cam = Camera(pos: vec3(16, 300, 16), target: vec3(16, 0, 16), up: vec3(0, 0, 1))
    var n = 0
    while window.frame() {
        n += 1
        draw.clear(draw.BLACK)
        draw3d.camera(cam)
        for m in meshes {
            draw3d.mesh(m, vec3(0, 0, 0), draw.WHITE)
        }
        if n == 3 {
            window.screenshot("SHOT_PATH")
            window.close()
        }
    }
    print("frames", n)
}
"#,
    stdout: "frames 3\n",
    size: (64, 64),
    pixels: &[
        (1, 1, BLACK, "beside the column"),
        (62, 62, BLACK, "beside the column"),
        (32, 30, SNOW, "inside the peaks' top faces, at full brightness, so they are wound to face the camera"),
    ],
    inked: &[(28, 28, 8, 8, "the column seen from above")],
    assets: &[("../../demos/voxels/world.kes", "world.kes")],
};

#[test]
fn the_voxel_worlds_top_faces_show_from_above_under_the_jit() {
    smoke(How::Jit, &VOXEL_COLUMN);
}

#[test]
fn the_voxel_worlds_top_faces_show_from_above_in_an_exe() {
    smoke(How::Exe, &VOXEL_COLUMN);
}
