# The Kestrel voxel engine

A voxel engine written in Kestrel (a Kestrel program, not a Cargo crate): an endless world of
blocks streamed around the player, who walks or flies, breaks and places blocks. Version 1 is
complete. The design, with every change made while building it ("As built"), is in
`docs/superpowers/specs/2026-10-25-voxel-engine-design.md`; the measurements are in
`kestrel/bench/engine/results.md`.

The files are flat because Kestrel finds a module only next to the file that uses it.

## Playing

Run this in `kestrel/` (it opens a 1280 x 720 window):

```
cargo run -q --release -p kestrel-cli -- run --release voxel/main.kes
```

- **WASD** moves; **Space** jumps (walking) or goes up (flying); **Shift** goes down (flying).
- **F** switches between walking and flying. Leaving fly mode inside a block is refused.
- **Mouse wheel** changes the flying speed (2 to 400 blocks a second).
- **Left click** breaks the block under the crosshair (within 6 blocks); **right click** places
  stone in front of it (not inside the player).
- **Looking:** the mouse starts free; hold the right button and drag to look (a right click
  that moves less than 4 pixels places a block instead). **Tab** captures the mouse, as in a
  first-person game: moving it looks, and Tab frees it again.

The player starts flying above column (0, 0) and lands on its top block once it has loaded.
The window shows a stats line at the top, and the program prints one once a second:
`voxel fps <f> work_ms <f> columns <n> chunks <n> faces <n> queue <n> mode <walk|fly>`.

## What it is, and its limits

- The world is columns of 32 x 32 blocks, 128 blocks high (4 chunks of 32 each), made from the frozen viewer's noise. The player's surroundings load within `VIEW` = 8 columns (269 columns, 441 slots in the pool), nearest first, `GEN_BATCH` = 2 columns a frame (see the spec's E65 for why not 8).
- **Edits are not saved:** a column that unloads and comes back is generated again without them.
- The mesher treats everything at y = 128 and above as air, like the player and the ray do (E70), so a peak cut off by the world's ceiling gets a top face.
- Looking and walking have only been run headless, never in a visible window (see the handoff); the numbers are in `kestrel/bench/engine/results.md`.

## Files

Libraries import nothing from the engine (only other libraries and Kestrel's built-in modules),
so they can be used on their own:

- `grid.kes`: `Grid[T]`, a typed 3D array over one list, and the `Solid` interface.
- `noise.kes`: the frozen world's 3D value noise, copied bit for bit.
- `blocks.kes`: block ids, the world's sizes (`SIZE`, `LAYERS`, `HEIGHT`, `PAD`, `DIRT_DEPTH`, `SNOW_LINE`), colours and solidity.
- `column.kes`: `Column`, `ColumnState`, column keys and the world-to-column maths, which is right for negative x.
- `gen.kes`: the world's density, `solid_at`, `block_type` and `generate`, which fills a column's grid in place.
- `ring.kes`: `Ring[T]`, a fixed-size first-in first-out queue.
- `pool.kes`: `Pool[T]` and `Handle`, fixed slots handed out with generation-checked handles.
- `ray.kes`: `cast`, the first solid block along a ray (a grid walk), generic over `Solid`.
- `collide.kes`: `Box` and `slide`, a box moving through blocks one axis at a time, generic over `Solid`.
- `frustum.kes`: `Frustum` and `frustum_of`, which chunk boxes a camera may see.

The engine:

- `world.kes`: `World`, the loaded columns in a pool, blocks read and set across columns, each chunk's meshing state and the queue of chunks waiting for a mesh.
- `mesher.kes`: `mesh_chunk`, one chunk's mesh with ambient occlusion, in chunk-local coordinates.
- `stream.kes`: `stream_step`, loading the columns around the player on all cores and unloading the ones left behind; `VIEW`, `CAPACITY` and the batch sizes.
- `player.kes`: `Player` and `Intent`, walking with gravity and collision or flying, and `spawn`.
- `edit.kes`: `apply_edit` (break or place through the eye ray), `edit_at` (an exact cell) and `dirty_for` (the chunks an edit changes).
- `render.kes`: `Render`, the chunk meshes on the GPU: `mesh_step` drops meshes of reused slots and meshes dirty and queued chunks in one arena a frame; `draw_world` draws the ones the camera sees.
- `hud.kes`: the crosshair, the target block's outline and the text lines.

The entry programs:

- `main.kes`: play, in a window (above).
- `bench.kes` (with `script.kes`, its bookkeeping): the play loop driven by a fixed script, for
  measuring: filling the view, flying straight at 20 and 100 blocks a second, 100 edits and a
  300 s circle, one machine-readable line per phase. Run it hidden:
  `KESTREL_HIDDEN=1 kestrel run --release voxel/bench.kes`, or through
  `bench/engine/measure.py`, which also samples memory; results in `bench/engine/results.md`.
  `tests/bench_smoke.kes` runs it headless with every timed phase cut to one second.
- `parity.kes` and `parity_*.kes`: the frozen voxel benchmark rebuilt on `Grid[T]` (a benchmark module, not a library), with its entry files; timed by `bench/engine/run_parity.ps1`, results in `bench/engine/results.md`.
- `race.kes` and the entry files `race_faces.kes`, `race_arrays.kes` and `race_check.kes`: the race against Rust's `block-mesh` (E26), round 1: block-face meshing of the frozen world's 256 chunks, timed by `bench/kernels`' timer (`py measure.py --only race`), results in `bench/engine/results.md` ("Race, round 1"). Benchmark files, not libraries; they import only `grid` and `noise` because the timer has no window natives.

`tests/` holds the test programs, one per library or engine part (`tests/grid.kes`, ...), plus
the runner's own self-tests (`tests/runner_*.kes`, with `tests/fixtures/`) and the contract
probes (`tests/contract_*.kes`: a `@prove` the compiler can't prove yet, expected to fail).

## Running the tests

Run these in `kestrel/`:

```
cargo test -p kestrel-cli --test voxel_engine
KESTREL_TEST_ONLY=grid cargo test -p kestrel-cli --test voxel_engine    # tests whose file name starts with grid
```

`cli/tests/voxel_engine.rs` runs each `tests/<name>.kes` in a scratch folder under `target/tmp`
(`voxel_<name>` for the `kestrel run` pass, `voxel_<name>_exe` and `voxel_<name>_release` for the
others), as `test_<name>.kes` beside copies of every `kestrel/voxel/*.kes`: once with
`kestrel run`, once as an executable (`kestrel build`), and with `kestrel run --release` when the
file says `// release` (the release run is compared with the same `// expect...` lines). The directives at the end of a test are the program tests' (`// expect:`,
`// expect-exit:`, `// expect-stderr:`, `// env:`) and these:

- `// copy: <repo-relative file>`: copies another file into the scratch folder.
- `// replace: <file> <old> => <new>`: replaces literal text in that scratch copy only.
- `// release`: also runs the test with `--release`.
- `// only-jit`: skips the executable run (say why in the file).

A comment that looks like a misspelt directive (`// relase`, `// replce: ...`) stops the run with
a message, so a typo can't silently drop a pass.

A program that doesn't compile exits non-zero before it runs; the runner compares that exit code
and the messages with `// expect-exit:` and `// expect-stderr:`, so a test can expect a compile
error.

`main.kes` itself is run by the Rust test `main_smoke_runs_the_play_program_headless` (selected
with `KESTREL_TEST_ONLY=main_smoke`, which runs it alone): 120 frames in a quiet headless window, with and without
`--release`, which must exit 0, print a stats line with columns and chunks loaded, and report no
failed contract. Every
window the tests open is headless; none appears. Every build and run is stopped after 120 seconds
and counted as a failure, so a hang fails the test instead of hanging the suite.
