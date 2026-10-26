# Voxel engine, version 1: design (2026-10-25)

**Status: spec, reviewed (builder check and plan review folded in, E31-E45), not built.** From the approved proposal `docs/superpowers/plans/2026-10-25-voxel-engine-proposal.md` (approved 2026-10-25 "as written, block-mesh for round 1"). The plan is `docs/superpowers/plans/2026-10-25-voxel-engine.md`. Rulings here are numbered E1, E2, ... so the plan and reviews can point at them; they are Claude's calls (architecture is delegated) unless marked **(user)**.

## Intent

Kestrel's next round is a real program: a voxel engine written in Kestrel, using the generics built on 2026-10-25. It has two jobs:

1. **Be a real program**: a world you fly and walk around in, dig and build in, streamed around you as you move.
2. **Find the language's gaps by needing things.** Every workaround goes in a findings list with the general fix it points at. That list picks the next rounds (`await`, hot reload, and whatever else shows up). Libraries (`Grid[T]`, noise, pools, ...) grow out of the engine instead of being designed in a vacuum.

Kestrel stays a general language. Voxels are one use of it, not its design centre. A fix the engine asks for should be general compiler or library work; anything voxel-specific says so.

## Goals, non-goals, anti-goals

**Goals (user, from the proposal):**
- A streaming, editable, walkable world in the standalone window (Pick 1).
- Library modules grown from the engine: `grid`, `noise`, `ring`, `pool`, `ray`, `collide`, plus `frustum` (E5) (Pick 3).
- Measured claims (Pick 4): generics cost nothing in a real program; no allocation once warmed up; no checks left in hot loops where provable; 60 fps while streaming with the worst frame reported; edits drawn within the frame; files under about 400 lines; every workaround recorded.
- Measurements and the Rust race, round 1 against the `block-mesh` crate **(user)** (Pick 5).

**Non-goals (version 1):** textures (vertex colours as now), light that spreads (ambient occlusion only), water, saving (no file I/O in Kestrel), mobs, greedy meshing (race round 2), several placeable block types, sharing the libraries with other programs (needs an import decision, the user's, later), multiplayer, an inventory or any game rules.

**Anti-goals (a failure even if it works):**
- **New syntax without the user.** Version 1 needs none. If the build finds it needs syntax (a new form, keyword or operator), it stops and writes a small proposal for the user. New library and native *names* are not syntax, but each new native is listed in the findings (E30).
- **Editing `kestrel/bench/voxels3d/`** (frozen), `kestrel/test.kes` or `voxels.exe` (the user's). Staging `kestrel/check/src/body/enums.rs` (shows modified with an empty diff).
- **Hiding a loss.** Every measured claim is written down with its number, also when Kestrel loses (the user wants honest, critical results).
- **Voxel-specific compiler features** (intrinsics, special cases for `Grid`). Compiler fixes found here must be general.
- **An "evaluation game"**: no side programs written only to rate the language. The engine's own entry programs (play, bench, parity, race) are part of the engine.
- **Windows popping up.** Every window run (tests, measurements, subagents) is headless (`KESTREL_HEADLESS=<frames>`) or hidden (`KESTREL_HIDDEN=1`). Never set `KESTREL_SMOKE_VISIBLE`.

## Constraints (the language as it is, checked 2026-10-25)

- **Modules are one file each, found only next to the file that uses it**; a module name is one identifier (`driver/src/lib.rs:186,208`). So the engine is a flat folder.
- **Integers:** `int` is 32-bit, plus `i64` and `u8`; no `u16`/`u32`. `float` is 32-bit. Overflow is checked in normal builds and wraps under `--release`, which also removes bounds checks.
- **Generics** (`docs/superpowers/specs/2026-10-24-generics-design.md`, syntax spec "Generics"): no arithmetic on `T`; `array[T](n)` refused in generic code, so a generic container holds a `list[T]` built with `add` (there is no `list[T](n, fill)` or `resize`); an owning type (`Mesh`, `Arena`, or a struct holding one) can't be `T`; an optional can't be `T`; methods have no placeholders of their own; a generic type from another module is imported by name.
- **Maps:** keys are `int`, `i64`, `u8`, `bool`, `string` only; values can't own a GPU object or an arena (`check/src/collect_ty.rs:152-192`).
- **Owning values:** a `Mesh` field or a `list[Mesh?]` item can be overwritten (the old GPU mesh is freed at the next `window.frame()`) or set to `none`. `let m = c.mesh else { ... }` on a `for` loop item is refused ("only whole variables move"); unwrap inside a helper that takes the item as a parameter.
- **Arenas:** `arena { }` frees on exit; `in x.mem { }` allocates into an owned `Arena`; an `Arena` field can't be reset or replaced, only freed with its owner. Freed arena blocks stay in a per-thread cache (up to 256 MiB) instead of going back to Windows (a known gap). **So steady-state memory comes from reusing pooled storage in place, not from arenas per chunk** (E9).
- **`par for`** runs over a list or array; the body may change its own item, read anything that isn't another item, call only `pure` functions (with `var` arguments rooted at the item or body locals), and use `arena { }`. It can't change outer variables, `print`, `return` or `break`. A `par for` always fails `@prove(no_alloc)`.
- **Function values** can be `pure` (`solid: pure (int, int, int) -> bool`), but every `@prove` contract fails on a function that calls one ("contracts can't see into a function value yet"). A generic with an interface rule is a direct, inlinable call and keeps contracts working (E6).
- **Window and input:** `input.held/pressed/released(Key)` including `Key.MouseLeft/MouseRight/MouseMiddle`, `input.mouse()` (window pixels), `input.wheel()`, `input.axis`. No mouse capture, cursor hiding or mouse delta. `window.ticks_usec() -> i64`, `window.time()`, `window.delta()`, `window.size()`. Headless input reads nothing held and the mouse at (0, 0).
- **`draw3d`:** `Camera { pos, target, up, fov = 75 }`, near 0.05, far 4,000; `MeshData { vertices, colors, uvs, indices }` (no normals, no lighting); `make_mesh` needs the window open and splits meshes over 65,536 vertices on its own; `mesh(m, pos, tint)` draws at an offset; `line`, `cube`. Back faces (counter-clockwise) aren't drawn.
- **No file I/O, no environment variables or command-line arguments in Kestrel.** A program's mode is chosen by which entry file runs.
- **Tests:** `cli/tests/programs.rs` runs only `kestrel/tests/programs/*.kes` (flat). Module programs are tested by copying modules next to a generated main in a scratch folder (`cli/tests/voxels_demo.rs`, `run_beside`). `KESTREL_HEADLESS_QUIET=1` silences the per-call headless lines.
- **Source files stay under about 400 lines; error messages are plain sentences that say what to do.** Never call a bare `link.exe`.

## Layout (E1)

```
kestrel/voxel/            the engine (a Kestrel program; not a Cargo crate)
  README.md               how to run it, controls, files, what it measures
  grid.kes   noise.kes   ring.kes   pool.kes   ray.kes   collide.kes   frustum.kes      libraries
  blocks.kes  column.kes  gen.kes  mesher.kes  world.kes  stream.kes  edit.kes
  player.kes  render.kes  hud.kes                                                     engine
  main.kes                play (the window)
  bench.kes               scripted timing run (hidden window), prints stats lines
  parity.kes              the frozen benchmark's exact work, written on the libraries (no window)
  race.kes                round 1 race kernels (no window; timed by bench/kernels)
  tests/*.kes             test programs, run beside copies of the modules (E28)
kestrel/bench/engine/     measure.py, results.md (engine timings); the race's Rust side lives in bench/kernels (E26)
```

- **E1:** The folder is `kestrel/voxel/`. Files are flat because modules are found next to the file. The split into files above is the starting point; the builder may split further to stay under 400 lines, but may not merge a library into an engine file.
- **E2: A library** imports nothing from the engine (built-in modules like `draw3d` are fine) and has its own tests. Engine files may import libraries; libraries never import engine files.

## The world model

- **E3: Columns.** The world is streamed by **columns**: a column is 32 x 128 x 32 blocks (x, y, z), the full height, so a block's depth below the surface never needs another column. A column holds `LAYERS = 4` **chunks** of 32 x 32 x 32, each with its own mesh. Column (cx, cz) covers blocks x in [32 cx, 32 cx + 32), z likewise; y in [0, 128). Sideways the world is endless within `int` (practically: block coordinates stay within about ±1,000,000 so `float` vertex positions stay exact enough; meshes are chunk-local, E15).
- **E4: Blocks** are `u8` ids in a `Grid[u8]` per column: `AIR = 0`, `STONE = 1`, `DIRT = 2`, `GRASS = 3`, `SNOW = 4`. `blocks.kes` has the colour of each id and whether it is solid (all but air). The block types are decided **at generation** from depth, as the viewer colours them at mesh time today (`demos/voxels/world.kes`, `layer`): top block grass (snow from `SNOW_LINE` up), the `DIRT_DEPTH - 1` under it dirt (stone under snow), deeper stone. Once stored, a block keeps its type: digging a hole shows stone, not new grass. The placeable block is `STONE` (one type in version 1).
- **Outside the world's height:** below y = 0 and above y = 127, block types are not stored. For meshing and generation the engine uses the terrain function directly there (solid where the density is above zero, as the frozen world samples it), so the faces match the frozen world exactly (E19). For collision and rays, below 0 is solid and above 127 is air (E21).
- **Coordinates:** a block (x, y, z) is the unit cube from (x, y, z) to (x + 1, y + 1, z + 1) in world space, as in the frozen world. `column_of(x) = x >> 5` (an arithmetic shift, so negative coordinates work), `local = x & 31`.
- **Column keys:** a map key is packed `i64(cx) << 32 | (i64(cz) & 0xFFFFFFFF)` (the expected finding: no struct keys).

## Libraries (E5)

The sketches give the shape; names and exact signatures are the builder's within these rules, and each library's file header says what it is in GDScript terms where useful.

**`grid`: `Grid[T]`** (the main generics test)
```
pub struct Grid[T] {
    sx: int
    sy: int
    sz: int
    items: list[T]                        // index ((y * sz) + z) * sx + x, the frozen world's order

    pure index(self, x: int, y: int, z: int) -> int
    pure get(self, x: int, y: int, z: int) -> T
    pure set(var self, x: int, y: int, z: int, v: T)
    pure inside(self, x: int, y: int, z: int) -> bool
    pure fill(var self, v: T)
}
pub pure grid[T](sx: int, sy: int, sz: int, fill: T) -> Grid[T]     // builds items with add
pub interface Solid { pure solid(self, x: int, y: int, z: int) -> bool }
```
- Steps between neighbours: x 1, z `sx`, y `sx * sz` (as the frozen world's `STEP_X/Z/Y`). Hot loops may index `items` directly with a computed index; `get`/`set` are for the rest.
- `Solid` lives here so `ray` and `collide` share it without importing each other.

**`noise`** (not generic): `hash`, `lattice`, `smooth`, `mix`, `octave` (3D value noise blending 8 corners every `2^shift` blocks) and `value2`/`octave2` in 2D, copied from the frozen world's maths so that 3D results are **bit-identical** (same operations in the same order). The frozen world's `density` (three octaves, pulled toward air higher up) is the engine's terrain in `gen.kes`, not in the library; the library has the general pieces.

**`ring`: `Ring[T]`**, a first-in-first-out queue of fixed capacity over a `list[T]` filled at creation: `ring[T](capacity, fill: T)`, `push(var self, v: T) -> bool` (false when full), `pop(var self) -> T?`, `len`, `clear`. No allocation after creation.

**`pool`: `Pool[T]`**, fixed slots reused through a free list:
```
pub struct Handle {
    slot: int
    gen: int
}
pub struct Pool[T] {
    items: list[T]       // read and changed in place: pool.items[h.slot]; never copied out (a column is 128 KB)
    gens: list[int]
    used: list[bool]
    free: list[int]
    pure take(var self) -> Handle?
    pure release(var self, h: Handle)
    pure alive(self, h: Handle) -> bool
}
pub pure pool[T](capacity: int, fill: T) -> Pool[T]     // every slot starts as a copy of fill (E38)
```
A handle whose slot was released and taken again no longer matches (`gen` grows on each release). `take` and `release` don't allocate.

**`ray`:** `pub pure cast[W: Solid](w: W, from: vec3, dir: vec3, reach: float) -> Hit?`, stepping cell by cell along the ray (Amanatides and Woo's grid traversal), with `Hit { cell: ivec3, before: ivec3, distance: float }` (`before` is the empty cell the ray was in just before, where a placed block goes). A `dir` of length zero finds nothing.

**`collide`:** a box moving through solid cells, axis by axis (y, then x, then z, as Minecraft does), clamping each axis at the first solid cell:
```
pub struct Box {
    min: vec3
    max: vec3
}
pub struct Moved {
    box: Box
    hit: ivec3          // -1, 0 or 1 per axis, the side that was blocked
}
pub pure slide[W: Solid](w: W, b: Box, motion: vec3) -> Moved
```
A motion longer than about 0.5 blocks on an axis is split into steps so a fast box can't pass through a wall. A box that starts inside a solid cell isn't pushed out (the player code avoids it).

**`frustum`:** `pub pure frustum_of(cam: Camera, aspect: float) -> Frustum` (six planes, as six `vec4` fields, not a list, E40; from `Camera`'s position, target, up, `fov` as vertical degrees, near 0.05 and far 4,000) and `pure sees(self, min: vec3, max: vec3) -> bool` (a box is skipped only when it is entirely outside one plane; conservative).

- **E6: `ray.cast` and `collide.slide` are generic over `[W: Solid]`,** not function values: the call is direct and inlinable, contracts still work, and it is a real use of interface rules. `World` implements `Solid`.
- **E7:** library functions are `pure` where they can be (all of them except none expected), so `par for` bodies can call them.

## Engine parts

(The one-line struct shapes below are shorthand: Kestrel fields go one per line.)

- **E8: `column.kes`:** `struct Column { cx: int, cz: int, key: i64, state: ColumnState, blocks: Grid[u8] }`, with `enum ColumnState { Free, Generating, Ready }`. The 128 KB `blocks` grid is built once per pool slot at start and rewritten in place by generation (E9).
- **E9: Memory.** The pool holds `CAPACITY` columns, built at start in the program arena: every column's `Grid[u8]` exists for the whole run. Loading a column takes a free slot and generates into its grid in place; unloading releases the slot. So walking around allocates no column memory. Mesh building allocates its `MeshData` lists inside one `arena { }` per frame, freed at the end of that frame's upload. Nothing else should allocate per frame (the HUD's text uses interpolation, which the runtime frees after the call). **Every per-frame `par for` runs inside an `arena { }` (E31)**, and the bookkeeping that stores into long-lived values (queue pushes, map writes, pool take/release, edits) runs outside it.
- **E10: `gen.kes`:** `pub pure generate(var col: Column, seed: int)` fills `col.blocks` from the frozen world's density (`HEIGHT = 128`, the same three octaves, `SNOW_LINE = 88`, `DIRT_DEPTH = 3`). It decides block types top-down per (x, z) using the density for the `DIRT_DEPTH - 1` rows above y = 127. Seed 1337 by default.
- **E11: `world.kes`:** `struct World { seed, columns: Pool[Column], index: map[i64, int] (key to slot), chunk_state: list[u8] (per slot * LAYERS + layer: none, queued, meshed), queue: Ring[ChunkRef] }`, with `ChunkRef { column: Handle, layer: int }`. `World` implements `Solid` (`solid(x, y, z)`: below 0 solid, above 127 air, an unloaded column solid so nothing falls or walks into it, otherwise the stored block). `block(x, y, z) -> u8` and `set_block(x, y, z, id)` for edits.
- **E12: Streaming (`stream.kes`), per frame,** around the column the player is in, with `R = VIEW = 8` columns:
  1. **Unload** every ready or generating column farther than `R + 2` (column centre to the player's position, in columns, straight-line distance): its chunks' meshes are set to `none` (E15), its chunk states cleared, its key removed, its slot released.
  2. **Pick up to `GEN_BATCH` missing columns** within `R + 1`, nearest first, by walking a list of offsets sorted by length once at start; take a slot each and mark it `Generating`.
  3. **Generate** with one `par for` over a job list of at most `GEN_BATCH` items, each job holding its own scratch `Grid[u8]` (built once at start, E35), inside an `arena { }` (E31); then, outside the arena, copy each job's blocks into its column's pooled grid and mark the column `Ready`.
  4. **Queue chunks that became meshable:** a chunk is meshable when its column is within `R`, all 9 columns around and including it are `Ready`, and its state is none. Check the new columns and their 8 neighbours, and rescan every column within `R` whenever the player's column changes (E43).
- **E13: Capacity.** `CAPACITY` is the number of columns within `R + 2` of any point, rounded up: `(2 (R + 2) + 1)^2 = 441` for `R = 8` (56 MB of blocks). If `take` ever fails, that is a bug: the stream step prints a loud error and skips the load (tested).
- **E14: Meshing (`mesher.kes`).** `pub pure mesh_chunk(w: World, col: Handle, layer: int) -> MeshData` copies the chunk's blocks plus a one-block border (block types are stored, E4, so no extra rows for depth are needed), 34 x 34 x 34, from up to 9 columns (outside the world's height, from the terrain function), into a scratch grid; then the culled mesher with ambient occlusion from `demos/voxels/world.kes` runs on it: a face for each side of a solid block that touches air; per-corner ambient occlusion; the quad cut along the brighter diagonal; the block's colour shaded by face direction (top 1.0, x sides 0.8, z sides 0.7, bottom 0.5) and by the per-block variation (`variation`, 0.97-1.03). The output is a `MeshData` with vertices, colours and indices in **chunk-local** coordinates. Version 1 keeps that algorithm; faster meshing is the race's job.
- **E15: Rendering (`render.kes`):** `meshes: list[Mesh?]`, one per slot * LAYERS + layer, each with the column generation it was built for (a mesh whose slot was released or reused is dropped before anything is drawn, so a reused slot never shows the old column), drawn with `draw3d.mesh(m, chunk origin, WHITE)` when `frustum.sees` the chunk's box. Per frame, inside one `arena { }`: build a job list of every chunk dirtied by edits this frame plus up to `MESH_BATCH` from the queue (skipping stale refs whose handle no longer matches), mesh them all with one `par for`, then upload each with `make_mesh` into its slot (an empty `MeshData` stores `none`), set its state to meshed, and leave the arena. Meshing and upload are in the same frame, so the arena holds one frame's data at most.
- **E16: Editing (`edit.kes`).** The eye ray is `ray.cast(world, eye, look, REACH = 6)`. **Left click** (`pressed(MouseLeft)`) sets the hit cell to air. **Right click** places stone in `hit.before` if that cell is not inside the player's box. While the mouse isn't captured (E34), a right-button press counts as a click only if the mouse moved less than 4 pixels between press and release (a drag is looking, as in the viewer). An edit dirties every chunk whose 34-block meshing region contains the cell (1 chunk in the middle, up to 8 at a corner, only chunks that are meshed or queued and whose columns are loaded); they are re-meshed in the same frame (E15). Edits are forgotten when their column unloads (no saving in version 1; a finding).
- **E17: Player (`player.kes`).** Two modes, toggled by `F`:
  - **Walk:** box 0.6 x 1.8 x 0.6, eye at 1.62; WASD at 4.3 blocks a second along the look direction flattened to the ground; gravity 32 blocks/s²; `Space` jumps (9 blocks/s up) when standing; falling speed capped at 60 blocks/s; `collide.slide` against the world; standing = the y motion was blocked from below.
  - **Fly:** the viewer's free-fly camera (`demos/voxels/fly.kes`: WASD, Space/Shift, wheel for speed 2-400), no collision.
  - **Looking (E34):** with the mouse captured, moving the mouse looks (as in a first-person game); `Tab` toggles capture, and the window starts uncaptured. Uncaptured, right mouse drag looks, as in the viewer.
  - **Spawn:** fly mode above the spawn column until it is ready, then walk mode standing on its top solid block. `dt` is capped at 0.1 s; physics steps of at most 1/60 s.
  - **E18:** the player takes an `Intent` (move, look turn, jump, wheel, break, place, toggle) built either from `input` (`main.kes`) or from a script (`bench.kes`), so the scripted runs use the same code as play.
- **HUD (`hud.kes`):** a crosshair at the centre, the targeted block's 12 outline edges (`draw3d.line`), and a text line: fps, frame ms, loaded columns, meshed chunks, faces, queue length, mode, position.

## Entry programs

- **`main.kes`:** opens a 1280 x 720 window, runs the frame loop: input to intent, player, edits, stream, mesh and upload, draw, HUD. Prints a stats line once a second (like the viewer).
- **E18b `bench.kes`:** the same loop in a hidden window (`KESTREL_HIDDEN=1` is set by whoever runs it) driven by a fixed script, printing one machine-readable line per phase and one summary line. Phases: (1) standing start until the view distance is filled (queue empty, every column within `R` meshed); (2) fly straight at 20 blocks/s for 20 s; (3) at 100 blocks/s for 20 s; (4) 50 breaks and 50 places at chosen cells, half in chunk middles and half on chunk corners; (5) a long flight in a circle at 50 blocks/s for 300 s (the memory check; `measure.py` samples the working set from outside). Per phase: frames, frame time median / 99th percentile / worst, columns generated, chunks meshed, mesh and upload ms, and for edits the time from the click's handling to its upload. Phase lengths are constants so a quick run can shorten them. Time base and pass mark: E32. `bench.kes` never captures the mouse.
- **E19 `parity.kes`:** the frozen benchmark's exact work (`bench/voxels3d/voxels3d.kes`: 8 x 4 x 8 chunks, each sampling its own 34^3 padded solids, meshed into vertices, normals and indices, seed 1337), written on `grid` (`Grid[u8]` instead of `[u8]`) and `noise`, in one thread. Prints `463749`. It is the "generics cost nothing in real code" measurement: timed against the frozen `main.kes`, both `--release`, taking turns. **Target: within 5%.** It also has a `par for` version like `main_par.kes`, and a variant that reuses one scratch grid with `fill` instead of building a fresh one per chunk (E36). No window, so it is timed from outside like the frozen `run.ps1` (whole process, an empty program's startup subtracted).
- **E20 the engine's own parity:** a test builds the same region through the engine's pipeline (columns -1..8 by -1..8 generated, the chunks of columns 0..7 meshed by `mesh_chunk`) and must count 463,749 faces (the colours are not compared to the viewer's: block types are now stored at generation, which gives the same colours for an unedited world but goes through different code).
- **E21 `race.kes`:** the race kernels (E26), exporting `pub bench(n, iters)` for `bench/kernels`' timer.

## Tests (E28)

- **E28:** a new `cli/tests/voxel_engine.rs` runs every `kestrel/voxel/tests/*.kes` beside copies of all of `kestrel/voxel/*.kes` in a scratch folder (as `voxels_demo.rs`'s `run_beside` does), with the program tests' `// expect:`, `// expect-exit:` and `// expect-stderr:` lines, JIT and exe (and `--release` where a test says so). The directive parsing is shared with `programs.rs` (moved into `cli/tests/common`), not copied. `KESTREL_TEST_ONLY` filters these too.
- Tests cover: `Grid` (index order, get/set, inside, fill, with `u8`, `int` and a struct); `Ring` (wrap-around, full, empty); `Pool` (take until full, release, stale handles); `noise` 3D bit-identical to the frozen world (E33: the test copies `bench/voxels3d/voxels3d.kes` into its scratch folder, makes the copy's `density` public by a text replacement in the copy only, and compares it with `gen.kes`'s density at a few hundred points, including negative coordinates, printing both as exact bits or with `==`); `ray` (hits along each axis and diagonals, from inside a cell edge, misses, reach); `collide` (falling onto a floor, sliding along a wall, a corner, a fast fall that must not tunnel, a ceiling); `frustum` (boxes in front, behind, beside, straddling); generation (block types by depth; deterministic); the engine parity (E20, 463,749 faces; it may run only JIT and `--release` if the exe run takes too long); streaming without a window (a simulated walk: the loaded set, never out of slots, no chunk queued twice, unload releases); edits (the dirtied chunk sets for a middle, edge and corner cell; break then place restores the block; the face count after an edit).
- `main.kes` gets a headless smoke test (`KESTREL_HEADLESS=<frames>` with `KESTREL_HEADLESS_QUIET=1`), checking its stats line. No GPU, window or demo timing in `cargo test`.
- Error tests only if the build adds a compiler error message.

## Measurements (`kestrel/bench/engine/results.md`)

As in the other results files: medians and ranges, rounds taken in turn when comparing, the machine's load stated, losses reported.

- **Parity (E19):** `parity.kes` against the frozen `main.kes` and `main_par.kes`, checks on and `--release`, 11 rounds in turn. Also its `kestrel check --report` against the frozen one's (how many checks are left where, and why).
- **Engine (from `bench.kes`):** per-chunk generate, mesh and upload times; faces and vertices per chunk; the five phases' frame-time median / 99th percentile / worst; columns per second at 20 and 100 blocks/s; time to fill the view from a standing start; edit-to-upload latency (middle, corner); working set at start, after filling, and over the 300 s flight (sampled by `measure.py`), with the runtime's arena count where useful.
- **Contracts:** `@prove(no_alloc)` and `@prove(no_checks)` on the inner loops that can carry them (mesher, generation, `ray`, `collide`, `Grid` access), with `kestrel check --report --release`; every check that remains listed with its reason. A contract that can't be proven fails the build, so it can't stay in the engine: it becomes a finding and a probe program (E37).
- **Compile time** of `main.kes` (`kestrel build`, `--release`, `kestrel check`), 9 runs, median and range.
- **Lines of code** per file, and the findings count.

## The race against Rust, round 1 (user: `block-mesh`)

- **E26:** the race uses `bench/kernels`' timer and `measure.py` (they already take turns between Kestrel and Rust and report ns per item). The Kestrel side is `kestrel/voxel/race.kes` (so it can use the libraries; the timer takes a file path). The Rust side is a new mode in `bench/kernels`' Rust binary with `block-mesh` (0.2, with `ndshape`) as a dependency of that workspace only (it is excluded from the main workspace already). A small Rust port of the frozen world's noise builds the same inputs (the earlier straight port matched 463,749 faces, so this is known to work).
- **Inputs (not timed):** the 256 chunks of the frozen world's region as 34^3 padded solid flags, seed 1337, built once before timing on both sides. Both sides check a total of 463,749 faces.
- **Kernel 1, faces:** padded solids to the list of visible unit faces (each face's block and side), against `block_mesh::visible_block_faces` into a `UnitQuadBuffer`. Kestrel writes six lists of packed ints (one per side, like the buffer's groups).
- **Kernel 2, mesh arrays:** faces to positions, normals and indices (4, 4 and 6 per face), against `block-mesh`'s `quad_mesh_positions`, `quad_mesh_normals` and `quad_mesh_indices`, into vectors cleared and reused per chunk (Kestrel: lists in an `arena { }` per chunk, or reused; the builder states which, and both are fair if said).
- **Measured:** ns per chunk, median of rounds, one thread, `--release` (and Kestrel checks on as a second line), Rust `--release` (and with overflow checks), on the same machine in turn. Also the existing straight Rust port of the whole frozen world build against the tuned and frozen Kestrel copies, if `C:\kt\voxtuned\rust` still builds (else say so).
- **Round 2 (after version 1, not in this plan):** greedy meshing in Kestrel against `binary-greedy-meshing`.

## Findings (E30)

`docs/superpowers/plans/2026-10-09-core-2a-followups.md` gets a section "Found while building the voxel engine (2026-10-25)". Each entry: what the engine needed, the workaround used (file and line), how much it cost (in lines, speed or clarity, measured where possible), and the general fix it points at, marked as syntax (the user's) or not. The expected ones, to be confirmed or dropped with evidence:
- column keys packed into an `i64` (no struct map keys);
- meshes outside the pool, in `list[Mesh?]` (an owning type can't be `T`; a map can't hold one);
- `Grid[T]` over a `list[T]` built with `add` (no `array[T](n)` in generics, no fill constructor); whether its indexing keeps bounds checks a `[u8]` would lose (range facts after `add`);
- per-frame batches instead of streaming work (no `await` or jobs): the hitches measured in phases 2-3;
- mouse capture needed two new natives (E34);
- generic stores refused inside `arena { }` (E31): `Grid[u8].set` on a pooled column, `Ring.push`, a function calling `Grid.set` from a `par for` in an arena are refused because the check assumes any `T` holds memory, though a `u8` holds none; the workaround writes `items[i]` directly;
- a `par for` outside an `arena { }` leaks its thread arenas into the program arena every call, uncounted by `KESTREL_ARENA_COUNT` (measured by the builder check: about 1.3 KB per call on 16 threads);
- the `par for` scheduler's fixed chunks spread a few busy items in a long list badly (E35);
- edits lost on unload (no files);
- anything else found.

New natives are allowed only if the engine can't work without them; each one is listed here with its name. Version 1 adds the two of E34 and is expected to need no others.

## Docs

- `kestrel/voxel/README.md`: how to run (`kestrel run --release voxel/main.kes`), the controls, the files and which are libraries, what the entry programs are.
- The main `README.md` gets a short "Voxel engine" pointer; the handoff's "Start here" is updated at the end of the round.
- No syntax-spec change is expected (no new syntax).

## From the builder check (2026-10-25)

An Opus builder check read this spec against the repo and probed the claims with small programs. Its seven points became these rulings (all Claude's; none needed the user):

- **E31: every per-frame `par for` runs inside an `arena { }`.** A `par for` adopts its thread arenas into the current arena (`runtime/src/par.rs:117-129`); outside any block that is the program arena, so each call leaks (measured: an empty `par for` over 64 items, 400,000 calls, 544 MB working set, while `KESTREL_ARENA_REPORT` stayed at 65,536 bytes; these arenas are uncounted, `arena.rs:183`). Inside an `arena { }`, a generic store into a long-lived value is refused (`Grid[u8].set` on a pooled column, `Ring.push` on the world's queue: the check assumes any `T` holds memory), so: generation and meshing go inside the arena and write `items[i]` directly; queue pushes, map writes, pool take/release and edits go outside it. **"Allocates nothing once warmed up" is measured as no growth**: the program arena's size is the same after the fill and after the 300 s flight, and the working set rises less than 5% over the flight's last 200 s. The runtime's spare-block cache (up to 256 MiB per thread, freed worker blocks landing on the main thread) will likely make the working set climb early and then level off; that plateau is reported as the known runtime gap, with its size, not hidden. If the plateau is large, the builder may make mesh jobs reuse their lists in place (then report both).
- **E32: time base.** Scripted runs (`bench.kes`, tests) step the simulation by a fixed 1/60 s per frame, so a run follows the same path and does the same work whatever the display does; `main.kes` uses the real `window.delta()`. Each frame's **work time** is `window.ticks_usec()` from the frame's start to just before `window.frame()` (real in a hidden window; fake in headless, so headless is never used for timing). The wall time per frame is reported too, labelled as possibly capped by vsync. **"60 fps while streaming" passes when the 99th-percentile work time is at most 16.7 ms**; the median and the worst are reported. Streaming speeds are per simulated second. `parity.kes` and `race.kes` have no window: `parity` is timed from outside, `race` by `bench/kernels`' own clock.
- **E33: the noise bit-identity test** edits only its scratch copy of `bench/voxels3d/voxels3d.kes` (making `density` public); the frozen file is never changed.
- **E34: mouse capture is added** (the approved proposal: "small `input`/`window` natives get added as the engine needs them, each one recorded"): `input.capture(on: bool)` (raylib `DisableCursor`/`EnableCursor`) and `input.mouse_delta() -> vec2` (`GetMouseDelta`). Headless or closed: `capture` does nothing and `mouse_delta` is (0, 0). Their names are Claude's and go to the user with the hand-over, who may rename them. Tests: the window crate's symbol counts, the declarations, the headless behaviour. `bench.kes` and every test never capture.
- **E35: generation runs as jobs.** A `par for` hands out items in fixed runs of `count / (threads * 8)` (`par.rs:181`), so a few `Generating` columns among 441 slots would land on 2 or 3 threads. So generation iterates a job list of at most `GEN_BATCH` items, each with its own scratch `Grid[u8]` built at start, and the main thread copies a finished job's blocks into the pooled column (128 KB, cheap next to about 5 ms of noise). The scheduler's behaviour is a finding.
- **E36: parity in two variants.** Filling a 34^3 `Grid[u8]` with `add` costs about 16 ms over the 256 chunks against about 1 ms for `array[u8](n)` (measured by the builder check), about 3% of the frozen world's time before any generics cost. `parity.kes` keeps the frozen program's exact work (a fresh grid per chunk; the 5% target applies to it) and also runs a variant reusing one grid with `fill`; the results give both, and the difference is the cost of the missing `array[T](n)` in generic code.
- **E37: contracts.** `@prove` annotations that hold stay in the engine's code. One that can't be proven would fail every build, so it moves to a probe program `kestrel/voxel/tests/contract_<name>.kes` that expects exit 1 and the exact contract error (`// expect-exit: 1`, `// expect-stderr:`), and to the findings; if a later compiler proves it, that test fails and says so. The E28 runner supports `// expect-stderr:` for this.

## From the plan review (2026-10-25)

An Opus review of the plan, with probe programs, found 2 critical and 8 important problems; the plan was rewritten with these rulings (Claude's):

- **E38: `pool[T](capacity, fill: T)`** replaces `pool[T](items)` (E5): building the items list and passing it copies every column again, and `grid`'s doubling leaves old buffers behind, measured at about 4 times E9's memory (300 columns took 162 MB; with one template column added `capacity` times, 100 took 13.5 MB).
- **E39: a contract stays only if both builds prove it.** Contracts are decided per build: `@prove(no_checks)` on `a + b` passes `check --release` and fails the checked build. So an annotation stays in the engine only if `kestrel check` and `kestrel check --release` both pass. A loop over `g.items` is expected to need an `@assume` on the grid's length ("nothing shows it is below `g.items.len`"), which is allowed and is a finding.
- **E40: nothing allocates per frame outside the per-frame arenas.** The frame loop can't run inside one arena (the generic stores are refused there), so a per-frame allocation outside the per-frame arenas grows the program arena: `Frustum` has six plane fields, not a list (a list built per frame grew 5 MB over the flight); bench buffers are sized before the run and written by index; a column's grid is copied item by item (a whole assignment allocated 128 KB per load). E31's "no growth" is read from `KESTREL_ARENA_REPORT` at exit (it only reports then), comparing a 100 s and a 300 s flight; the memory curve uses `mark` lines from `bench.kes`.
- **E41: no 2D noise in version 1**: nothing uses it, so it would not be grown from the engine.
- **E42: parity's verdict line indexes `g.items` with the frozen program's own arithmetic** (so it measures list against array plus `grid[T]`); a second line goes through `get()` (the generic method); a third reuses one grid with `fill` outside the per-chunk arena (inside it, `fill` is refused).
- **E43: meshability is rescanned when the player changes column.** Columns unload at `R + 2` but load at `R + 1`, so a column can come within `R` with all its neighbours already loaded, and checking only around new columns left up to 16 chunks unmeshed on a circular flight (simulated). A queue push that fails leaves the chunk `CHUNK_NONE` and is counted (`queue_full`), so it is retried, never lost.
- **E44: a popped ref** whose column is gone is dropped; one whose chunk is no longer meshable (a neighbour unloaded) resets to `CHUNK_NONE`, so a later check queues it again. Without this a skipped chunk stayed queued for ever.
- **E45: spawn** is `player.spawn`, called each frame until the player's column is Ready, then standing on the top solid block in walk mode. `edit_at(world, cell, id)` takes an exact cell (the bench's corner edits), and `apply_edit` uses it after its ray. The bench sets its speeds directly and flies a circle of radius 1,000 blocks.
- Also folded in: test programs are written as `test_<name>.kes` in the scratch folder (names like `grid.kes` would clash with the modules, and `main.kes` with the play program); a build that fails is compared with `// expect-exit:`/`// expect-stderr:` (contract probes fail at build time); a `// replace:` directive does E33's edit and the bench test's shortened phases; the race's `bench` returns a `float` and builds its inputs inside an `arena { }` per call (there are no global variables); per-column generate time is reported (`gen_us`); tests were added for a reused slot's mesh, stale and unmeshable refs, flying at 400 blocks/s, collision across x = 0 and z = 0, standing at y = 0 and jumping under the top of the world.

## What's left to the builder

- Exact names and signatures inside the shapes above, file splits beyond E1, the script's exact path and edit cells, the constants `GEN_BATCH` and `MESH_BATCH` (start at 8 and 16, then tune from phase 2-3's frame times and say what was picked), the HUD's look, and how race kernel 2 manages its lists (stated in the results).
- Where the language fights the design, the builder may change the design to fit, records the change here under "As built" with the reason, and adds a finding. It may not add syntax.

## As built (E46 on)

The rulings made while building (ledger `.superpowers/sdd/2026-10-25-voxel-engine/progress.md`), in the ledger's order. Each is Claude's; "cost if wrong" is what changes if it is overturned.

- **E46:** `main.kes`'s smoke test is a Rust test in `cli/tests/voxel_engine.rs` that runs the real `main.kes` headless (a `tests/*.kes` program is written as `test_<name>.kes` and can't run another program's `main`). Cost if wrong: one moved test.
- **E47:** the directive parser's tests stay in `cli/tests/programs/directives.rs`, not `common/` (which is compiled into about 20 test binaries). Cost if wrong: tests move later.
- **E48:** each engine test gets three scratch folders (`voxel_<name>`, `_exe`, `_release`): the passes run in parallel and would clear each other's. Cost if wrong: none.
- **E49:** the release pass compares standard output, exit code and standard error like the other passes (later tasks rely on `// release` output). Cost if wrong: a slower release pass.
- **E50:** Tasks 2 and 3 went to one implementer, and Tasks 2-3 and 7-8 had no per-task review (reviews only on risky tasks, the user's standing preference); the final review covers them. Cost if wrong: a defect found later.
- **E51:** `generate` reads `DIRT_DEPTH` (3) rows above y = 127, not `DIRT_DEPTH - 1` as E10 says: the viewer's chunk top sees its border row plus `DIRT_DEPTH - 1` more, so 3 rows types the top of the world as the viewer types a chunk top. Cost if wrong: only top-row block types.
- **E52:** streaming loads the columns within `view + 1` **and every neighbour of a column within `view`** (269 columns at `VIEW = 8`, not 253): `view + 1` alone leaves 16 edge columns' neighbours unloaded, so their chunks are never meshable (E12's 9-column rule). The farthest is 9.93 columns from the player, inside the `view + 2` unload line; `CAPACITY` 441 holds. The margin fails for `VIEW` 5, 9 and 10 (load/unload thrash); `stream.kes` warns. Cost if wrong: a different `VIEW` can thrash.
- **E53:** a queue push refused because the queue is full makes the next stream step rescan (it forgets the player's column), so the chunk is retried even while the player stands still (E43's "retried, never lost"). Cost if wrong: one extra rescan per step while the queue is full.
- **E54:** Tasks 7 and 8 went to one implementer while Task 6's read-only review ran (they share no files with Task 6). Cost if wrong: a fix round for Task 6 waits.
- **E55:** the two natives of E34 were built with the names the spec gives (`input.capture`, `input.mouse_delta`); the user is told, not asked, and may rename them. Cost if wrong: a two-name rename.
- **E56:** `ray.cast`'s start is named `origin` and `Frustum.sees` takes `(self, lo, hi)`, not `from`/`min`/`max`: `from` is a keyword and `min`/`max` built-in names, which can't be parameter names. Callers pass them by position. Cost if wrong: none.
- **E57:** a ray cast from inside a solid block hits at distance 0 with `before` the same (solid) cell; `apply_edit` refuses to place when the hit distance is 0 or `before` is solid. Cost if wrong: placing could overwrite a solid block.
- **E58:** `mesh_step` takes two more parameters, the player's position `at` and `view` (E44's "beyond view" needs them): `mesh_step(var r, var w, dirty, batch, at, view)`. A popped ref beyond view resets to `CHUNK_NONE`. Cost if wrong: an interface change for its callers.
- **E59:** `spawn` checks every cell under the box's footprint (up to 4 columns), not one cell, and stands on the highest top among them, so a player straddling a column edge can't spawn inside a block. Cost if wrong: none.
- **E60:** a popped ref is skipped when its chunk is no longer `CHUNK_QUEUED` (meshed from an edit this frame, or reset), so no chunk is meshed twice. Cost if wrong: a chunk missed until it is queued again.
- **E61:** a dirty ref for a meshed chunk that is no longer meshable (a neighbour column unloaded) resets it to `CHUNK_NONE` and keeps its old mesh, so no hole appears; the edit shows once the stream queues it again. Cost if wrong: an edit beside an unloading column stays invisible until then.
- **E62:** two small fixes from Task 9's review were done in Task 10 (the smoke test fails on a contract error; `main.kes`'s unused imports removed). Cost if wrong: none.
- **E63:** Task 10 (bench, measurements, findings) ran on Opus. Cost if wrong: token cost.
- **E64:** Task 10 measured without the user confirming the machine was free (the user was away); the load is recorded beside every number. Cost if wrong: noisier numbers, re-run later.
- **E65: `GEN_BATCH` is 2, not 8** (the tuning the spec leaves to the builder, from phase 2-3's frame times). One column takes about 7.5 ms to generate on one core, and a batch's columns run side by side, so every frame that generates costs at least that plus the copy into the pool, the meshing, the upload and the drawing. At 8 the 99th-percentile work time was 17.5, 19.9 and 19.7 ms in `fly20`, `fly100` and `circle` (E32 fails); 2 is the largest batch that passed with a margin in the tuning runs (3 passed by 0.3 ms). It still generates up to 120 columns a simulated second (flying at 100 blocks a second needs about 60). Cost: the view fills in 135 frames instead of 51 (2.25 s instead of 0.85 s at 60 fps). `MESH_BATCH` stays 16. Revisit after the inliner-budget fix: with the noise inlined, generation is 1.65 times faster (`results.md`, "Contracts"), which may let a larger batch pass. Cost if wrong: back to 8, a faster fill and frames over budget while streaming.
- **E66: `bench.kes` as built.** The script sets the player's speed, its height at the start of each flight (y = 100, so it flies over the hills: fly mode has no collision) and its heading on the circle directly; everything else goes through an `Intent`. Edits are timed with `mesh_step`'s batch at 0 (only the edit's chunks), break and place back in the same frame, one cell a frame; the cells are the middle of the second chunk (y 48) and the corner where the first two chunks meet (y 32) of the 5 x 5 columns around the player, after the view has filled again. It prints a `fill_ms` line (the fill's real time) besides the brief's lines. Its sample buffers are sized for 18,000 frames whatever the phases' lengths, so `measure.py`'s 100 s build has the same buffers as the 300 s one (sized from the circle's length, the two builds' program arenas differed by 983,040 bytes, the buffers alone). Cost if wrong: none (bench only).
- **E67:** `race.kes` writes out `gen.solid_at` (the density test) instead of importing `gen` or `blocks`: they import `draw`, whose natives `bench/kernels`' timer JIT can't link. The 463,749 face total and the two checksums, equal on both sides, pin that the input is the same. Cost if wrong: none (input verified).
- **E68: `mesh_chunk` as built (Task 5), two departures from E14.** It takes the column's slot, `mesh_chunk(w: World, slot: int, layer: int)` (`mesher.kes:175`), not `col: Handle`: its callers (`mesh_step`, the tests) have the slot already, and a handle would be checked again for nothing. And the padded 34^3 copy holds 0/1 solid flags (`array[u8]`), not block types: the faces and ambient occlusion need only solidity, and the block's type, for its colour, is read from the column's grid for each block that has a face. Cost if wrong: a signature change for two callers.
- **E69 (final review, B1): unloading a column resets its 8 neighbours' meshed chunks to `CHUNK_NONE`** (`stream.kes`, `forget_neighbour_meshes`), keeping their meshes drawn until new ones replace them (as E61). Their meshes read one block into the unloaded column, which loads again without its edits (E16); before this, a neighbour that stayed loaded (beyond `view`, within `view + 2`) kept a mesh built against the edited blocks, a lasting hole or a floating face (reproduced headless; test `tests/stream_return.kes`). Queued chunks are left alone (the renderer checks their neighbours when it pops them, E44). Cost: a chunk near the unload ring is meshed again when it comes back into view; the bench's scripted phases mesh the same number of chunks (they fly away from what unloads). Cost if wrong: that extra meshing.
- **E70 (final review, B2): the engine's mesher treats y >= 128 as air**, as `World.block` does (so the player stands on it and the ray passes through it), instead of taking that row from the terrain function as E14 and the frozen viewer do. With the terrain's row, the top face of every y = 127 block under solid terrain was left out: about 96% of the y = 127 blocks (1,827 of 1,909 in the frozen region), so every cut-off peak had a see-through floor the player stood on. Rows below y = 0 still come from the terrain function (visible only from under the world). E20's engine parity now counts 465,576 faces, 1,827 more than the frozen 463,749, every one such a top face (`tests/engine_parity.kes`); the frozen count stays pinned by `tests/parity.kes` (the frozen work on the libraries) and the race. Flagged to the user (a departure from the frozen viewer's picture). Cost if wrong: a one-line revert and the old pin.
- **E71 (final review, C1): the race also runs Rust built for the host CPU.** Kestrel compiles for the machine it runs on (`cranelift_native`, `codegen/src/isa.rs:18`), while the race's Rust was built for the baseline `x86-64` target, which tilted the comparison toward Kestrel. The Rust modes are now also built with `-C target-cpu=native` (`--target-dir target/native`, `measure.py --rust-native`) and both are reported (`results.md`, "Race, round 1"): Kestrel `--release` takes 1.39 and 1.84 times the baseline build's time and 1.36 and 1.78 times the native build's (native Rust measured 2 to 4% slower here). The same review made the plain Rust port push each of a face's 14 entries one at a time, as Kestrel `add`s them (it had appended normals and indices with `extend_from_slice`). Cost if wrong: one extra build and column.
- **E72 (final review, A1): `Grid.get` and `Grid.set` carry no `@prove(no_alloc)`.** A generic function's contract is checked for each type it is made for, so the library's `no_alloc` failed every program that made a `Grid` of a type owning heap memory (`grid[string]`, a struct holding a string): `get` copies the value. It is proven for `Grid[u8]` instead, in `tests/grid.kes` (a `@prove(no_alloc)` function calling both), and `gen.generate`'s contract covers the engine's hot path (it indexes `items` directly). Cost if wrong: none (the engine is unaffected).
