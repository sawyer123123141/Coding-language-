# 3D plan 3: the voxel viewer and the docs

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a demo that flies through the `bench/voxels3d` world in Kestrel's own window, measured honestly, and the docs brought up to date with everything sub-project 4b part 2 built.

**Architecture:**
- **`kestrel/demos/voxels/world.kes`**: the benchmark's noise and mesher, copied and changed to fill a `draw3d.MeshData`. Each face gets its height band's colour, shaded by the way it faces. All of it is `pure`, so it runs in a `par for`.
- **`kestrel/demos/voxels/fly.kes`**: a flying camera (`Flyer`) as pure maths: yaw and pitch in degrees, WASD-style movement, mouse look, a wheel-set speed, and the `Camera` it gives.
- **`kestrel/demos/voxels/voxels.kes`**: the viewer. Inside an `arena { }`, a `par for` builds every chunk's `MeshData`, a loop makes a GPU `Mesh` from each non-empty one, and the arena then frees all the CPU data. The frame loop flies the camera, draws every mesh, and draws a text overlay. It prints a line at start (build and upload times) and one a second (draw and frame times), like the particles demo.
- **The benchmark stays frozen.** `kestrel/bench/voxels3d` is the reference for the C# comparison, so the demo has its own copy of the noise and mesher instead of changing or importing the benchmark's.
- **Docs:** the 3D spec gains an "As built" section, the syntax and window specs lose stale examples, the follow-ups file gains plan 2's and plan 3's findings, and the README shows textures and 3D.

**Tech Stack:** Kestrel (`.kes` demo code), Rust CLI integration tests (`kestrel/cli/tests/`), the GPU smoke helpers (`kestrel/cli/tests/common/smoke.rs`, hidden windows), Python for the measuring script.

**Spec:** `docs/superpowers/specs/2026-10-10-3d-textures-design.md` (the "Demos" section, "The voxel viewer", and "Build order" item 3). Read "What it looks like", "`draw3d`" and "GPU objects" too. Plans 1 and 2 (`docs/superpowers/plans/2026-10-10-3d-1-compiler.md`, `2026-10-10-3d-2-window.md`) built what this plan uses. The handoff (`docs/superpowers/plans/2026-10-10-handoff.md`) lists plan 2's rulings and follow-ups, which Task 4 writes into the docs.

## Global Constraints

- **Success (spec):** "the voxel viewer shows the 8×4×8-chunk world (463,749 faces) at 60 fps on the development machine." At seed 1337 and `WIDTH = 8`, `world.kes` must make exactly 463,749 faces, the benchmark's count.
- **The viewer (spec, verbatim):** "`voxels.kes` opens a 1280×720 window. Inside an `arena { }`, a `par for` builds every chunk's `MeshData`, a loop makes a `Mesh` from each, and the arena then frees all the CPU data." "The frame loop flies the camera: WASD to move, Space and Shift to go up and down, the right mouse button held and dragged to look, and the wheel to change speed. It draws every chunk mesh and a text overlay with the fps, the frame time and the face count." "It prints a line once a second, like the particles demo, and a line at start with the world build time and the upload time (from `ticks_usec`)." "A constant `WIDTH` sets the world size: 8 chunks across, as in the benchmark, and larger for a stress run."
- **Colours (spec, verbatim):** "Each face is coloured by its height band and shaded by the way it faces (top 1.0, sides 0.8 and 0.7, bottom 0.5)." This plan's ruling: faces along ±x get 0.8, along ±z 0.7.
- **Results (spec, verbatim):** "`results.md` records the build (`par for`) and upload times, the frame time and fps at `WIDTH` 8 and 16, the exe size, and what limits it, honestly."
- **Never show a window.** Every window program run by hand or by a script has `KESTREL_HEADLESS=<frames>` or `KESTREL_HIDDEN=1`. Never set `KESTREL_SMOKE_VISIBLE`. A hidden window does **not** wait for vsync on this machine, so fps there is uncapped: judge "60 fps" by a frame time under 16.67 ms. Hidden windows were once seen capped at 60 fps for a while (`kestrel/bench/particles/results.md`, the end); throw away runs whose frame is pinned at 16.67 ms.
- Source files stay under about 400 lines; one job per file.
- Error and comment text are plain sentences. Demo comments explain in GDScript/Godot terms where that helps (the user knows Godot).
- Plan 1's known leak shapes don't apply to `Mesh` values kept in a `let` or a list, but don't make a `Mesh` or `Texture` in a `while` condition or in a mixed `if` passed as an argument.
- From `kestrel/`, before every commit: `cargo test --workspace --no-fail-fast`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo clippy --all-targets -- -D warnings` in `kestrel/godot`.
- Write files containing `\n` or `\\` with the Write/Edit tools, not shell heredocs (a hook rewrites shell commands).
- Commit after each task with a plain-sentence message ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. The controller pushes after the final review.
- The flaky `relocated` CLI test sometimes fails with "file in use"; re-run it.

## Review Focus

1. **Looking straight up or down.** A camera whose view lines up with `up` is a runtime error ("draw3d.camera needs an up that doesn't point along the view"). Pitch must stay within ±89°, however far the mouse is dragged. Task 2's `fly.kes` test drags far up and far down.
2. **The speed wheel.** Many notches in either direction must keep the speed within its bounds (2 to 400 blocks a second), not reach 0 or infinity. Task 2 tests 100 notches each way.
3. **All-air chunks.** An empty `MeshData` must not become a mesh the viewer draws for nothing. The viewer skips chunks with no vertices, and the face count stays exact. Task 2's headless test checks one `make_mesh` per drawn mesh and every mesh drawn each frame.
4. **CPU memory after the upload.** The spec promises "the arena then frees all the CPU data", about 70 MB of mesh lists at `WIDTH = 8`. Task 2 checks the program arena's size with `KESTREL_ARENA_REPORT=1` after a headless run.
5. **Winding of the world's faces.** A face wound the wrong way is culled, and the world looks inside-out. Task 1 checks every face's winding in pure Kestrel (the right-hand normal of a clockwise face points inward). Task 2's GPU smoke case checks top faces from above show their full-brightness colour.

---

### Task 1: The world: noise and mesher filling `MeshData`

**Files:**
- Create: `kestrel/demos/voxels/world.kes`, `kestrel/cli/tests/voxels_demo.rs`

**Interfaces:**
- Consumes: `draw3d.MeshData` with `pure quad(var self, a: vec3, b: vec3, c: vec3, d: vec3, color: Color)` (two triangles `a b c` and `a c d`, clockwise seen from the front); `draw.Color`.
- Produces (`world.kes`):
  - `pub let SIZE = 32`, `pub let PAD = SIZE + 2`, `pub let LAYERS = 4`, `pub let HEIGHT = LAYERS * SIZE`
  - `pub let STONE`, `DIRT`, `GRASS`, `SNOW` (Colors, below)
  - `pub pure band(y: int) -> Color`: the colour of world height `y`
  - `pub pure shaded(c: Color, light: float) -> Color`
  - `pub pure solids(cx: int, cy: int, cz: int, seed: int) -> [u8]`: solid flags for a chunk plus a one-block border
  - `pub pure mesh_of(solid: [u8], cx: int, cy: int, cz: int) -> MeshData`: the faces of a chunk's solid flags
  - `pub pure chunk_mesh(cx: int, cy: int, cz: int, seed: int) -> MeshData`
  - Face order inside each block: +x, -x, +y, -y, +z, -z (the benchmark's order).
- Produces (`voxels_demo.rs`): a helper `run_beside(name: &str, modules: &[&str], main: &str) -> String` that Task 2 reuses.

- [ ] **Step 1: Write the failing test.** Create `kestrel/cli/tests/voxels_demo.rs`:

```rust
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
```

- [ ] **Step 2: Run it to see it fail.**

Run (from `kestrel/`): `cargo test -p kestrel-cli --test voxels_demo`
Expected: FAIL: `copy world.kes: ... cannot find the file` (the demo doesn't exist yet).

- [ ] **Step 3: Write `kestrel/demos/voxels/world.kes`.** The noise is the benchmark's (`bench/voxels3d/voxels3d.kes`), unchanged; the mesher fills a `MeshData` with colours instead of normals:

```
// The voxel world of bench/voxels3d, for the viewer: chunks of 32x32x32
// blocks, stacked 4 high. Density is 3D value noise in three octaves, so
// there are hills, caves and overhangs. Each chunk's mesh gets a face for
// every side of a solid block that touches air, coloured by its height and
// shaded by the way it faces (there is no lighting, as in early Minecraft).
//
// The benchmark keeps its own copy of this code: it is the reference for
// the C# comparison, so it doesn't change when the demo does.
use { Color from draw }
use { MeshData from draw3d }

pub let SIZE = 32
pub let PAD = SIZE + 2       // solid flags include a one-block border
pub let LAYERS = 4           // chunks stacked vertically
pub let HEIGHT = LAYERS * SIZE

// The height bands, bottom to top.
pub let STONE = Color(0.52, 0.52, 0.55)
pub let DIRT = Color(0.47, 0.34, 0.22)
pub let GRASS = Color(0.33, 0.62, 0.27)
pub let SNOW = Color(0.93, 0.95, 0.97)

// How bright each side of a block is: the top brightest, the bottom darkest.
let TOP = 1.0
let SIDE_X = 0.8
let SIDE_Z = 0.7
let BOTTOM = 0.5

pure hash(x: int, y: int, z: int, seed: int) -> int {
    var h = i64(x) * 73856093 ^ i64(y) * 19349663 ^ i64(z) * 83492791 ^ i64(seed) * 1610612741
    h = h & 0x7FFFFFFF
    h = (h ^ (h >> 13)) * 0x5BD1E995 & 0x7FFFFFFF
    int(h ^ (h >> 15))
}

pure lattice(x: int, y: int, z: int, seed: int) -> float {
    float(hash(x, y, z, seed) & 0xFFFF) / 65535.0
}

pure smooth(t: float) -> float {
    t * t * (3.0 - 2.0 * t)
}

pure mix(a: float, b: float, t: float) -> float {
    a + (b - a) * t
}

// One octave: lattice values every 2^shift blocks, blended smoothly
// between the 8 corners of the cell around (x, y, z).
pure octave(x: int, y: int, z: int, shift: int, seed: int) -> float {
    let mask = (1 << shift) - 1
    let scale = float(1 << shift)
    let cx = x >> shift
    let cy = y >> shift
    let cz = z >> shift
    let fx = smooth(float(x & mask) / scale)
    let fy = smooth(float(y & mask) / scale)
    let fz = smooth(float(z & mask) / scale)
    let x00 = mix(lattice(cx, cy, cz, seed), lattice(cx + 1, cy, cz, seed), fx)
    let x10 = mix(lattice(cx, cy + 1, cz, seed), lattice(cx + 1, cy + 1, cz, seed), fx)
    let x01 = mix(lattice(cx, cy, cz + 1, seed), lattice(cx + 1, cy, cz + 1, seed), fx)
    let x11 = mix(lattice(cx, cy + 1, cz + 1, seed), lattice(cx + 1, cy + 1, cz + 1, seed), fx)
    mix(mix(x00, x10, fy), mix(x01, x11, fy), fz)
}

// Solid where above zero: noise, pulled toward air higher up.
pure density(x: int, y: int, z: int, seed: int) -> float {
    let n = octave(x, y, z, 5, seed) * 0.5 + octave(x, y, z, 4, seed + 1) * 0.3 +
        octave(x, y, z, 3, seed + 2) * 0.2
    n - 0.5 - float(y - HEIGHT / 2) / float(HEIGHT * 2)
}

// The colour of a block at world height y: stone low down, then dirt,
// grass, and snow on the peaks.
pub pure band(y: int) -> Color {
    if y < HEIGHT * 3 / 8 {
        STONE
    } else if y < HEIGHT / 2 {
        DIRT
    } else if y < HEIGHT * 11 / 16 {
        GRASS
    } else {
        SNOW
    }
}

// `c` at brightness `light` (1 leaves it as it is).
pub pure shaded(c: Color, light: float) -> Color {
    Color(c.r * light, c.g * light, c.b * light, c.a)
}

// Solid flags (1 or 0) for chunk (cx, cy, cz) plus a one-block border, so
// the mesher can see the neighbouring chunks' blocks.
pub pure solids(cx: int, cy: int, cz: int, seed: int) -> [u8] {
    var out = array[u8](PAD * PAD * PAD)
    var i = 0
    for y from 0 to PAD {
        for z from 0 to PAD {
            for x from 0 to PAD {
                if density(cx * SIZE + x - 1, cy * SIZE + y - 1, cz * SIZE + z - 1, seed) > 0.0 {
                    out[i] = 1
                }
                i += 1
            }
        }
    }
    out
}

// The mesh of chunk (cx, cy, cz) from its solid flags: a face for each
// solid block side that touches air. The corners of each face go clockwise
// seen from outside, Godot's front-face order.
pub pure mesh_of(solid: [u8], cx: int, cy: int, cz: int) -> MeshData {
    var m = MeshData()
    for y from 0 to SIZE {
        let colour = band(cy * SIZE + y)
        let top = shaded(colour, TOP)
        let side_x = shaded(colour, SIDE_X)
        let side_z = shaded(colour, SIDE_Z)
        let bottom = shaded(colour, BOTTOM)
        for z from 0 to SIZE {
            for x from 0 to SIZE {
                let i = ((y + 1) * PAD + z + 1) * PAD + x + 1
                if solid[i] == 1 {
                    let p = vec3(cx * SIZE + x, cy * SIZE + y, cz * SIZE + z)
                    if solid[i + 1] == 0 {
                        m.quad(p + vec3(1, 0, 0), p + vec3(1, 0, 1), p + vec3(1, 1, 1), p + vec3(1, 1, 0), side_x)
                    }
                    if solid[i - 1] == 0 {
                        m.quad(p + vec3(0, 0, 1), p, p + vec3(0, 1, 0), p + vec3(0, 1, 1), side_x)
                    }
                    if solid[i + PAD * PAD] == 0 {
                        m.quad(p + vec3(0, 1, 0), p + vec3(1, 1, 0), p + vec3(1, 1, 1), p + vec3(0, 1, 1), top)
                    }
                    if solid[i - PAD * PAD] == 0 {
                        m.quad(p, p + vec3(0, 0, 1), p + vec3(1, 0, 1), p + vec3(1, 0, 0), bottom)
                    }
                    if solid[i + PAD] == 0 {
                        m.quad(p + vec3(1, 0, 1), p + vec3(0, 0, 1), p + vec3(0, 1, 1), p + vec3(1, 1, 1), side_z)
                    }
                    if solid[i - PAD] == 0 {
                        m.quad(p, p + vec3(1, 0, 0), p + vec3(1, 1, 0), p + vec3(0, 1, 0), side_z)
                    }
                }
            }
        }
    }
    m
}

// The mesh of chunk (cx, cy, cz) of the world made from `seed`.
pub pure chunk_mesh(cx: int, cy: int, cz: int, seed: int) -> MeshData {
    mesh_of(solids(cx, cy, cz, seed), cx, cy, cz)
}
```

  If the checker rejects something here (for example a `Color` constant's use in a `pure` function, or the `if` chain as `band`'s value), make the smallest change that keeps the behaviour, and say what and why in the report.

- [ ] **Step 4: Run the test to see it pass.**

Run: `cargo test -p kestrel-cli --test voxels_demo`
Expected: PASS (1 test). If the world count differs from 463,749, compare the noise with `bench/voxels3d/voxels3d.kes` line by line: it must be the same.

- [ ] **Step 5: Run the suite and clippy** (Global Constraints). Commit "Add the voxel viewer's world: the benchmark's noise and mesher filling MeshData".

---

### Task 2: The flying camera and the viewer

**Files:**
- Create: `kestrel/demos/voxels/fly.kes`, `kestrel/demos/voxels/voxels.kes`, `kestrel/cli/tests/window_smoke_voxels.rs`
- Modify: `kestrel/cli/tests/voxels_demo.rs`

**Interfaces:**
- Consumes: `world.kes` (Task 1): `chunk_mesh`, `LAYERS`, `SIZE`, the band colours. `draw3d`: `Camera { pos, target, up = vec3(0, 1, 0), fov = 75 }`, `make_mesh(data: MeshData) -> Mesh`, `mesh(m: Mesh, pos: vec3, tint: Color)`, `camera(cam: Camera)`. `window.ticks_usec() -> i64`. `input`: `held`, `axis`, `mouse`, `wheel`, `Key.MouseRight`, `Key.W/A/S/D`, `Key.Space`, `Key.Shift`. The smoke helpers in `cli/tests/common/smoke.rs` (`Case`, with `assets` copied next to the program from paths relative to `kestrel/tests/programs/`), used the way `cli/tests/window_smoke_mesh.rs` uses them. `run_beside` from Task 1.
- Produces (`fly.kes`):
  - `pub struct Flyer { pos: vec3, yaw: float, pitch: float, speed: float = 20 }`. Yaw and pitch are degrees; yaw 0 looks along +z and yaw 90 along +x; pitch is positive looking up.
  - `pub pure forward(yaw: float, pitch: float) -> vec3`, `pub pure right(yaw: float) -> vec3`
  - `Flyer`'s method `pure fly(var self, move: vec3, turn: vec2, wheel: float, dt: float)`: `move` is (right, up, forward), each -1 to 1; `turn` is the mouse movement in pixels; `wheel` is notches.
  - `pub pure view(f: Flyer) -> Camera`
- Produces (`voxels.kes`) two printed line formats, which Task 3 measures:
  - at start: `voxels world W chunks C meshes M faces F build B ms upload U ms`
  - once a second: `voxels W meshes M faces F draw D ms frame T ms fps P`

- [ ] **Step 1: Write the failing camera test.** Add to `voxels_demo.rs`:

```rust
/// The flying camera's directions, moves, turns, limits and the `Camera` it gives.
const FLY_CHECK: &str = r#"use { Flyer, forward, right, view from fly }

// Rounded to three decimals, with -0 made 0, so the printed numbers are exact.
pure r(v: vec3) -> vec3 {
    vec3(round(v.x * 1000) / 1000 + 0.0, round(v.y * 1000) / 1000 + 0.0, round(v.z * 1000) / 1000 + 0.0)
}

main() {
    print(r(forward(0, 0)), r(forward(90, 0)), r(forward(0, 90)))
    print(r(right(0)), r(right(90)))
    var f = Flyer(pos: vec3(0, 0, 0), yaw: 0, pitch: 0)
    f.fly(vec3(0, 0, 1), vec2(0, 0), 0, 0.5)
    print(r(f.pos))
    f.fly(vec3(1, 1, 0), vec2(0, 0), 0, 0.5)
    print(r(f.pos))
    f.fly(vec3(0, 0, 0), vec2(-450, 0), 0, 0.1)
    print(f.yaw, f.pitch)
    f.fly(vec3(0, 0, 0), vec2(0, -10000), 0, 0.1)
    print(f.pitch)
    f.fly(vec3(0, 0, 0), vec2(0, 20000), 0, 0.1)
    print(f.pitch)
    f.fly(vec3(0, 0, 0), vec2(0, 0), 1, 0.1)
    print(f.speed)
    f.fly(vec3(0, 0, 0), vec2(0, 0), 100, 0.1)
    print(f.speed)
    f.fly(vec3(0, 0, 0), vec2(0, 0), -100, 0.1)
    print(f.speed)
    let cam = view(f)
    print(r(cam.pos), r(cam.target - cam.pos))
}
"#;

#[test]
fn the_flying_camera_moves_turns_and_keeps_its_limits() {
    let out = run_beside("fly", &["fly.kes"], FLY_CHECK);
    let expected = [
        // Yaw 0 looks along +z and yaw 90 along +x; pitch 90 looks up.
        "vec3(x: 0, y: 0, z: 1) vec3(x: 1, y: 0, z: 0) vec3(x: 0, y: 1, z: 0)",
        // Right-handed with y up: looking along +z, right is -x.
        "vec3(x: -1, y: 0, z: 0) vec3(x: 0, y: 0, z: 1)",
        // W for half a second at 20 blocks a second.
        "vec3(x: 0, y: 0, z: 10)",
        // D and Space together: the move is normalised, so diagonals aren't faster.
        "vec3(x: -7.071, y: 7.071, z: 10)",
        // Dragging 450 pixels left turns 90 degrees left (0.2 degrees a pixel).
        "90 0",
        // Dragging far up or down stops at 89 degrees, so the view never lines up with up.
        "89",
        "-89",
        // One wheel notch is 1.25 times as fast; 100 notches stop at 400, and -100 at 2.
        "25",
        "400",
        "2",
        // The camera looks from the flyer's position along its view (yaw 90, pitch -89).
        "vec3(x: -7.071, y: 7.071, z: 10) vec3(x: 0.017, y: -1, z: 0)",
    ];
    assert_eq!(out, expected.map(|line| format!("{line}\n")).concat());
}
```

- [ ] **Step 2: Run it to see it fail.** `cargo test -p kestrel-cli --test voxels_demo the_flying_camera` → FAIL (no `fly.kes`).

- [ ] **Step 3: Write `kestrel/demos/voxels/fly.kes`:**

```
// A flying camera, like the Godot editor's free look: the right mouse
// button held and dragged turns it, WASD moves it where it looks, Space
// and Shift go straight up and down, and the wheel changes the speed.
// It is plain maths (`pure`); voxels.kes reads the input and calls `fly`.
use { Camera from draw3d }

let LOOK = 0.2           // degrees of turn per pixel dragged
let MOST_PITCH = 89.0    // so the view never lines up with up
let WHEEL_STEP = 1.25    // speed change per wheel notch
let SLOWEST = 2.0        // blocks a second
let FASTEST = 400.0

pub struct Flyer {
    pos: vec3
    yaw: float           // degrees: 0 looks along +z, 90 along +x
    pitch: float         // degrees: positive looks up
    speed: float = 20    // blocks a second

    // One frame of flying: `move` is (right, up, forward), each -1 to 1,
    // `turn` the mouse movement in pixels, and `wheel` the wheel's notches.
    pure fly(var self, move: vec3, turn: vec2, wheel: float, dt: float) {
        self.yaw -= turn.x * LOOK
        self.pitch = clamp(self.pitch - turn.y * LOOK, -MOST_PITCH, MOST_PITCH)
        self.speed = clamp(self.speed * pow(WHEEL_STEP, wheel), SLOWEST, FASTEST)
        var dir = move
        if dir.length() > 1 {
            dir = dir.normalized()
        }
        let step = right(self.yaw) * dir.x + vec3(0, 1, 0) * dir.y + forward(self.yaw, self.pitch) * dir.z
        self.pos += step * self.speed * dt
    }
}

// The way a flyer at `yaw` and `pitch` looks, as a vector of length 1.
pub pure forward(yaw: float, pitch: float) -> vec3 {
    let y = yaw * PI / 180
    let p = pitch * PI / 180
    vec3(sin(y) * cos(p), sin(p), cos(y) * cos(p))
}

// The flyer's right on the ground: forward (level) crossed with up.
pub pure right(yaw: float) -> vec3 {
    let y = yaw * PI / 180
    vec3(-cos(y), 0, sin(y))
}

// The camera that sees what the flyer sees.
pub pure view(f: Flyer) -> Camera {
    Camera(pos: f.pos, target: f.pos + forward(f.yaw, f.pitch))
}
```

  Run the test: it passes. If a printed number differs only in its last digit (float rounding), fix `fly.kes` only if the maths is wrong; otherwise report the exact line and the reason, and adjust the expected line.

- [ ] **Step 4: Write the failing viewer tests.** Add to `voxels_demo.rs`:

```rust
use std::process::Output;

/// Runs the viewer with `--release` in a headless window of `frames` frames, with these extra
/// environment variables.
fn run_viewer(frames: u32, env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    let cmd = common::hermetic(cmd.arg("run").arg("--release").arg(demo("voxels.kes"))).env("KESTREL_HEADLESS", frames.to_string());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

/// The number after `word` in `line` (`meshes 230` gives 230 for "meshes").
fn number_after(line: &str, word: &str) -> usize {
    let mut words = line.split(' ');
    words.position(|w| w == word).unwrap_or_else(|| panic!("no {word:?} in {line:?}"));
    words.next().and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("no number after {word:?} in {line:?}"))
}

#[test]
fn the_viewer_builds_the_world_and_reports_its_times() {
    // The headless clock is 60 frames a second and the viewer prints once a second, so 70
    // frames give the start line and one report line.
    let out = run_viewer(70, &[("KESTREL_HEADLESS_QUIET", "1")]);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "exit {:?}\n{stderr}", out.status.code());
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "expected the start line and one report line, got:\n{stdout}");
    assert!(lines[0].starts_with("voxels world 8 chunks 256 meshes "), "start line: {}", lines[0]);
    assert_eq!(number_after(lines[0], "faces"), 463_749, "start line: {}", lines[0]);
    assert!(lines[0].contains(" build ") && lines[0].contains(" upload ") && lines[0].ends_with(" ms"), "start line: {}", lines[0]);
    let meshes = number_after(lines[0], "meshes");
    assert!(meshes > 0 && meshes <= 256, "meshes: {meshes}");
    assert!(lines[1].starts_with(&format!("voxels 8 meshes {meshes} faces 463749 draw ")), "report line: {}", lines[1]);
    assert!(lines[1].ends_with(" fps 60"), "the headless clock is 60 fps: {}", lines[1]);
}

#[test]
fn the_viewer_uploads_each_non_empty_chunk_once_and_draws_every_mesh_each_frame() {
    let out = run_viewer(2, &[]);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "exit {:?}\n{stderr}", out.status.code());
    let start = stdout.lines().find(|l| l.starts_with("voxels world ")).unwrap_or_else(|| panic!("no start line:\n{stdout}"));
    let meshes = number_after(start, "meshes");
    let called = |name: &str| stdout.lines().filter(|line| line.split(' ').next() == Some(name)).collect::<Vec<_>>();
    let made = called("draw3d.make_mesh");
    assert_eq!(made.len(), meshes, "one make_mesh per mesh");
    for line in &made {
        // `draw3d.make_mesh MeshData(vertices: N, colors: N, uvs: 0, indices: N)`
        let n = |field: &str| number_after(&line.replace([',', '(', ')'], " "), field);
        assert!(n("vertices:") > 0, "an empty chunk was uploaded: {line}");
        assert_eq!(n("colors:"), n("vertices:"), "one colour per vertex: {line}");
        assert_eq!(n("uvs:"), 0, "{line}");
        assert_eq!(n("indices:"), n("vertices:") / 4 * 6, "two triangles per four vertices: {line}");
    }
    assert_eq!(called("draw3d.camera").len(), 2, "one camera a frame");
    assert_eq!(called("draw3d.mesh").len(), 2 * meshes, "every mesh in both frames");
}

#[test]
fn the_viewer_frees_the_cpu_mesh_data_once_it_is_uploaded() {
    // The world's mesh lists are about 70 MB at WIDTH 8; they live in an `arena { }` that ends
    // after the upload, so the program arena stays small. (Without the arena it holds all of them.)
    let dir = scratch("arena");
    let exe = dir.join("voxels").with_extension(std::env::consts::EXE_EXTENSION);
    let mut build = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    let built = common::hermetic(build.arg("build").arg("--release").arg(demo("voxels.kes")).arg("-o").arg(&exe)).output().unwrap();
    assert!(built.status.success(), "build failed:\n{}", String::from_utf8_lossy(&built.stderr));
    let mut run = Command::new(&exe);
    let out = common::hermetic(&mut run)
        .env("KESTREL_HEADLESS", "2")
        .env("KESTREL_HEADLESS_QUIET", "1")
        .env("KESTREL_ARENA_REPORT", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "exit {:?}\n{}", out.status.code(), String::from_utf8_lossy(&out.stderr));
    // Read the report the way cli/tests/text_memory.rs does (same stream, same line).
    let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let bytes: u64 = all
        .lines()
        .find_map(|l| l.strip_prefix("kestrel: program arena ")?.strip_suffix(" bytes")?.parse().ok())
        .unwrap_or_else(|| panic!("no arena report in:\n{all}"));
    assert!(bytes < 8 << 20, "the program arena kept {bytes} bytes: the mesh data wasn't freed");
}
```

  Check `cli/tests/text_memory.rs` for which stream the report is on and whether `kestrel build` takes `-o` this way; follow it exactly.

- [ ] **Step 5: Run them to see them fail.** `cargo test -p kestrel-cli --test voxels_demo the_viewer` → FAIL (no `voxels.kes`).

- [ ] **Step 6: Write `kestrel/demos/voxels/voxels.kes`:**

```
// The voxel viewer: fly through the bench/voxels3d world in Kestrel's own window.
//
//   kestrel run --release demos/voxels/voxels.kes
//
// Hold the right mouse button and drag to look, WASD to move, Space and
// Shift to go up and down, and the wheel to change speed.
//
// At start a `par for` builds every chunk's mesh data on all cores, and one
// loop copies each chunk to the GPU as a Mesh (like making an ArrayMesh in
// Godot). All of that CPU data lives in an `arena { }` that frees it once
// the meshes are made. Then every frame draws every mesh. It prints the
// build and upload times at start, and the frame times once a second.
use window
use draw
use draw3d
use input
use { Color from draw }
use { Mesh, MeshData from draw3d }
use { Key from input }
use { chunk_mesh, LAYERS from world }
use { Flyer, view from fly }

// Chunks across and deep: 8 is the benchmark's world (463,749 faces); try 16
// for a stress run.
let WIDTH = 8
let SEED = 1337

let SCREEN_WIDTH = 1280
let SCREEN_HEIGHT = 720
let SKY = Color(0.55, 0.75, 0.95)
let TEXT = Color(0.05, 0.05, 0.1)

// Seconds between the printed lines.
let REPORT = 1.0

struct Chunk {
    x: int
    y: int
    z: int
    data: MeshData
}

// Rounds to two decimals for printing.
pure two(x: float) -> float {
    round(x * 100) / 100
}

main() {
    window.open(SCREEN_WIDTH, SCREEN_HEIGHT, "Kestrel voxels")

    // The GPU meshes outlive the arena below; a Mesh holds only its GPU id and counts.
    var meshes = list[Mesh]()
    var chunk_count = 0
    var faces = 0
    var build_ms = 0.0
    var upload_ms = 0.0
    arena {
        var chunks = list[Chunk]()
        for cx from 0 to WIDTH {
            for cy from 0 to LAYERS {
                for cz from 0 to WIDTH {
                    chunks.add(Chunk(x: cx, y: cy, z: cz, data: MeshData()))
                }
            }
        }
        chunk_count = chunks.len
        let build_start = window.ticks_usec()
        par for c in chunks {
            c.data = chunk_mesh(c.x, c.y, c.z, SEED)
        }
        let upload_start = window.ticks_usec()
        for c in chunks {
            // An all-air chunk has nothing to draw, so it gets no mesh.
            if c.data.vertices.len > 0 {
                faces += c.data.indices.len / 6
                meshes.add(draw3d.make_mesh(c.data))
            }
        }
        let upload_end = window.ticks_usec()
        build_ms = float(upload_start - build_start) / 1000
        upload_ms = float(upload_end - upload_start) / 1000
    }
    print("voxels world {WIDTH} chunks {chunk_count} meshes {meshes.len} faces {faces} build {two(build_ms)} ms upload {two(upload_ms)} ms")

    // Start outside the world's corner, looking in and down.
    var flyer = Flyer(pos: vec3(-24, 100, -24), yaw: 45, pitch: -30)
    var last_mouse = input.mouse()

    // Sums over the current report interval, and the averages the last one printed.
    var frames = 0
    var draw_sum = 0.0
    var interval_start = 0.0
    var draw_ms = 0.0
    var frame_ms = 0.0
    var fps = 0.0

    while window.frame() {
        let mouse = input.mouse()
        let turn = if input.held(Key.MouseRight) { mouse - last_mouse } else { vec2(0, 0) }
        last_mouse = mouse
        let move = vec3(input.axis(Key.A, Key.D), input.axis(Key.Shift, Key.Space), input.axis(Key.S, Key.W))
        flyer.fly(move, turn, input.wheel(), window.delta())

        let draw_start = window.time()
        draw.clear(SKY)
        draw3d.camera(view(flyer))
        for m in meshes {
            draw3d.mesh(m, vec3(0, 0, 0), draw.WHITE)
        }
        let draw_end = window.time()

        // The overlay: a draw.* call goes back to 2D on its own.
        draw.text("{two(fps)} fps   {two(frame_ms)} ms a frame   {faces} faces in {meshes.len} meshes", vec2(10, 10), 20, TEXT)
        draw.text("right mouse drag to look   WASD move   Space/Shift up and down   wheel speed ({two(flyer.speed)})", vec2(10, SCREEN_HEIGHT - 28), 20, TEXT)

        frames += 1
        draw_sum += draw_end - draw_start
        let elapsed = window.time() - interval_start
        if elapsed >= REPORT {
            draw_ms = draw_sum / frames * 1000
            frame_ms = elapsed / frames * 1000
            fps = frames / elapsed
            print("voxels {WIDTH} meshes {meshes.len} faces {faces} draw {two(draw_ms)} ms frame {two(frame_ms)} ms fps {two(fps)}")
            frames = 0
            draw_sum = 0.0
            interval_start += elapsed
        }
    }
}
```

  If the checker refuses the build shape (for example `c.data = chunk_mesh(...)` in the `par for`, or `meshes.add(...)` inside the `arena { }`), stop and report the exact error with status NEEDS_CONTEXT rather than restructuring: the shape is the spec's.

- [ ] **Step 7: Run the viewer tests.** `cargo test -p kestrel-cli --test voxels_demo` → all pass. If the arena test's bytes are close to its bound, report the number; check once by hand (in a scratch copy, not committed) that removing the `arena { }` makes the report tens of MB, so the test can see the difference.

- [ ] **Step 8: Write the GPU smoke case.** Create `cli/tests/window_smoke_voxels.rs`, following `cli/tests/window_smoke_mesh.rs` (its doc comment, `mod common;`, the `Case` and run helpers, JIT and exe). One case, `voxel_column`, in a 64×64 hidden window: one column of chunks seen from straight above, with `world.kes` copied next to the program as an asset (`("../../demos/voxels/world.kes", "world.kes")`, a path relative to `kestrel/tests/programs/`).

```
use window
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
```

  Checks:
  - `stdout`: `"frames 3\n"`.
  - `inked`: `(28, 28, 8, 8, "the column seen from above")`.
  - `pixels`: `(1, 1, BLACK, "beside the column")` and `(62, 62, BLACK, "beside the column")`. (From 300 blocks up with a 75° fov, the 32-block column covers only the middle few pixels.)
  - One exact pixel inside a top face: run the case once, read its screenshot's centre region with `kestrel_window::png_pixels`, pick a pixel that is exactly one of the full-brightness band colours (STONE `[133, 133, 140, 255]`, DIRT `[120, 87, 56, 255]`, GRASS `[84, 158, 69, 255]`, SNOW `[237, 242, 247, 255]`), check that the pixels around it have the same colour (so it isn't on an edge), and pin it with a comment naming the band. Top faces seen from above are drawn at full brightness only if they are wound the right way; a culled top would show a side or nothing instead.

  Run `cargo test -p kestrel-cli --test window_smoke_voxels` (hidden windows only): it passes under the JIT and as an exe.

- [ ] **Step 9: Run the suite and clippy.** Commit "Add the voxel viewer: a flying camera over the benchmark's world".

---

### Task 3: Measure the viewer and write `results.md`

**Files:**
- Create: `kestrel/demos/voxels/results.md`, and `kestrel/demos/voxels/screenshot.png` if it is under 500 KB
- Modify: `kestrel/bench/particles/measure.py` (read the viewer's lines too)

**Interfaces:**
- Consumes: the viewer's two line formats (Task 2): `voxels world W chunks C meshes M faces F build B ms upload U ms` and `voxels W meshes M faces F draw D ms frame T ms fps P`.

- [ ] **Step 1: Let `measure.py` read the viewer.** Add a second line pattern for `voxels W meshes M faces F draw D ms frame T ms fps P` (columns `draw`, `frame`, `fps`), and collect the start line's `build` and `upload` (one value a run, summarised as a median with the range over the runs). Keep its particles behaviour and output unchanged, keep the "held to 60 fps" warning and `--retry-capped` working for both, and update its docstring. Check it by running it on the particles demo once (a measured copy, hidden) and on the viewer once.

- [ ] **Step 2: Measured copies.** In the scratch folder (not committed), copy `voxels.kes`, `world.kes` and `fly.kes`, and make one copy with `WIDTH = 8` and one with `WIDTH = 16`. The camera stays at its start, so every run draws the same view (a hidden window gets no input). Build the CLI once (`cargo build --release -p kestrel-cli` in `kestrel/`) and use `kestrel/target/release/kestrel.exe`.

- [ ] **Step 3: Measure** with `measure.py`, every command with `KESTREL_HIDDEN=1`:
  - `kestrel run --release` and a `kestrel build --release` exe of each copy, interleaved in rounds (at least 3 rounds; more for the start-line times, which come once a run).
  - The method of `demos/particles/results.md`: about 11 s a run, the first per-second line dropped, medians with ranges in brackets.
  - Drop and redo runs whose frame is pinned at 16.67 ms (the cap seen once on this machine).
  - Also: the release exe's size (`kestrel build --release` of `voxels.kes`); the largest chunk's vertex count (does any chunk go over 65,536 vertices and split into parts?); and `KESTREL_THREADS=1` against all threads for the build time at `WIDTH = 8`.
  - Find out what limits the frame: compare `draw` (the CPU side of the mesh calls) with `frame` (which includes waiting for the GPU at `window.frame()`), and try `WIDTH = 16` to see how both scale with the face count.

- [ ] **Step 4: A screenshot, to check the picture.** In a scratch copy at `WIDTH = 8`, add `if frames_seen == 30 { window.screenshot("viewer.png") }` (count frames in a new `var`) and run it hidden. Look at `viewer.png` (the Read tool shows images): terrain in four bands, faces shaded by side, nothing inside-out, the overlay readable. If something looks wrong, stop and report it with the picture's path before writing results. If the PNG is under 500 KB, copy it to `kestrel/demos/voxels/screenshot.png` and reference it in `results.md`.

- [ ] **Step 5: Write `kestrel/demos/voxels/results.md`,** in the style of `demos/particles/results.md` (what the numbers are, setup, results, where the time goes):
  - build (`par for`) and upload times, and draw, frame and fps, at `WIDTH` 8 and 16, JIT and exe;
  - whether `WIDTH = 8` meets the spec's 60 fps (a frame under 16.67 ms), said plainly, with the note that hidden windows aren't capped and a visible one shows at most 60;
  - the exe size, the meshes' count and largest chunk, and the one-thread build time;
  - what limits the frame, from the measurements, and anything that missed.

- [ ] **Step 6: Run the suite.** Commit "Measure the voxel viewer".

---

### Task 4: The docs

**Files:**
- Modify: `docs/superpowers/specs/2026-10-10-3d-textures-design.md`, `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md`, `docs/superpowers/specs/2026-10-10-window-design.md`, `docs/superpowers/plans/2026-10-09-core-2a-followups.md`, `README.md`, `kestrel/window/src/natives_mesh.rs` (one comment)

**Interfaces:**
- Consumes: the handoff's plan 2 section (`docs/superpowers/plans/2026-10-10-handoff.md`: "Plan 2 rulings made on the user's behalf" and "Follow-ups found in plan 2"), `kestrel/bench/particles/results.md` (the Rust comparison), and Task 3's `results.md`.

- [ ] **Step 1: The 3D spec.** In `2026-10-10-3d-textures-design.md`:
  - In "`draw`: textures, sprites and bulk rectangles", make the negative-size bullet say exactly what was built: a negative `size` covers the rectangle it spans back from `pos`, mirrored; a negative `region` size reads the same region (at `region.pos`, with the size's absolute value), mirrored; both negative cancel.
  - Add a section "As built (plans 2 and 3)" after "How it's built", listing where the build settled what the spec left open or said differently, one bullet each: the map error names `draw.Texture`; `window.screenshot` saves only `.png` paths; image files are read and written through Rust so non-ASCII folders work; `draw.clear` draws the pending batch first, so it erases everything drawn before it (also before the first frame); `draw3d.cube` uses the size's absolute values, a `sphere` with radius 0 or less draws nothing, and a zero `up` gets the "up" message; an empty mesh counts as having uvs; `make_mesh`'s size-limit errors; one default material shared by `draw.rects` and meshes; hidden windows don't wait for vsync; the voxel viewer's results (one line, pointing at `kestrel/demos/voxels/results.md`).
- [ ] **Step 2: The syntax spec.** In `2026-10-09-kestrel-syntax-design.md`, "Optional values": replace `let tex = load("grass.png") else { return }` (texture loading fails with a runtime error now, not an optional) with a map lookup, which does give an optional: `let hp = scores["ana"] else { return }`. Check the bullets under it still read right.
- [ ] **Step 3: The window spec.** In `2026-10-10-window-design.md`, the frame-loop paragraph on constants (the bullet starting "Using an array, list or map constant inside the loop also allocates a fresh copy at each use"): replace it with one sentence saying heap constants are now built once before `main` and read in place (3D spec, "Language additions"), so the palette pattern allocates nothing. Keep the `let label` bullet before it, which is still true.
- [ ] **Step 4: The follow-ups.** In `2026-10-09-core-2a-followups.md`:
  - Add "## Found while building plan 2 of the 3D work (2026-10-11)" with every item of the handoff's "Follow-ups found in plan 2" list, the camera-check edge cases, and the Rust comparison's findings: the particle step's compute is 1.5x Rust's (3.4x with `pow` hoisted) and the copy loop 2-3x, which are inputs for the stronger backend; `draw.rects`'s upload mostly waits for the driver, and sending fewer bytes is the fix.
  - Add "## Found while building plan 3 of the 3D work (2026-10-11)" with what Task 3's measurements found that isn't fixed (if nothing, say so).
  - Mark as done any older item plans 1-3 fixed (for example the window section's screenshot-error rewording, or the heap-constants hoisting decision): strike it through or move it under a "Done" line, the way the file already records resolved items.
- [ ] **Step 5: The README.** In the root `README.md`'s "A window" section: mention `draw3d`, textures and `draw.rects` in the module list; update the particles sentence (200,000 dots now draw in about 4.6 ms with one `draw.rects` call; point at `kestrel/demos/particles/results.md` and the Rust comparison in `kestrel/bench/particles/results.md`); and add the voxel viewer with its run command (`target/release/kestrel run --release demos/voxels/voxels.kes`), its controls, and its headline number from Task 3. Point at the 3D spec for the API.
- [ ] **Step 6: The comment.** `kestrel/window/src/natives_mesh.rs` near line 17 says a headless window keeps only the counts; the registry keeps `has_uvs` and `indexed` now (the counts live in the program's `Mesh`). Fix the sentence.
- [ ] **Step 7: Run the suite and clippy** (the comment change touches Rust). Commit "Bring the specs, follow-ups and README up to date with the 3D work".

---

## After the last task

Run the whole-branch review (subagent-driven development's final review) on this plan's commits, fix what it finds, push. The controller then updates the handoff: sub-project 4b is done, and the next is the user's choice (the order they set was a stronger backend, sub-project 5, after 4b).
