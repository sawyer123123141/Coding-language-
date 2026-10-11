//! The voxel viewer demo (`demos/voxels`): its world, its flying camera and the viewer itself.
//! The world and camera modules are tested by small programs written next to copies of them in
//! a scratch folder, so the demo folder holds only the demo.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

/// The demo's file `file`.
fn demo(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../demos/voxels").join(file)
}

/// A fresh, empty folder for one test's files.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kestrel_voxels_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes `main` as `main.kes` next to copies of the demo's `modules`, runs it with
/// `kestrel run` (checks on), and gives what it printed. A failed run fails the test with its stderr.
fn run_beside(name: &str, modules: &[&str], main: &str) -> String {
    let dir = scratch(name);
    for module in modules {
        fs::copy(demo(module), dir.join(module)).unwrap_or_else(|e| panic!("copy {module}: {e}"));
    }
    fs::write(dir.join("main.kes"), main).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    let out = common::hermetic(cmd.arg("run").arg(dir.join("main.kes"))).output().unwrap();
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{name}: exit {:?}\n{stderr}", out.status.code());
    stdout.into_owned()
}

/// One solid block makes six faces, wound and coloured right; the bands meet where they should;
/// and the whole 8x4x8 world has the benchmark's 463,749 faces.
const WORLD_CHECK: &str = r#"use { chunk_mesh, mesh_of, band, shaded, PAD, LAYERS, STONE, DIRT, GRASS, SNOW from world }
use { Color from draw }

struct Count {
    x: int
    y: int
    z: int
    faces: int = 0
    vertices: int = 0
    colors: int = 0
    uvs: int = 0
    indices: int = 0

    pure measure(var self) {
        arena {
            let m = chunk_mesh(self.x, self.y, self.z, 1337)
            self.faces = m.indices.len / 6
            self.vertices = m.vertices.len
            self.colors = m.colors.len
            self.uvs = m.uvs.len
            self.indices = m.indices.len
        }
    }
}

pure near(a: Color, b: Color) -> bool {
    abs(a.r - b.r) < 0.000001 and abs(a.g - b.g) < 0.000001 and abs(a.b - b.b) < 0.000001 and a.a == b.a
}

main() {
    // One solid block at (5, 6, 7) in chunk (0, 0, 0).
    var solid = array[u8](PAD * PAD * PAD)
    solid[((6 + 1) * PAD + 7 + 1) * PAD + 5 + 1] = 1
    let m = mesh_of(solid, 0, 0, 0)
    print("block", m.vertices.len, m.colors.len, m.uvs.len, m.indices.len)
    // The faces come in the mesher's order: +x, -x, +y, -y, +z, -z.
    let outward = [vec3(1, 0, 0), vec3(-1, 0, 0), vec3(0, 1, 0), vec3(0, -1, 0), vec3(0, 0, 1), vec3(0, 0, -1)]
    let light = [0.8, 0.8, 1.0, 0.5, 0.7, 0.7]
    for f, out in outward {
        let a = m.vertices[f * 4]
        let b = m.vertices[f * 4 + 1]
        let c = m.vertices[f * 4 + 2]
        // Clockwise seen from outside, so the right-hand normal of a b c points in.
        let inward = (b - a).cross(c - a).dot(out) < 0.0
        print("face", f, inward, near(m.colors[f * 4], shaded(STONE, light[f])))
    }
    print("bands", near(band(0), STONE), near(band(47), STONE), near(band(48), DIRT), near(band(63), DIRT), near(band(64), GRASS), near(band(87), GRASS), near(band(88), SNOW), near(band(127), SNOW))

    var counts = list[Count]()
    for cx from 0 to 8 {
        for cy from 0 to LAYERS {
            for cz from 0 to 8 {
                counts.add(Count(x: cx, y: cy, z: cz))
            }
        }
    }
    par for c in counts {
        c.measure()
    }
    var faces = 0
    var vertices = 0
    var colors = 0
    var uvs = 0
    var indices = 0
    for c in counts {
        faces += c.faces
        vertices += c.vertices
        colors += c.colors
        uvs += c.uvs
        indices += c.indices
    }
    print("world", faces, vertices, colors, uvs, indices)
}
"#;

#[test]
fn the_world_makes_the_benchmarks_faces_wound_and_coloured_right() {
    let out = run_beside("world", &["world.kes"], WORLD_CHECK);
    let mut expected = String::from("block 24 24 0 36\n");
    for f in 0..6 {
        expected += &format!("face {f} true true\n");
    }
    expected += "bands true true true true true true true true\n";
    expected += "world 463749 1854996 1854996 0 2782494\n";
    assert_eq!(out, expected);
}
