# Proposal: the voxel engine, version 1 (2026-10-25)

**Status: approved 2026-10-25** ("Approved as written, block-mesh for round 1"): every pick as written, and `block-mesh` as round 1's opponent. Next: a spec in `docs/superpowers/specs/` and a plan in `docs/superpowers/plans/`, both reviewed by subagents, then the build. Each part shows a pick, what was rejected, and (at the end) the one question for the user. Architecture is Claude's call and can be overruled; anything that would need new syntax stops and comes back to the user as its own proposal.

## Why an engine, and what it is for

The agreed order is generics (done 2026-10-25), then a voxel engine written in Kestrel as the language's real test, then libraries grown out of the engine, then `await` and hot reload by what the engine shows it needs. So the engine has two jobs:

1. **Be a real program**: something you fly and walk around in, dig and build in, that streams its world as you move. Not a benchmark loop, and not a small "evaluation game".
2. **Find the language's gaps** by needing things. Every time the engine works around Kestrel, the workaround and the general fix go in a findings list. That list picks the next rounds (`await`, hot reload, and whatever else turns up).

Voxels are one use of Kestrel, not its design centre: fixes the engine asks for should be general compiler or library work, and a proposal that leans voxel-specific says so.

## Pick 1: what version 1 does

**A streaming, editable, walkable world in the standalone window.**

- **Endless world (sideways), streamed around you.** Chunks of 32x32x32 blocks, a fixed number of chunks high (4 to start, 128 blocks, like the frozen world), generated and meshed as you move and dropped when you leave. View distance is a constant (8 chunks to start).
- **Terrain** from noise, starting with the frozen world's density (hills, caves, overhangs), and its colours: grass, dirt, stone, snow, per-corner ambient occlusion.
- **Dig and build.** Look at a block, left click breaks it, right click places one. The chunk and any neighbour it touches are re-meshed at once.
- **Walk or fly.** Walking has gravity, jumping and collision with blocks (like a `CharacterBody3D` with `move_and_slide`, but against the voxel grid); a key toggles the free-fly camera the viewer has now.
- **Only what you can see is drawn**: chunk meshes outside the camera's view are skipped.
- **A headless scripted mode** for tests and timing: a fixed camera path and edit list, a frame count, printed stats. No window pops up (`KESTREL_HEADLESS` / `KESTREL_HIDDEN`, as for every window program).

**Not in version 1:** textures (vertex colours, as now), light that spreads (only ambient occlusion), water, saving (Kestrel has no file reading or writing yet), mobs, greedy meshing (that is the race's second round, Pick 5), more than one block type you can place.

**Rejected:**
- **Port the viewer onto libraries and stop there.** Too small: it would prove generics compile in a bigger program, and little else. Streaming, editing and collision are where the gaps are.
- **A full Minecraft-like feature set** (textures, light spreading, water, saves, mobs). Months of work, and several parts need features Kestrel doesn't have yet (files, `await`). Better to let version 1 show which ones matter most.
- **The Rust race first, as kernels.** That is benchmark work, not an engine; the race is a measurement of the engine (Pick 5).

## Pick 2: how it relates to the frozen world and the viewer

- **`kestrel/bench/voxels3d/` stays frozen.** It remains the benchmark the compiler is measured on, byte for byte.
- **The engine starts from the same density and the same culled mesher with ambient occlusion,** rewritten onto the new libraries (`Grid[T]`, `noise`).
- **A parity check ties the two together:** in headless mode the engine can build the frozen world's region (chunks 0-7 x 0-3 x 0-7, seed 1337) and must count exactly **463,749 faces**, as the benchmark does. Its build time against the frozen `main.kes` (`--release`, taking turns) is the test of "generic code runs like hand-written code" in a real program, not just a 20-line loop. The target is within 5%; if it misses, the gap is a compiler finding.
- **`kestrel/demos/voxels/` (the viewer) stays as it is** for now. Once the libraries are settled it can move onto them, which would be their second user.

**Rejected:** editing the frozen world to share code with the engine (it would stop being a fixed benchmark); measuring the engine against nothing (we would not know whether generics cost anything in real code).

## Pick 3: which parts become library modules

The engine lives in a new folder, **`kestrel/voxel/`**, as flat files, because a Kestrel module is found next to the file that uses it. A file counts as a library when it imports nothing of the engine's and has its own tests. The candidates:

| Module | What it is | Generic? |
|---|---|---|
| `grid` | `Grid[T]`: a 3D block of values with a size, index maths, `get`/`set`, in-bounds tests and fill. Chunk blocks are a `Grid[u8]` | yes |
| `noise` | Hash, value noise in 2D and 3D, octaves (fractal sums). The frozen world's noise, made reusable | no (floats only) |
| `ring` | `Ring[T]`: a first-in-first-out queue with a fixed capacity, for the work queues (chunks to generate, mesh, upload) | yes |
| `pool` | `Pool[T]`: fixed slots reused through a free list, with handles that know when their slot was reused (index plus generation). Chunk storage is a pool, so walking around reuses chunk memory instead of allocating | yes |
| `ray` | Stepping a ray through a grid cell by cell (the classic voxel "DDA"), with the solid test passed in as a function | no |
| `collide` | A box moving through a grid of solid cells: slide along walls, stand on floors | no |

Engine-only files: `blocks` (block ids and colours), `chunk`, `gen` (terrain), `mesher`, `stream` (what to load, generate, mesh and drop each frame), `player`, `main`.

**Where they go after version 1:** sharing them with another program needs a way to find modules somewhere other than next to the file (a library folder, or shipping them with Kestrel like `draw`). That is an import question, so it is the user's and comes as its own small proposal, once a second program (the viewer) actually wants them.

**Rejected:**
- **Designing a standard library up front.** You asked for libraries grown out of the engine; designed in a vacuum, they get the wrong shape.
- **Shipping them inside Kestrel now** (like `draw` and `window`). It would freeze an API that only one program has used.
- **Copying them into each program** that wants them. Fine as a stopgap, not as the answer.

## Pick 4: what it should prove about the language

Each of these is a measured claim, with the result written down whether Kestrel wins or loses:

1. **Generics cost nothing in a real program**: the parity build (Pick 2) within 5% of the hand-written frozen world.
2. **Walking around allocates nothing once warmed up**: chunk memory comes from the pool, mesh building uses a per-frame `arena { }`. Checked by the working set staying flat over a long scripted flight, and with `@prove(no_alloc)` on the inner loops where the contracts can say it.
3. **Hot loops have no checks left**: `kestrel check --report` on the mesher, the noise and the ray, with `@prove(no_checks)` where they can be proven, and a short reason for every check that stays.
4. **It is smooth**: 60 frames a second while streaming at full speed, with the worst frame (not only the average) reported.
5. **Edits feel instant**: the time from a click to the new mesh being drawn.
6. **The code reads well**: no file over about 400 lines, and every workaround recorded in the findings list.

## Pick 5: what to measure, and the race against Rust

**Measured** (headless or hidden window, medians and ranges, taking turns when two things are compared, as in the other benchmarks):

- Per chunk: generate time, mesh time, upload time; faces and vertices per chunk.
- Streaming: chunks ready per second at full flying speed; frame time while streaming (median, 99th percentile, worst); how long until the view distance is filled from a standing start.
- Edits: click to new mesh drawn, for a block in the middle of a chunk and on a chunk corner (which re-meshes up to 4 chunks).
- Memory: working set at start, after filling the view, and after a 5-minute scripted flight. (The runtime keeps freed arena memory, a known gap; the pool is meant to keep the engine clear of it, and the flight shows whether it does.)
- The parity build against the frozen world (Pick 2).
- Compile time of the engine and the editor's `kestrel check` time per edit.

**The race against Rust, round 1:** the meshing step, on identical chunk data, against **the `block-mesh` crate** (a widely used, optimized Rust voxel mesher: its `visible_block_faces` is the same algorithm as ours, its `greedy_quads` merges faces into big rectangles). Same face count checked on both sides. Plus the existing straight Rust port of the frozen world's build (Kestrel's tuned copy ran at 0.92 of its time on 2026-10-24). **Round 2, after version 1:** a greedy mesher in Kestrel against the fastest known one (`binary-greedy-meshing`, bitmask tricks); that is where "working smarter" has to win.

**Rejected:** racing a whole Rust game engine (Veloren, a Bevy voxel game): too much differs (rendering, threads, features) for the number to mean anything about the language. Racing our own Rust port only: it is a race against ourselves; the user wanted an existing, optimized engine.

## What the engine will probably hit (expected findings)

These are known from the generics spec and the handoff; the engine will show how much each one hurts:

- **Map keys are only `int`, `i64`, `u8`, `bool` and `string`,** so a chunk's (x, y, z) has to be packed into one `i64`. A struct key would be the general fix.
- **An owning type (a GPU `Mesh`) can't be `T`,** so meshes can't live in a `Pool[T]`; they go in a plain list indexed by slot.
- **`array[T](n)` is refused in generic code,** so `Grid[T]` holds a `list[T]` filled with `add`. If that is slower than a native array, the fix is likely an `array[T](n, fill)`-style form (that would be syntax, so the user's).
- **No streaming `par for`**: work is done in per-frame batches. What that costs in hitches is the measured case for `await` or jobs.
- **No mouse capture** for walking: small `input`/`window` natives get added as the engine needs them, each one recorded.
- **No files**, so no saving yet.

## The question for you

**Is `block-mesh` the right opponent for round 1 of the race, or do you have a particular Rust voxel engine in mind?** Default if you don't mind: `block-mesh` for round 1 (same algorithm, so it measures the language), `binary-greedy-meshing` for round 2 (the best-known algorithm, so it measures whether Kestrel can win by working smarter).

Everything else above goes ahead as written unless you say otherwise.
