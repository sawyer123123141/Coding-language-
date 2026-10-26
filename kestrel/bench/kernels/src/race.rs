//! The Rust side of the race against Kestrel's block-face meshing (E26): `kestrel/voxel/race.kes`
//! and the `block-mesh` crate on the same input, one thread.
//!
//! The input is the frozen world's 256 chunks (8 x 4 x 8, in the frozen loops' order: chunk c is
//! column (c / 32, c % 8), layer c / 8 % 4) as padded 34^3 solid flags, seed 1337. They are made
//! once, before the timing, from a straight port of the frozen world's noise (`f32`, the same
//! order of operations as `voxels3d.kes`; it must give 463,749 faces).
//!
//! Kernel 1 (`race_faces`) is `block_mesh::visible_block_faces` over a `ConstShape3u32<34, 34,
//! 34>` with `RIGHT_HANDED_Y_UP_CONFIG.faces` into a `UnitQuadBuffer` that is `reset()` (its six
//! `Vec`s cleared, their memory kept) for each chunk. Kernel 2 (`race_arrays`) is that and then,
//! for each face of each group, `quad_mesh_positions`, `quad_mesh_normals` and
//! `quad_mesh_indices` appended to three `Vec`s that are cleared and reused for each chunk.
//!
//! Timed like the other modes: `iters` passes over the chunks per repetition (median of `reps`
//! after one warm-up), ns per chunk. The first pass of the first repetition checks the face
//! total and, for kernel 2, the vector lengths (14 entries a face), and panics if they are wrong.
//! `--verify` prints the check value `race_check.kes` prints for the Kestrel side.
//!
//! `race_faces_plain` and `race_arrays_plain` are not part of the race: they are `race.kes`'s
//! kernels written out in Rust (the same loops over y, z, x with checked indexing into a flat
//! `Vec<u8>` laid out as `Grid[u8]` is, six `Vec<u32>`s of packed faces, the same corners), to
//! tell how much of a gap to `block-mesh` is the crate's code and how much is the compiler's.

use std::hint::black_box;
use std::time::Instant;

use block_mesh::ndshape::{ConstShape, ConstShape3u32};
use block_mesh::{
    visible_block_faces, UnitQuadBuffer, UnorientedQuad, Voxel, VoxelVisibility, RIGHT_HANDED_Y_UP_CONFIG,
};

const SIZE: i32 = 32;
const PAD: i32 = SIZE + 2;
const LAYERS: i32 = 4;
const HEIGHT: i32 = LAYERS * SIZE;
const SEED: i32 = 1337;
const CHUNKS: usize = 256;
const FACES_ALL: usize = 463_749;

type Shape = ConstShape3u32<34, 34, 34>;

/// A block as `block-mesh` sees it: 0 is air, anything else is opaque.
#[derive(Clone, Copy)]
struct Solid(u8);

impl Voxel for Solid {
    #[inline]
    fn get_visibility(&self) -> VoxelVisibility {
        if self.0 == 0 {
            VoxelVisibility::Empty
        } else {
            VoxelVisibility::Opaque
        }
    }
}

// The noise, as `bench/voxels3d/voxels3d.kes` has it (and `C:\kt\voxtuned\rust\voxels3d.rs`).
fn hash(x: i32, y: i32, z: i32, seed: i32) -> i32 {
    let mut h = ((x as i64) * 73856093) ^ ((y as i64) * 19349663) ^ ((z as i64) * 83492791) ^ ((seed as i64) * 1610612741);
    h &= 0x7FFFFFFF;
    h = ((h ^ (h >> 13)) * 0x5BD1E995) & 0x7FFFFFFF;
    (h ^ (h >> 15)) as i32
}

fn lattice(x: i32, y: i32, z: i32, seed: i32) -> f32 {
    (hash(x, y, z, seed) & 0xFFFF) as f32 / 65535.0
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn octave(x: i32, y: i32, z: i32, shift: i32, seed: i32) -> f32 {
    let mask = (1 << shift) - 1;
    let scale = (1 << shift) as f32;
    let (cx, cy, cz) = (x >> shift, y >> shift, z >> shift);
    let fx = smooth((x & mask) as f32 / scale);
    let fy = smooth((y & mask) as f32 / scale);
    let fz = smooth((z & mask) as f32 / scale);
    let x00 = mix(lattice(cx, cy, cz, seed), lattice(cx + 1, cy, cz, seed), fx);
    let x10 = mix(lattice(cx, cy + 1, cz, seed), lattice(cx + 1, cy + 1, cz, seed), fx);
    let x01 = mix(lattice(cx, cy, cz + 1, seed), lattice(cx + 1, cy, cz + 1, seed), fx);
    let x11 = mix(lattice(cx, cy + 1, cz + 1, seed), lattice(cx + 1, cy + 1, cz + 1, seed), fx);
    mix(mix(x00, x10, fy), mix(x01, x11, fy), fz)
}

fn density(x: i32, y: i32, z: i32, seed: i32) -> f32 {
    let n = octave(x, y, z, 5, seed) * 0.5 + octave(x, y, z, 4, seed + 1) * 0.3 + octave(x, y, z, 3, seed + 2) * 0.2;
    n - 0.5 - (y - HEIGHT / 2) as f32 / (HEIGHT * 2) as f32
}

/// The padded solid flags of chunk `c`, laid out as block-mesh's shape wants (x fastest, then y,
/// then z); the voxel set is the one `race.kes` builds, which lays it out y, z, x instead.
fn chunk(c: usize) -> Vec<Solid> {
    let (cx, cy, cz) = ((c / 32) as i32, (c / 8 % 4) as i32, (c % 8) as i32);
    let mut out = vec![Solid(0); Shape::SIZE as usize];
    for z in 0..PAD {
        for y in 0..PAD {
            for x in 0..PAD {
                if density(cx * SIZE + x - 1, cy * SIZE + y - 1, cz * SIZE + z - 1, SEED) > 0.0 {
                    out[Shape::linearize([x as u32, y as u32, z as u32]) as usize] = Solid(1);
                }
            }
        }
    }
    out
}

/// Kernel 1 for one chunk: its faces into `buffer` (reset first). Returns the face count.
#[inline(never)]
fn find_faces(voxels: &[Solid], buffer: &mut UnitQuadBuffer) -> usize {
    buffer.reset();
    visible_block_faces(voxels, &Shape {}, [0; 3], [PAD as u32 - 1; 3], &RIGHT_HANDED_Y_UP_CONFIG.faces, buffer);
    buffer.num_quads()
}

/// Kernel 2's second half for one chunk: the arrays of `buffer`'s faces, in the vectors
/// (cleared first).
#[inline(never)]
fn build_arrays(buffer: &UnitQuadBuffer, positions: &mut Vec<[f32; 3]>, normals: &mut Vec<[f32; 3]>, indices: &mut Vec<u32>) {
    positions.clear();
    normals.clear();
    indices.clear();
    for (group, face) in buffer.groups.iter().zip(RIGHT_HANDED_Y_UP_CONFIG.faces.iter()) {
        for quad in group.iter() {
            let quad: UnorientedQuad = (*quad).into();
            indices.extend_from_slice(&face.quad_mesh_indices(positions.len() as u32));
            positions.extend_from_slice(&face.quad_mesh_positions(&quad, 1.0));
            normals.extend_from_slice(&face.quad_mesh_normals());
        }
    }
}

/// One pass of kernel 1 over every chunk: the face total.
#[inline(never)]
fn faces_pass(chunks: &[Vec<Solid>], buffer: &mut UnitQuadBuffer) -> usize {
    let mut total = 0;
    for voxels in chunks {
        total += find_faces(black_box(voxels), buffer);
    }
    total
}

/// One pass of kernel 2: the face total and the vectors' total length after each chunk.
#[inline(never)]
fn arrays_pass(
    chunks: &[Vec<Solid>],
    buffer: &mut UnitQuadBuffer,
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
) -> (usize, usize) {
    let (mut total, mut size) = (0, 0);
    for voxels in chunks {
        total += find_faces(black_box(voxels), buffer);
        build_arrays(buffer, positions, normals, indices);
        size += positions.len() + normals.len() + indices.len();
    }
    (total, size)
}

/// The check value of `race_check.kes` for kernel 1: the packed faces of every chunk, weighted by
/// their side, summed below 2^24. Kestrel's packed int is `x | y << 6 | z << 12`.
fn faces_checksum(chunks: &[Vec<Solid>], buffer: &mut UnitQuadBuffer) -> i32 {
    let mut sum = 0i32;
    for voxels in chunks {
        find_faces(voxels, buffer);
        for (side, group) in buffer.groups.iter().enumerate() {
            for q in group {
                let p = (q.minimum[0] | q.minimum[1] << 6 | q.minimum[2] << 12) as i32;
                sum = (sum + p * (side as i32 + 1)) & 0xFFFFFF;
            }
        }
    }
    sum
}

/// Adds one chunk's arrays to a kernel 2 check value (see `arrays_checksum`).
fn add_arrays(mut sum: i32, positions: &[[f32; 3]], normals: &[[f32; 3]], indices: &[u32]) -> i32 {
    for k in 0..positions.len() {
        let v = positions[k];
        let w = normals[k];
        let corner = (k & 3) as i32 + 1;
        sum = (sum + corner * (v[0] as i32 + 3 * v[1] as i32 + 7 * v[2] as i32)) & 0xFFFFFF;
        sum = (sum + (w[0] as i32 + 2 * w[1] as i32 + 4 * w[2] as i32) * corner) & 0xFFFFFF;
    }
    for (k, &i) in indices.iter().enumerate() {
        sum = (sum + i as i32 * (k as i32 % 6 + 1)) & 0xFFFFFF;
    }
    sum
}

/// The check value for kernel 2: every position (weighted by its corner and its coordinates),
/// normal and index (weighted by its place in the face's six), summed below 2^24.
fn arrays_checksum(chunks: &[Vec<Solid>], buffer: &mut UnitQuadBuffer) -> i32 {
    let (mut positions, mut normals, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    let mut sum = 0i32;
    for voxels in chunks {
        find_faces(voxels, buffer);
        build_arrays(buffer, &mut positions, &mut normals, &mut indices);
        sum = add_arrays(sum, &positions, &normals, &indices);
    }
    sum
}

/// `race.kes`'s chunk flags: the same voxels as `chunk`, laid out y, z, x (`Grid[u8]`'s order).
fn chunk_plain(c: usize) -> Vec<u8> {
    let (cx, cy, cz) = ((c / 32) as i32, (c / 8 % 4) as i32, (c % 8) as i32);
    let mut out = vec![0u8; Shape::SIZE as usize];
    let mut i = 0;
    for y in 0..PAD {
        for z in 0..PAD {
            for x in 0..PAD {
                if density(cx * SIZE + x - 1, cy * SIZE + y - 1, cz * SIZE + z - 1, SEED) > 0.0 {
                    out[i] = 1;
                }
                i += 1;
            }
        }
    }
    out
}

/// `race.kes`'s `find_faces`, in Rust: bounds-checked reads, six `Vec`s pushed to.
#[inline(never)]
fn find_faces_plain(world: &[u8], sides: &mut [Vec<u32>; 6]) {
    const P: usize = PAD as usize;
    for y in 0..SIZE as usize {
        for z in 0..SIZE as usize {
            for x in 0..SIZE as usize {
                let i = ((y + 1) * P + z + 1) * P + x + 1;
                if world[i] == 1 {
                    let p = ((x + 1) | (y + 1) << 6 | (z + 1) << 12) as u32;
                    if world[i - 1] == 0 {
                        sides[0].push(p);
                    }
                    if world[i - P * P] == 0 {
                        sides[1].push(p);
                    }
                    if world[i - P] == 0 {
                        sides[2].push(p);
                    }
                    if world[i + 1] == 0 {
                        sides[3].push(p);
                    }
                    if world[i + P * P] == 0 {
                        sides[4].push(p);
                    }
                    if world[i + P] == 0 {
                        sides[5].push(p);
                    }
                }
            }
        }
    }
}

/// `race.kes`'s `build_arrays`, in Rust, with `Vec<[f32; 3]>` for the positions and normals.
#[inline(never)]
fn build_arrays_plain(sides: &[Vec<u32>; 6], pos: &mut Vec<[f32; 3]>, nor: &mut Vec<[f32; 3]>, idx: &mut Vec<u32>) {
    // For each side: its normal, the offsets of its corners b, c and d from a, the offset of a
    // from the block, and whether its triangles wind a b c / b d c (`up`).
    type Side = ([f32; 3], [[f32; 3]; 3], [f32; 3], bool);
    const SIDES: [Side; 6] = [
        ([-1.0, 0.0, 0.0], [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [0.0, 1.0, 1.0]], [0.0, 0.0, 0.0], true),
        ([0.0, -1.0, 0.0], [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0]], [0.0, 0.0, 0.0], false),
        ([0.0, 0.0, -1.0], [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]], [0.0, 0.0, 0.0], false),
        ([1.0, 0.0, 0.0], [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [0.0, 1.0, 1.0]], [1.0, 0.0, 0.0], false),
        ([0.0, 1.0, 0.0], [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0]], [0.0, 1.0, 0.0], true),
        ([0.0, 0.0, 1.0], [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]], [0.0, 0.0, 1.0], true),
    ];
    for (side, &(n, d, o, up)) in sides.iter().zip(SIDES.iter()) {
        for &p in side {
            let q = [(p & 63) as f32 + o[0], (p >> 6 & 63) as f32 + o[1], (p >> 12) as f32 + o[2]];
            let base = pos.len() as u32;
            pos.push(q);
            for k in d {
                pos.push([q[0] + k[0], q[1] + k[1], q[2] + k[2]]);
            }
            nor.extend_from_slice(&[n; 4]);
            idx.extend_from_slice(&if up {
                [base, base + 1, base + 2, base + 1, base + 3, base + 2]
            } else {
                [base, base + 2, base + 1, base + 1, base + 2, base + 3]
            });
        }
    }
}

/// The `_plain` modes (see the top of this file): ns per chunk, like `race`.
fn race_plain(arrays: bool, n: usize, iters: usize, reps: usize, verify: bool) {
    let chunks: Vec<Vec<u8>> = (0..n).map(chunk_plain).collect();
    let mut sides: [Vec<u32>; 6] = Default::default();
    let (mut pos, mut nor, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    if verify {
        let mut sum = 0i32;
        for world in &chunks {
            sides.iter_mut().for_each(Vec::clear);
            find_faces_plain(world, &mut sides);
            if arrays {
                pos.clear();
                nor.clear();
                idx.clear();
                build_arrays_plain(&sides, &mut pos, &mut nor, &mut idx);
                sum = add_arrays(sum, &pos, &nor, &idx);
            } else {
                for (side, group) in sides.iter().enumerate() {
                    for &p in group {
                        sum = (sum + p as i32 * (side as i32 + 1)) & 0xFFFFFF;
                    }
                }
            }
        }
        println!("rust race_{}_plain: check value {sum}", if arrays { "arrays" } else { "faces" });
        return;
    }
    let (mut per, mut faces) = (Vec::new(), 0);
    for rep in 0..=reps {
        let start = Instant::now();
        for _ in 0..iters {
            faces = 0;
            for world in black_box(&chunks) {
                for s in sides.iter_mut() {
                    s.clear();
                }
                find_faces_plain(black_box(world), &mut sides);
                faces += sides.iter().map(Vec::len).sum::<usize>();
                if arrays {
                    pos.clear();
                    nor.clear();
                    idx.clear();
                    build_arrays_plain(&sides, &mut pos, &mut nor, &mut idx);
                }
            }
            black_box((&sides, &pos, &nor, &idx));
        }
        let t = start.elapsed().as_secs_f64();
        if rep == 0 && n == CHUNKS {
            assert_eq!(faces, FACES_ALL, "the plain port must find the frozen world's faces");
        }
        if rep > 0 {
            per.push(t / (n * iters) as f64 * 1e9);
        }
    }
    per.sort_by(f64::total_cmp);
    println!(
        "rust race_{}_plain: {:.2} ns per item (median; {:.2}-{:.2}), n {n}, iters {iters}, reps {reps}, checksum {faces}",
        if arrays { "arrays" } else { "faces" },
        per[per.len() / 2],
        per[0],
        per[per.len() - 1]
    );
}

/// Runs `race_faces` or `race_arrays`: `n` chunks (256 for the frozen world), `iters` passes per
/// timed repetition, `reps` repetitions after a warm-up one.
pub fn race(mode: &str, n: usize, iters: usize, reps: usize, verify: bool) {
    assert!(n <= CHUNKS, "the frozen world has {CHUNKS} chunks");
    if mode.ends_with("_plain") {
        return race_plain(mode == "race_arrays_plain", n, iters, reps, verify);
    }
    let chunks: Vec<Vec<Solid>> = (0..n).map(chunk).collect();
    let arrays = mode == "race_arrays";
    let mut buffer = UnitQuadBuffer::new();
    if verify {
        let sum = if arrays { arrays_checksum(&chunks, &mut buffer) } else { faces_checksum(&chunks, &mut buffer) };
        println!("rust {mode}: check value {sum}");
        return;
    }
    let (mut positions, mut normals, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    let (mut per, mut faces) = (Vec::new(), 0);
    for rep in 0..=reps {
        let start = Instant::now();
        for pass in 0..iters {
            let (total, size) = if arrays {
                arrays_pass(black_box(&chunks), &mut buffer, &mut positions, &mut normals, &mut indices)
            } else {
                (faces_pass(black_box(&chunks), &mut buffer), 0)
            };
            black_box((&buffer, &positions, &normals, &indices));
            faces = total;
            if rep == 0 && pass == 0 {
                if n == CHUNKS {
                    assert_eq!(total, FACES_ALL, "the Rust port must find the frozen world's faces");
                }
                if arrays {
                    assert_eq!(size, total * 14, "4 positions, 4 normals and 6 indices a face");
                }
            }
        }
        let t = start.elapsed().as_secs_f64();
        // The first repetition warms up.
        if rep > 0 {
            per.push(t / (n * iters) as f64 * 1e9);
        }
    }
    per.sort_by(f64::total_cmp);
    println!(
        "rust {mode}: {:.2} ns per item (median; {:.2}-{:.2}), n {n}, iters {iters}, reps {reps}, checksum {faces}",
        per[per.len() / 2],
        per[0],
        per[per.len() - 1]
    );
}
