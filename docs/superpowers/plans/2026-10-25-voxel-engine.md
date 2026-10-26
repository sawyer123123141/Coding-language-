# Voxel engine, version 1: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A streaming, editable, walkable voxel world written in Kestrel (`kestrel/voxel/`), with library modules grown from it, measured against the frozen world and raced against the `block-mesh` crate.

**Architecture:** Flat Kestrel program folder: seven library modules (`grid`, `noise`, `ring`, `pool`, `ray`, `collide`, `frustum`) that import nothing of the engine, and engine modules (columns in a fixed pool, streamed around the player, generated and meshed in per-frame `par for` batches inside `arena { }` blocks, uploaded as chunk-local GPU meshes). Entry programs pick the mode: `main.kes` (play), `bench.kes` (scripted, hidden window), `parity*.kes` and `race*.kes` (no window). Tests are Kestrel programs in `kestrel/voxel/tests/` run beside copies of the modules by a new Cargo test.

**Tech Stack:** Kestrel (the compiler in `kestrel/`), Rust for the test runner, two new window natives and the race's Rust side (`block-mesh` 0.2 in `bench/kernels`), Python for `measure.py`.

**Spec:** `docs/superpowers/specs/2026-10-25-voxel-engine-design.md` (rulings E1-E37, plus E38-E45 from this plan's review, in the spec's "From the plan review"). Read it before any task; every task's requirements include it. Proposal: `docs/superpowers/plans/2026-10-25-voxel-engine-proposal.md`.

## Global Constraints

- **No new syntax.** If a task finds it needs a new form, keyword or operator, stop and report; the user decides syntax. New library and native names are fine; natives other than E34's two need the controller's OK and a findings entry.
- **Never edit** `kestrel/bench/voxels3d/` (frozen), `kestrel/test.kes`, `voxels.exe`. **Never stage** `kestrel/check/src/body/enums.rs` (shows modified, empty diff). Stage files by name, never `git add -A`.
- **No window pops up:** every window run uses `KESTREL_HEADLESS=<frames>` (tests) or `KESTREL_HIDDEN=1` (timing). Never set `KESTREL_SMOKE_VISIBLE`. No window, GPU or demo-timing tests in `cargo test`. Headless `make_mesh` works and is fine in tests.
- **Light mode unless the controller says otherwise:** `CARGO_BUILD_JOBS=2`, `--test-threads=2`, idle priority. Run only the tests the task names; the full suite runs once, in Task 12, when the user says the machine is free.
- **Commands run in `kestrel/`.** Engine tests: `cargo test -p kestrel-cli --test voxel_engine` (filter with `KESTREL_TEST_ONLY=<test name prefix>`). Lint: `cargo clippy --workspace --all-targets -- -D warnings` (and in `bench/kernels` when it changes).
- **Kestrel source:** files under about 400 lines; fields one per line; comments explain in plain sentences (GDScript terms where useful), matching `demos/voxels/world.kes`'s density. Library files import nothing from engine files (E2). There are no global variables: state lives in values `main` owns.
- **Constants (spec E3, E4, E10, E12, E13):** `SIZE = 32`, `LAYERS = 4`, `HEIGHT = 128`, `PAD = 34`, `DIRT_DEPTH = 3`, `SNOW_LINE = 88`, `SEED = 1337`, `VIEW = 8`, `CAPACITY = 441`, `GEN_BATCH = 8`, `MESH_BATCH = 16`, `REACH = 6`; block ids `AIR = 0`, `STONE = 1`, `DIRT = 2`, `GRASS = 3`, `SNOW = 4`; chunk states `CHUNK_NONE = 0`, `CHUNK_QUEUED = 1`, `CHUNK_MESHED = 2`.
- **Memory rules (E31, E40):** every per-frame `par for` is inside an `arena { }`. Inside an arena, store into long-lived values only with plain-value writes (`items[i] = v` of a `u8`, `int`, `vec3`, a `Mesh` or `none`); generic method stores (`Grid.set`, `Grid.fill`, `Ring.push`) are refused there. Outside any arena (the frame loop), nothing may allocate per frame: no per-frame lists or `list` fields built anew (an empty list allocates nothing), no whole-grid assignment (`a.blocks = b.blocks` copies 128 KB; copy items one by one), buffers sized before the run and written by index.
- **Contracts (E37, E39):** an `@prove` stays in engine code only if both `kestrel check` and `kestrel check --release` pass with it (contracts are decided per build). A failing one becomes a probe test and a finding. An `@assume` that makes one pass is allowed, with a finding saying what the compiler couldn't see.
- **Honest results:** every number with its median and range and the machine's load; losses written down.
- **Commits:** one or more per task, message in the repo's style (a plain sentence, e.g. "Add Grid[T], the engine's test runner and the engine folder"), ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. The controller pushes to `main`.
- **Ledger:** `.superpowers/sdd/2026-10-25-voxel-engine/` (git-ignored): `progress.md` (each task's commits, review result, rulings), briefs and reports. New rulings are numbered E46 on and also go in the spec's "As built".

## Review Focus

The inputs most likely to bite a player, each pinned to a task's tests:

1. **Negative coordinates** (the world goes both ways from 0): `column_of(-1) == -1`, `local(-1) == 31`, keys of negative columns, rays and collision across x = 0 and z = 0. Tests in Tasks 2, 5 and 7.
2. **Column and chunk borders:** a face on a column edge looks at the neighbour column; an edit on a chunk corner re-meshes all 8 chunks around it; a ray crossing into the next column keeps going. Tests in Tasks 5, 7 and 9.
3. **The top and bottom of the world:** standing on y = 0 blocks, breaking a block at y = 0 (the face below it comes from the terrain function), placing at y = 127 and refusing y = 128, walking and jumping under y = 128 with nothing above. Tests in Tasks 5, 7 and 9.
4. **Unloaded, reused or skipped chunks:** a ray or a fall into an unloaded column stops (it is solid); an edit there does nothing; a queued chunk whose column unloaded and whose slot was reused is skipped; a reused slot never draws the old column's mesh; a chunk that came within view with all its neighbours already loaded still gets meshed. Tests in Tasks 6, 7 and 9.
5. **Long frames and fast flying:** a 2 s frame moves the player at most 0.1 s and doesn't tunnel; flying at 400 blocks/s with one stream step per 1/60 s outruns streaming without running out of slots, overflowing the queue or crashing. Tests in Tasks 6 and 9.

---

### Task 1: The engine folder, its test runner, and `Grid[T]`

**Model: Sonnet; review: yes (every later task runs through this runner).**

**Files:**
- Create: `kestrel/voxel/grid.kes`, `kestrel/voxel/README.md` (a stub: what the folder is, the file list so far, how to run the tests), `kestrel/voxel/tests/grid.kes` (built exes are already ignored by the repo's `*.exe` rule)
- Create: `kestrel/cli/tests/voxel_engine.rs`, `kestrel/cli/tests/common/directives.rs`
- Modify: `kestrel/cli/tests/programs.rs` (uses the moved parser), `kestrel/cli/tests/programs/directives.rs` (the parser's existing tests, `use super::expectations`, wired at `programs.rs:7-8`: move them with the parser), `kestrel/cli/tests/common/mod.rs`

**Interfaces:**
- Produces (Kestrel, `grid.kes`): `pub struct Grid[T] { sx: int, sy: int, sz: int, items: list[T] }` with `pure index(self, x: int, y: int, z: int) -> int` = `(y * self.sz + z) * self.sx + x`, `pure get(self, x, y, z) -> T`, `pure set(var self, x, y, z, v: T)`, `pure inside(self, x, y, z) -> bool`, `pure fill(var self, v: T)`; `pub pure grid[T](sx: int, sy: int, sz: int, fill: T) -> Grid[T]`; `pub interface Solid { pure solid(self, x: int, y: int, z: int) -> bool }`.
- Produces (Rust): `voxel_engine.rs` runs each `kestrel/voxel/tests/<name>.kes` in `CARGO_TARGET_TMPDIR/voxel_<name>`, written there as **`test_<name>.kes`** (never `main.kes`, which is the engine's play program, and never a module's name) beside copies of every `kestrel/voxel/*.kes`. Passes: JIT (`kestrel run`), exe (`kestrel build` then run), and `--release` when the file has `// release`. Both runs go through `common::hermetic` and set `KESTREL_CHECK_PROOFS=1` as `programs.rs`'s runs do. **A build or check that exits non-zero is compared against `// expect-exit:` and `// expect-stderr:` like a run** (contract probes fail at build time, E37), not reported as a build failure. Directives: `// expect:`, `// expect-exit:`, `// expect-stderr:`, `// env:`, `// copy: <repo-relative path>` (copies another file into the scratch folder), `// replace: <file> <old> => <new>` (a literal text replacement in that scratch copy only, done before the run; used for E33's `density` and for shortened constants), `// release`, `// only-jit` (for a test too slow as an exe; the file says why). `KESTREL_TEST_ONLY` filters by test name prefix. One `#[test]` per pass that runs every file and reports every failure together (as `programs.rs` does).

- [ ] **Step 1:** Move the directive parsing (`programs.rs:49-106`) and its tests into `common/directives.rs`, returning a struct of the parsed lines; `programs.rs` uses it, behaviour unchanged, and refuses the new directives (`copy`, `replace`, `release`, `only-jit`: they only make sense beside modules). Run `cargo test -p kestrel-cli --test programs` (light mode): same pass count as before the move.
- [ ] **Step 2: Write the failing test** `kestrel/voxel/tests/grid.kes`. It prints, each line one `// expect:`: `index true` (`index(1, 2, 3) == (2 * sz + 3) * sx + 1` on a 4 x 5 x 6 grid); `len 120`; `get 7 0` after `set(1, 2, 3, 7)` (the cell, then a neighbour); `inside true false false false` (a corner inside; `x = sx`, `y = -1`, `z = sz` outside); `fill true` (every item equal after `fill(9)`); `int 42` and `struct 3 4` (a `Grid[int]` and a `Grid[Cell]` of a two-field struct round-trip a value through `set`/`get`); `solid 5` (a struct `Floor: Solid` over a `Grid[u8]` with 5 solid cells, counted through a generic `pure count_solid[W: Solid](w: W) -> int`).
- [ ] **Step 3:** Write `voxel_engine.rs`; also a tiny self-test of the runner: `tests/runner_replace.kes` (uses `// copy:` of a fixture file in `kestrel/voxel/tests/fixtures/`, a `// replace:` on it, and prints the replaced value) and `tests/runner_fail.kes` (a program with a type error: `// expect-exit: 1` and `// expect-stderr:` of the message, proving build failures compare). Run: FAIL (no `grid.kes`).
- [ ] **Step 4:** Implement `grid.kes` (file header: what a `Grid` is, in GDScript terms a typed 3D array over one `Array`; the index order and steps). `grid[T]` builds `items` with `add` (`array[T](n)` is refused in generic code: the comment says so).
- [ ] **Step 5:** Run `cargo test -p kestrel-cli --test voxel_engine`: PASS (JIT and exe). Clippy clean.
- [ ] **Step 6:** Commit.

### Task 2: Noise, blocks, columns and generation

**Files:**
- Create: `kestrel/voxel/noise.kes`, `blocks.kes`, `column.kes`, `gen.kes`; tests `kestrel/voxel/tests/noise.kes`, `tests/gen.kes`

**Interfaces:**
- Consumes: `Grid[T]`, `grid[T]` (Task 1).
- Produces: `noise.kes`: `pub pure hash(x: int, y: int, z: int, seed: int) -> int`, `pub pure lattice(x, y, z, seed) -> float`, `pub pure smooth(t: float) -> float`, `pub pure mix(a: float, b: float, t: float) -> float`, `pub pure octave(x, y, z, shift: int, seed) -> float` (3D, bit-identical to `bench/voxels3d/voxels3d.kes`). No 2D noise: nothing in version 1 uses it (E41). `blocks.kes`: the block ids and `SIZE`, `LAYERS`, `HEIGHT`, `PAD`, `DIRT_DEPTH`, `SNOW_LINE`; `pub pure colour(id: u8) -> Color` (STONE, DIRT, GRASS, SNOW as in `demos/voxels/world.kes`), `pub pure is_solid(id: u8) -> bool`. `column.kes`: `pub enum ColumnState { Free, Generating, Ready }`, `pub struct Column { cx: int, cz: int, key: i64, state: ColumnState, blocks: Grid[u8] }`, `pub pure column_key(cx: int, cz: int) -> i64`, `pub pure column_of(x: int) -> int` (`x >> 5`), `pub pure local(x: int) -> int` (`x & 31`), `pub pure new_column() -> Column` (Free, a 32 x 128 x 32 grid of AIR). `gen.kes`: `pub pure density(x, y, z, seed) -> float` (the frozen world's), `pub pure solid_at(x, y, z, seed) -> bool` (`density > 0.0`), `pub pure block_type(depth: int, y: int) -> u8` (E4), `pub pure generate(var g: Grid[u8], cx: int, cz: int, seed: int)` (fills a 32 x 128 x 32 grid by writing `g.items[i]` directly, E31; decides types top-down per (x, z), using `solid_at` for the `DIRT_DEPTH - 1` rows above y = 127).

- [ ] **Step 1: Write the failing tests.** `tests/noise.kes` has `// copy: kestrel/bench/voxels3d/voxels3d.kes` and `// replace: voxels3d.kes pure density( => pub pure density(` (E33: the scratch copy only), uses `{ density from voxels3d }` under another name if the names clash (or `use voxels3d` and `voxels3d.density`), and prints `same 512`: the count of 512 points (x and z from -40 to 40, y from -1 to 129, seeds 1337 and 7) where `gen.density == voxels3d.density` exactly. `tests/gen.kes` prints: `key true true` (distinct keys for (1, -1) and (-1, 1); `column_key(-1, -1)` round-trips by shifting back); `column_of -1 31 0 31` (`column_of(-1)`, `local(-1)`, `column_of(31)`, `local(31)`); for a generated column at (0, 0) seed 1337: `types true` (grass or snow only where the block above is air or y = 127 with `solid_at` above false; dirt only within `DIRT_DEPTH - 1` under grass; stone under snow), `solidity true` (every block solid exactly where `solid_at` is), `deterministic true` (generating twice gives the same items); and a column at (-3, -2) gives `negative true` (its block at local (0, y, 0) agrees with `solid_at(-96, y, -64)`).
- [ ] **Step 2:** Run: FAIL.
- [ ] **Step 3:** Implement the four files. `noise` copies the frozen maths operation for operation (same order, same constants), with a header saying why.
- [ ] **Step 4:** Run `KESTREL_TEST_ONLY=noise,gen,grid cargo test -p kestrel-cli --test voxel_engine`: PASS.
- [ ] **Step 5:** Commit.

### Task 3: `Ring[T]` and `Pool[T]`

**Files:**
- Create: `kestrel/voxel/ring.kes`, `pool.kes`; tests `tests/ring.kes`, `tests/pool.kes`

**Interfaces:**
- Produces: `ring.kes`: `pub struct Ring[T] { items: list[T], head: int, count: int }`, `pub pure ring[T](capacity: int, fill: T) -> Ring[T]`, `pure push(var self, v: T) -> bool`, `pure pop(var self) -> T?`, `pure len(self) -> int`, `pure clear(var self)`. `pool.kes`: `pub struct Handle { slot: int, gen: int }`, `pub struct Pool[T] { items: list[T], gens: list[int], used: list[bool], free: list[int] }`, **`pub pure pool[T](capacity: int, fill: T) -> Pool[T]`** (E38: every slot starts as a copy of `fill`, added one by one; building the items list first and passing it would copy every item again, about 4 times the memory), `pure take(var self) -> Handle?`, `pure release(var self, h: Handle)` (does nothing for a handle that isn't alive), `pure alive(self, h: Handle) -> bool`, `pure handle(self, slot: int) -> Handle` (the current handle of a slot), `pure in_use(self) -> int`.

- [ ] **Step 1: Write the failing tests.** `tests/ring.kes` prints: `push true true true false` (capacity 3, the fourth push refused); `pop 1 2` (first in, first out); `wrap 3 4 5` (after pops and pushes past the end); `empty true` (pop on empty gives `none`); `clear 0`. `tests/pool.kes` prints: `take 0 1 2 true` (capacity 3, then `take` gives `none`); `alive true`; after `release(h1)`: `dead true` and `in_use 2`; the next `take` gives slot 1 with a new `gen` (`reuse 1 true`), the old handle stays dead (`stale true`); releasing a dead handle changes nothing (`in_use 3`); an item changed in place through `p.items[h.slot]` keeps its value (`in place 7`).
- [ ] **Step 2:** Run: FAIL.
- [ ] **Step 3:** Implement both (no allocation after creation: `free` is a stack built at creation; `gen` grows by one on each release).
- [ ] **Step 4:** Run (`KESTREL_TEST_ONLY=ring,pool`): PASS. Also a quick memory probe in the ledger: `pool(441, new_column())` with `KESTREL_ARENA_REPORT=1` should hold about 441 x 128 KB plus the doubling waste of one grid (well under 100 MB); record the number.
- [ ] **Step 5:** Commit.

### Task 4: Parity with the frozen benchmark (E19, E36, E42)

The early answer to "do generics cost anything in real code". **Model: Sonnet; review: yes (the number decides whether the build stops to look at the compiler).**

**Files:**
- Create: `kestrel/voxel/parity.kes` (the module: `parity_faces`, `parity_faces_par`), entry files `kestrel/voxel/parity_main.kes`, `parity_get.kes`, `parity_reuse.kes`, `parity_par.kes`, `parity_empty.kes` (startup only); test `tests/parity.kes`
- Create: `kestrel/bench/engine/run_parity.ps1`, `kestrel/bench/engine/results.md` (section "Parity"), `kestrel/bench/engine/.gitignore` (`out/`)

**Interfaces:**
- Consumes: `grid`, `gen.density` (the frozen one, Task 2).
- Produces: `pub pure parity_faces(seed: int, width: int, how: int) -> int` with `how` one of `PARITY_ITEMS = 0` (the verdict line, E42: each chunk samples its 34^3 padded solids into a fresh `Grid[u8]` and the mesher indexes `g.items[i]` with the frozen program's own index arithmetic and steps, so the difference to the frozen copy is list-against-array plus `grid[T]`), `PARITY_GET = 1` (the same through `g.get(x, y, z)`, measuring the generic method), `PARITY_REUSE = 2` (one grid made once and `fill(0)`ed per chunk **outside** the per-chunk `arena { }`, where `fill` is allowed); and `pub parity_faces_par(seed: int, width: int) -> int` (the `PARITY_ITEMS` work across cores, like `main_par.kes`). Every variant meshes vertices, normals and indices as `voxels3d.kes`'s `Mesh.quad` does, each chunk's mesh in its own `arena { }`.

- [ ] **Step 1: Write the failing test** `tests/parity.kes` (`// release`, `// only-jit` if the exe pass is slow): prints `463749 463749 463749 463749` for the four variants at width 8, seed 1337.
- [ ] **Step 2:** Run: FAIL. **Step 3:** Implement. **Step 4:** Run: PASS.
- [ ] **Step 5: Measure.** `run_parity.ps1` builds the frozen `main.kes`, `main_par.kes`, `main0.kes` (from `bench/voxels3d`, output into `kestrel/bench/engine/out/`, never into the frozen folder) and the five parity entry files, checks on and `--release`, and runs every exe in turn for 11 rounds (`KESTREL_THREADS=1` for the one-thread variants), median and range with the empty program's startup subtracted. Also `kestrel check --report --release` of `parity_main.kes` against the frozen `main.kes`: total checks, removed, left, and which lines differ.
- [ ] **Step 6:** Write "Parity" in `results.md`: the table, the machine's load, and the verdict against the 5% target (`PARITY_ITEMS`), with `GET` and `REUSE` as the explaining lines. **If `PARITY_ITEMS` is more than 5% slower than the frozen copy, stop and report to the controller** with evidence of where (the controller decides whether a general compiler fix comes first).
- [ ] **Step 7:** Commit.

### Task 5: The world and the mesher (E8, E11, E14, E20)

**Model: Opus; review: yes.**

**Files:**
- Create: `kestrel/voxel/world.kes`, `kestrel/voxel/mesher.kes` (split into `mesher.kes` and `ao.kes` if over 400 lines); tests `tests/world.kes`, `tests/mesher.kes`, `tests/engine_parity.kes`

**Interfaces:**
- Consumes: Tasks 1-3.
- Produces: `world.kes`: `pub struct ChunkRef { column: Handle, layer: int }`; `pub struct World: Solid { seed: int, columns: Pool[Column], index: map[i64, int], chunk_state: list[u8], queue: Ring[ChunkRef] }`; `pub new_world(seed: int, capacity: int) -> World` (`pool(capacity, new_column())`, `chunk_state` of `capacity * LAYERS` zeros, a queue of capacity `capacity * LAYERS`); `pure slot_at(self, cx: int, cz: int) -> int` (-1 when not loaded or not Ready); `pure block(self, x: int, y: int, z: int) -> u8` (AIR above 127, STONE below 0, STONE in a column that isn't Ready); `pure solid(self, x, y, z) -> bool` (`is_solid(block(...))`); `pure set_block(var self, x, y, z, id: u8) -> bool` (false outside 0..127 or in an unloaded column); `pub load_now(var w: World, cx: int, cz: int)` (take a slot, generate in place, mark Ready, index it: for tests and region builds, no queueing). `mesher.kes`: `pub pure mesh_chunk(w: World, slot: int, layer: int) -> MeshData` (E14: the 34^3 padded copy from up to 9 columns, outside 0..127 from `gen.solid_at` as STONE or AIR; chunk-local vertices; colours by `blocks.colour`, face shade, `variation`, ambient occlusion, the brighter-diagonal cut, as `demos/voxels/world.kes`); `pub pure faces(m: MeshData) -> int` (`indices.len / 6`); `pub pure chunk_index(slot: int, layer: int) -> int` (`slot * LAYERS + layer`).

- [ ] **Step 1: Write the failing tests.** `tests/world.kes`: `load_now` of (0, 0) and (-1, 0); `block` agrees with the column's grid at a few cells including x = -1; `block(5, -1, 5) == STONE`, `block(5, 128, 5) == AIR`, `block` in an unloaded column is STONE; `set_block` at y = 127 true, at y = 128 false, in an unloaded column false; `solid` through a generic `[W: Solid]` helper. `tests/mesher.kes`: an all-AIR world (columns loaded with `load_now`, then every item set to AIR by a helper in the test file) with one STONE block in the middle of chunk (0, 0, 0) gives `6 24 36` (faces, vertices, indices) with chunk-local positions in [0, 32]; a block at local x = 31 next to a solid block at x = 32 in column (1, 0) shows 5 faces; a block at x = 0 next to one at x = -1 in column (-1, 0) shows 5 faces; a block at y = 0 shows its bottom face only where `solid_at(x, -1, z)` is false; ambient occlusion corner brightness `0.55 + 0.15 * ao` for faces with one and two solid neighbours, and the cut diagonal. `tests/engine_parity.kes` (`// release`, `// only-jit` allowed): `load_now` of columns -1..8 x -1..8, meshes every chunk of columns 0..7 and prints `463749`.
- [ ] **Step 2:** Run: FAIL. **Step 3:** Implement. **Step 4:** Run (`KESTREL_TEST_ONLY=world,mesher,engine_parity`): PASS.
- [ ] **Step 5:** Contracts on the mesher's inner loop (E39: both `check` and `check --release` must pass; an `@assume` on the grid's length is the expected help; failures go to a `tests/contract_mesher.kes` probe and the ledger).
- [ ] **Step 6:** Commit.

### Task 6: Streaming (E12, E13, E35, E43, E44)

**Model: Opus; review: yes.**

**Files:**
- Create: `kestrel/voxel/stream.kes`; tests `tests/stream.kes`, `tests/stream_fast.kes`

**Interfaces:**
- Consumes: Task 5.
- Produces: `pub struct GenJob { slot: int, cx: int, cz: int, active: bool, blocks: Grid[u8] }`; `pub struct Stream { view: int, offsets: list[ivec2], jobs: list[GenJob], last_column: ivec2 }`; `pub new_stream(view: int, batch: int) -> Stream` (offsets within `view + 1` sorted by length, then by angle for a stable order; `batch` jobs with their own 32 x 128 x 32 grids; `last_column` a value no column has); `pub struct StreamStats { unloaded: int, started: int, generated: int, queued: int, out_of_slots: int, queue_full: int, gen_us: i64 }`; `pub stream_step(var s: Stream, var w: World, at: vec3) -> StreamStats`:
  1. **Unload** Ready columns farther than `view + 2` (column centre to `at`, in columns): clear their chunk states, remove the key, release the slot. (Render notices on its own, Task 9.)
  2. **Start** up to `batch` jobs with the nearest missing columns within `view + 1`: take a slot each, mark it `Generating`, index the key. A failed `take` counts `out_of_slots` and prints "the column pool is full (441 slots); this is a bug in the stream's capacity".
  3. **Generate**: `arena { par for j in s.jobs { if j.active { generate(var j.blocks, j.cx, j.cz, w.seed) } } }`, timed into `gen_us`.
  4. **Finish**, outside the arena: copy each active job's items into its slot's grid **item by item** (a whole assignment allocates 128 KB, E40), mark Ready, deactivate the job.
  5. **Queue** (E43): check the chunks of the finished columns and their 8 neighbours; **also, whenever the player's column differs from `last_column`, rescan every column within `view`** (columns can come within view with all neighbours already loaded, since unloading is at `view + 2`). A chunk is queued when its column is within `view`, all 9 columns around it are Ready, and its state is `CHUNK_NONE`: `w.queue.push(ref)`; on `true` set `CHUNK_QUEUED`, on `false` leave it `CHUNK_NONE` and count `queue_full` (a later rescan or finish retries it).
  - **E44, for Task 9:** a ref popped by the mesher whose column is no longer alive is dropped; one whose column is alive but whose chunk is no longer meshable (a neighbour unloaded) is reset to `CHUNK_NONE`, so a later check queues it again.

- [ ] **Step 1: Write the failing tests.** Both drain the queue each step with a stand-in mesher in the test (pop up to `MESH_BATCH`, apply E44's rule, set `CHUNK_MESHED`). `tests/stream.kes` (no window): a walk along +x at 20 blocks per step for 60 steps, a jump of 2,000 blocks, 30 more steps, then a circle of radius 200 blocks in 120 steps and a back-and-forth of 40 steps; each step calls `stream_step` until it starts nothing, then drains. Prints: `filled true` (after each settled step, every column within `view` is Ready and every chunk of them is queued or meshed); `never full true` (`out_of_slots` and `queue_full` 0 throughout); `in use true` (at most 441); `no duplicates true` (no `ChunkRef` in the queue twice, checked by popping into a list and comparing); `unloaded true` (no Ready column farther than `view + 2`); `teleport true`; `stale true` (a ref for a column whose slot was released and reused fails `alive`). `tests/stream_fast.kes`: one `stream_step` and one drain per 1/60 s step while flying at 400 blocks/s for 600 steps: `never full true`, `no crash true`, and `behind true` (the loaded area lags but every column within `view + 2` stays within capacity).
- [ ] **Step 2:** Run: FAIL. **Step 3:** Implement. **Step 4:** Run (`KESTREL_TEST_ONLY=stream`): PASS.
- [ ] **Step 5:** Commit.

### Task 7: `ray`, `collide` and `frustum`

**Files:**
- Create: `kestrel/voxel/ray.kes`, `collide.kes`, `frustum.kes`; tests `tests/ray.kes`, `tests/collide.kes`, `tests/frustum.kes`

**Interfaces:**
- Consumes: `Solid` (Task 1). Tests use a small `Solid` over a `Grid[u8]` with an origin offset so negative cells exist.
- Produces: `ray.kes`: `pub struct Hit { cell: ivec3, before: ivec3, distance: float }`, `pub pure cast[W: Solid](w: W, from: vec3, dir: vec3, reach: float) -> Hit?` (a zero `dir` gives `none`; `distance` along the normalised `dir`). `collide.kes`: `pub struct Box { min: vec3, max: vec3 }`, `pub struct Moved { box: Box, hit: ivec3 }`, `pub pure slide[W: Solid](w: W, b: Box, motion: vec3) -> Moved` (y, then x, then z; steps of at most 0.5 per axis; a skin of 0.001 so a box resting on a floor isn't counted inside it). `frustum.kes`: **`pub struct Frustum` with six `vec4` plane fields** (E40: no list, it is built every frame), `pub pure frustum_of(cam: Camera, aspect: float) -> Frustum`, `pure sees(self, min: vec3, max: vec3) -> bool`.

- [ ] **Step 1: Write the failing tests.** `tests/ray.kes`: from (0.5, 0.5, 0.5) along each of the six axes to a block 3 away: the hit cell, `before` one cell back, distance 2.5; a diagonal hit; a ray starting exactly on a cell boundary; a ray crossing x = 0 into negative cells and one crossing z = 0; a miss past `reach` gives `none`; a zero direction gives `none`. `tests/collide.kes` (positions compared within 0.01): a box falling 5 blocks onto a floor stops with `min.y` at the floor top and `hit.y == -1`; a fall of 30 blocks in one call doesn't pass through a 1-block floor; sliding diagonally into a wall keeps the motion along the wall (`hit.x != 0`, z moved fully); a corner; jumping into a ceiling stops with `hit.y == 1`; a box resting on a floor moving sideways isn't stopped by the floor; a box moving from x = 0.5 to x = -2 into a wall at x = -2 (negative cells) and the same across z = 0.
  `tests/frustum.kes`: a camera at the origin looking along +z (fov 75, aspect 16/9): a box ahead is seen, behind is not, far to the side is not, one straddling the left plane is seen, one beyond 4,000 is not.
- [ ] **Step 2:** Run: FAIL. **Step 3:** Implement (file headers: the algorithm in a few sentences; GDScript terms, e.g. `move_and_slide`, `Camera3D.is_position_in_frustum`). **Step 4:** Run (`KESTREL_TEST_ONLY=ray,collide,frustum`): PASS.
- [ ] **Step 5:** Contracts on `cast`, `slide` and `sees` (E39); failures to probes.
- [ ] **Step 6:** Commit.

### Task 8: Mouse capture natives (E34)

**Files:**
- Modify: `kestrel/driver/std/input.kes`, `kestrel/window/src/natives_input.rs`, `kestrel/window/src/raylib.rs` (externs for `DisableCursor`, `EnableCursor`, `GetMouseDelta`), `kestrel/window/src/lib.rs` (`table!`), `kestrel/window/src/native_tests.rs` (the count 36 at lines 206, 208, 210 and 238 becomes 38), `kestrel/driver/tests/builtin.rs:151` (`9 + 15 + 7 + 5`: the input term grows by 2), the syntax spec's built-in modules list (one line each, no syntax change).
- Test: a headless program test `kestrel/tests/programs/input_capture_headless.kes`.

**Interfaces:**
- Produces: `pub native capture(on: bool)` and `pub native mouse_delta() -> vec2` in `input`. Headless or closed window: `capture` does nothing, `mouse_delta` is (0, 0). Before `window.open` they fail like the other input natives (same error).

- [ ] **Step 1: Write the failing test** `input_capture_headless.kes` (`// env: KESTREL_HEADLESS=2`): opens a window, calls `input.capture(true)`, prints `delta vec2(x: 0, y: 0)` (how `print` shows a `vec2`) on each of two frames, then `input.capture(false)`; the expect lines as the headless harness prints them.
- [ ] **Step 2:** Run `KESTREL_TEST_ONLY=input_capture cargo test -p kestrel-cli --test programs`: FAIL. **Step 3:** Implement. **Step 4:** Run it and `cargo test -p kestrel-window` and `cargo test -p kestrel-driver --test builtin`: PASS. No GPU or visible-window test.
- [ ] **Step 5:** Commit.

### Task 9: Player, edits, rendering, HUD and `main.kes` (E15-E18, E44, E45)

**Model: Opus; review: yes.** The largest task; the implementer may commit it in two parts (player and edits first, then render, HUD and main), with one review at the end.

**Files:**
- Create: `kestrel/voxel/player.kes`, `edit.kes`, `render.kes`, `hud.kes`, `main.kes`; tests `tests/player.kes`, `tests/edit.kes`, `tests/render.kes`, `tests/main_smoke.kes` (or a Rust test in `voxel_engine.rs` that runs the real `main.kes` headless)

**Interfaces:**
- Consumes: Tasks 5-8.
- Produces:
  - `player.kes`: `pub struct Intent { move: vec3, turn: vec2, jump: bool, wheel: float, break_block: bool, place_block: bool, toggle_fly: bool }` (`move` is (right, up, forward), each -1 to 1); `pub struct Player { pos: vec3, vel: vec3, yaw: float, pitch: float, fly: bool, speed: float = 20, standing: bool }` (`pos` is the feet, the box's bottom centre); `pub pure step_player(var p: Player, w: World, i: Intent, dt: float)` (E17: `dt` capped at 0.1, physics in steps of at most 1/60 s; walk: 4.3 blocks/s, gravity 32, jump 9, fall cap 60, box 0.6 x 1.8 x 0.6; fly: `demos/voxels/fly.kes`'s movement, no collision; `toggle_fly` switches, and leaving fly mode inside a solid block is refused, staying in fly); `pub pure eye(p: Player) -> vec3` (pos + 1.62 up), `pub pure look(p: Player) -> vec3`, `pub pure view(p: Player) -> Camera`, `pub pure box_of(p: Player) -> Box`; **`pub pure spawn(var p: Player, w: World) -> bool`** (E45: while the player's column isn't Ready, stays in fly mode where it is and gives false; once Ready, puts the feet on top of the highest solid block of that (x, z), switches to walk, gives true; `main.kes` and `bench.kes` call it each frame until it gives true).
  - `edit.kes`: `pub pure dirty_for(w: World, cell: ivec3) -> list[ChunkRef]` (every loaded chunk whose 34-block region contains `cell` and whose state is queued or meshed; 1 to 8; called only on an edit, so its list is not per-frame); **`pub edit_at(var w: World, cell: ivec3, id: u8) -> list[ChunkRef]`** (set the block and give the dirtied chunks; refused, with an empty list, outside 0..127 or in an unloaded column); `pub apply_edit(var w: World, p: Player, i: Intent) -> list[ChunkRef]` (the ray `ray.cast(w, eye(p), look(p), REACH)`, then `edit_at`; placing refused inside the player's box). The bench calls `edit_at` with exact cells.
  - `render.kes`: `pub struct Render { meshes: list[Mesh?], gens: list[int], faces: list[int], total_faces: int }` (`gens[i]`: the column generation chunk `i`'s mesh was built for); `pub new_render(capacity: int) -> Render`; `pub struct MeshStats { meshed: int, mesh_us: i64, upload_us: i64 }`; `pub mesh_step(var r: Render, var w: World, dirty: list[ChunkRef], batch: int) -> MeshStats` (E15: first drop every mesh whose slot is no longer in use or whose slot's generation differs from `gens[i]`; then pop up to `batch` refs, applying E44 to stale and unmeshable ones, add the dirty refs; mesh the jobs in `arena { par for }` and upload each in the same arena with `make_mesh` (an empty `MeshData` stores `none`), setting `gens`, `faces` and `CHUNK_MESHED`); `pub draw_world(r: Render, w: World, cam: Camera, aspect: float) -> int` (chunks drawn after `frustum.sees`). The job list lives inside the arena.
  - `hud.kes`: `pub draw_hud(...)` (crosshair, target outline with 12 `draw3d.line`s, the text line).
  - `main.kes`: the loop of the spec's "Entry programs", real `window.delta()`, `Tab` toggles `input.capture`, uncaptured right-drag look with the 4-pixel click rule (E16), a stats line once a second: `voxel fps <f> work_ms <f> columns <n> chunks <n> faces <n> queue <n> mode <walk|fly>`.

- [ ] **Step 1: Write the failing tests.** `tests/player.kes` (worlds built with `load_now` and `set_block`): a walking player dropped from 10 blocks above flat ground lands and `standing` is true; walking into a 2-block wall stops; a jump clears a 1-block step and doesn't clear a 2-block wall; a 2-second `dt` moves at most 0.1 s worth; walking toward an unloaded column stops at its edge; standing on a floor at y = 0 (blocks of y = 0 only) doesn't fall through; jumping under y = 128 with air above rises and falls back; fly mode passes through blocks; `toggle_fly` inside a block stays in fly; `spawn` gives false before the column is Ready and true after, with the feet on the top solid block. `tests/edit.kes`: `dirty_for` gives 1 chunk for a chunk-middle cell, 2 for a face-border cell (x local 0), 4 for an edge (x and z local 0), 8 for a corner (x, z local 0 and y = 32); breaking then placing at the same cell restores the block and the chunk's face count; placing into the player's box is refused; placing at y = 128 is refused; breaking at y = 0 leaves a hole whose bottom face appears only if `solid_at(x, -1, z)` is false; an edit in an unloaded column does nothing.
- [ ] **Step 2:** Run: FAIL. **Step 3:** Implement `player.kes` and `edit.kes`. **Step 4:** Run (`KESTREL_TEST_ONLY=player,edit`): PASS. Commit.
- [ ] **Step 5: Write the failing tests** `tests/render.kes` (`// env: KESTREL_HEADLESS=10`, `// env: KESTREL_HEADLESS_QUIET=1`; headless `make_mesh` works): a chunk meshed then its column unloaded and the slot reused for another column: the old mesh is dropped before drawing (`dropped true`) and the new column's chunk is meshed with the new generation (`regen true`); an all-air chunk stores `none` (`empty none`); a stale queued ref is skipped (`stale skipped true`); a queued ref whose neighbour unloaded resets to `CHUNK_NONE` (`reset true`). And the smoke test: the real `main.kes` under `KESTREL_HEADLESS=120` and `KESTREL_HEADLESS_QUIET=1` (and `--release`) exits 0 and prints at least one `voxel fps ...` line with `columns` and `chunks` above 0. (Headless time is fake, 1/60 s a frame.) Keep headless frame counts low: `hermetic` sets `KESTREL_POISON=1`, which never returns freed blocks.
- [ ] **Step 6:** Run: FAIL. **Step 7:** Implement `render.kes`, `hud.kes`, `main.kes`. **Step 8:** Run: PASS. (`main.kes` has no exit of its own, so its first real-window run is `bench.kes` in Task 10.)
- [ ] **Step 9:** Commit. Update `kestrel/voxel/README.md` (controls, files, libraries vs engine).

### Task 10: `bench.kes`, the engine measurements, contracts and the findings (E18b, E31, E32, E37)

**Files:**
- Create: `kestrel/voxel/bench.kes` (and `script.kes` if needed for size), test `tests/bench_smoke.kes`; `kestrel/bench/engine/measure.py`
- Modify: `kestrel/bench/engine/results.md` (sections "Engine", "Memory", "Contracts", "Compile time and size"), `docs/superpowers/plans/2026-10-09-core-2a-followups.md` (section "Found while building the voxel engine (2026-10-25)", E30)

**Interfaces:**
- Consumes: everything above. `bench.kes` drives `Player` with scripted `Intent`s and a fixed `dt` of 1/60 (E32), sets `p.speed` directly (20, 100, 50), never captures the mouse, calls `spawn` until it holds, and does its edits with `edit_at` at fixed cells (25 chunk-middle cells and 25 chunk-corner cells, each broken then placed back). Its frame-time buffer is sized before the run (the longest phase's frame count) and written by index (E40). Output, one line each:
  - `phase <name> frames <n> work_med <ms> work_p99 <ms> work_max <ms> wall_med <ms> generated <n> meshed <n> gen_ms <f> mesh_ms <f> upload_ms <f>` at the end of each phase (`gen_ms` per column, `mesh_ms` and `upload_ms` per chunk, medians);
  - `edits mid_ms <f> corner_ms <f>` (handling to upload done, `ticks_usec`, medians);
  - `mark <seconds>` every 10 simulated seconds of the flight, for `measure.py` to timestamp its memory samples;
  - `done` at the end.
  - Phases (constants at the top): `fill` (standing until the queue is empty and every column within `VIEW` is meshed), `fly20` (20 s straight at 20 blocks/s), `fly100` (20 s at 100), `edits`, `circle` (300 s on a circle of radius 1,000 blocks at 50 blocks/s, so it keeps streaming).

- [ ] **Step 1:** Write `bench.kes` and `tests/bench_smoke.kes`: the test copies `bench.kes` with `// replace:` lines cutting every phase to 1 simulated second and runs it under `KESTREL_HEADLESS=400`, `KESTREL_HEADLESS_QUIET=1`: the five `phase` lines, the `edits` line and `done` appear (the timings are fake there).
- [ ] **Step 2:** Write `measure.py`: builds `bench.kes` `--release` into `kestrel/bench/engine/out/`, runs it with `KESTREL_HIDDEN=1` three times, samples the process's working set every 250 ms (Python `ctypes` `GetProcessMemoryInfo`, no new packages), timestamps the `mark` lines, and prints the phase lines' medians and ranges and the memory curve (start, after `fill`, every 50 s of `circle`). Also two runs with `KESTREL_ARENA_REPORT=1` and `circle` cut to 100 s and 300 s (a second build with the constant replaced): the program arena's size at exit must be the same (E31: no growth; the report prints only at exit, `runtime/src/start.rs:9`). Never a visible window.
- [ ] **Step 3:** Run it (after asking the controller whether the machine is free; the load is noted either way) and write "Engine" and "Memory" in `results.md`: the pass marks (E32: 99th-percentile work time at most 16.7 ms in `fly20`, `fly100` and `circle`; E31: the same program arena at both flight lengths, and a working set within 5% over the last 200 s of `circle`), with the numbers whether they pass or not; per-column generate and per-chunk mesh and upload times; columns per simulated second; fill time; edit latency; the spare-block plateau, sized.
- [ ] **Step 4: Contracts.** For the hot loops (generation, the mesher, `Grid.get/set`, `cast`, `slide`, `sees`), keep the annotations that pass both builds (E39), list the probes, and write "Contracts" in `results.md` from `kestrel check --report` and `--report --release` of `main.kes`: checks left in each hot function, with the reason.
- [ ] **Step 5: Compile time** of `main.kes` (`kestrel build`, `--release`, `kestrel check`: 9 runs, median and range) and lines per file ("Compile time and size").
- [ ] **Step 6: Findings.** Write the followups section from the ledger: every workaround from Tasks 1-9 (the spec's E30 list confirmed or dropped with evidence, plus E31's arena rule, E38's pool memory, E40's whole-grid copy, the `@assume` contracts needed), each with file:line, cost, the general fix, and "syntax (the user's)" or not.
- [ ] **Step 7:** Commit.

### Task 11: The race, round 1 (E26)

**Model: Sonnet; review: yes (fairness of the comparison).**

**Files:**
- Create: `kestrel/voxel/race.kes` (the shared input building and kernels), entry files `kestrel/voxel/race_faces.kes`, `race_arrays.kes` (one kernel each, since the timer has no mode argument)
- Modify: `kestrel/bench/kernels/Cargo.toml` (`block-mesh = "0.2"`; use its re-exported `block_mesh::ndshape`, no separate `ndshape` line), `kestrel/bench/kernels/src/bin/rust.rs` (or a new `src/race.rs` it includes) with modes `race_faces` and `race_arrays`, `kestrel/bench/kernels/measure.py` (entries whose labels contain "race"), `kestrel/bench/kernels/results.md` (a pointer) and `kestrel/bench/engine/results.md` (section "Race, round 1")

**Interfaces:**
- Consumes: `grid`, `gen.solid_at`.
- Produces: each race entry file exports **`pub bench(n: int, iters: int) -> float`** (the timer's signature, `bench/kernels/src/main.rs:62-63`; read that file for how `n`, `iters` and the base call `bench(n, 0)` are used). Kestrel has no globals, so each call builds the 256 chunks' padded 34^3 solid flags (seed 1337) **inside an `arena { }`** together with the kernel passes (else each call keeps about 30 MB), and the timer's base call subtracts that setup; pick `iters` so the kernel time is well above the setup's jitter, and say so. Kernel 1 (`race_faces`) writes six lists of packed ints (`x | y << 6 | z << 12` of each face's block) per chunk, cleared and reused; kernel 2 (`race_arrays`) also turns them into positions, normals and indices (4, 4 and 6 per face) in lists cleared and reused per chunk. The result is the face total of one pass (463,749) as a `float` (exact below 2^24), checked once per call. The Rust modes build the same inputs with a Rust port of `hash`/`lattice`/`smooth`/`mix`/`octave`/`density` (`f32`, the same order of operations; it must also total 463,749), a newtype voxel implementing `Voxel` (`get_visibility`), `block_mesh::visible_block_faces` over a `ConstShape3u32<34, 34, 34>` with `RIGHT_HANDED_Y_UP_CONFIG.faces` into a `UnitQuadBuffer` (cleared per chunk if the call doesn't reset it), and for kernel 2 `quad_mesh_positions`, `quad_mesh_normals` and `quad_mesh_indices` into reused `Vec`s. Check the crate's real API on docs.rs / the downloaded source and note any difference in the ledger.

- [ ] **Step 1:** Write `race.kes`, the two entry files and the Rust modes; each checks the face total 463,749 on its first pass (a mismatch is an error, not a number).
- [ ] **Step 2:** `cargo build --release` in `bench/kernels`; run each mode once.
- [ ] **Step 3:** Measure with `measure.py --rounds 9 --only race --threads 1`: Kestrel `--release` and checks on, Rust release (and with overflow checks if `measure.py` supports it), ns per chunk, median and range, in turn. Also, if `C:\kt\voxtuned\rust` builds, the straight port of the whole frozen world against the frozen and tuned Kestrel copies (else say it didn't).
- [ ] **Step 4:** Write "Race, round 1": the table, what each side does (the same algorithm; block-mesh's generic `Voxel` trait against Kestrel's `Grid[u8]`), the list management stated (E26), the verdict whichever way it goes, and what round 2 (greedy, `binary-greedy-meshing`) would need.
- [ ] **Step 5:** Commit (the `Cargo.lock` change included).

### Task 12: Final review, full suite, docs

**Model: Opus reviewers.**

- [ ] **Step 1:** Whole-branch review by three Opus reviewers in parallel, by area: (a) the libraries and their tests (generic API shape, bit-identity, the Review Focus edge cases), (b) the engine (streaming, memory and the E31/E40 rules, edits, physics, rendering), (c) measurements and honesty (results files, findings, the race's fairness). Findings to the ledger; one fix wave; a re-review of the fixes.
- [ ] **Step 2:** Full suite when the user says the machine is free: `cargo test --workspace --no-fail-fast` and clippy in `kestrel/` and `bench/kernels`; the frozen world's `check --report` and `--report --release` byte-identical to before (17,502 and 15,184 bytes).
- [ ] **Step 3:** Docs: `kestrel/voxel/README.md` complete; the main `README.md`'s "Voxel engine" pointer; the spec's "As built" (every E46+ ruling and design change, with reasons); the handoff's new "Start here" (status, numbers, findings count, the natives' names for the user to confirm, next: race round 2 and the rounds the findings pick); `CLAUDE.md`'s "Start here" line.
- [ ] **Step 4:** Commit; the controller pushes.
