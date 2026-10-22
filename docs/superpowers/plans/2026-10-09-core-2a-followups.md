# Plan 2a follow-ups

Carried forward from plan 2a's task reviews and final whole-branch review.
Input for writing plans 2b and 2c.

## Language decisions (resolved 2026-10-10)

- **Narrowing integer conversions** (`u8(x)`, `int(x)` from a wider type) panic when the value doesn't fit and checks are on; `--release` truncates. A literal that doesn't fit is a compile error. Float->int still saturates.
- **`while true` with no `break`** counts as never finishing, so it can end a value-returning function.

## Do before or at the start of 2b

Done in 2b-i: labelled arguments evaluate in source order; `func/mod.rs` split; parenthesised spans include the parens; KIR has `Place` + aggregates (struct layouts, return slots, field-by-field copies); compound assignment takes its type from the place; exclusivity works on place roots. Recursion depth limits were done on 2026-10-17 (below). Still open from this list: interned `Ty` ids for enums, `Diagnostic` labels, `match` arm newline note, `list`/`array`/`map` type constructors, thin KIR-shape tests.


- **Labelled arguments evaluate in parameter order, not source order** (`kir/src/lower/expr.rs` `call_parts`). The ignored test `labelled_arguments_evaluate_in_source_order` in `codegen/tests/jit.rs` documents the expected behaviour. The fix needs synthetic temporaries, and the locals list must be able to grow during lowering. Struct construction with labels will hit the same problem.
- **Split `codegen/src/func/mod.rs`** (378 lines) before adding aggregates: move print/string/call helpers into their own files.
- **Parenthesised expression spans exclude the parens**, which misplaces carets on parent nodes. Fix when postfix `.`/`[]` parsing lands.
- **Recursion depth.** Done (2026-10-17): the parser refuses code nested more than 1,000 levels deep (parentheses, blocks, unary operators, operator and postfix chains, types), so the checker, lowering and codegen never recurse deeper. The CLI and runtime still run on 256 MiB stacks.
- **KIR needs places and aggregates.** `Stmt::Assign(LocalIdx, _)` and `Arg::Ref(LocalIdx)` can only target a local. Fields and indexes need a `Place` (Local/Field/Index/Deref). `KTy`, `cl_type` and codegen's `Slot` model are scalar-only, and `expr() -> Value` can't return aggregates (needs destination-passing). `LocalKind::RefParam` is tied to `var`, but spec §2 also passes read-only aggregates by pointer.
- **`Ty` needs interned ids** for structs/enums. `Ty::name() -> &'static str` and `resolve_type` must take the type table, and `collect` needs a two-phase declare. The AST has no ids for patterns, field labels or variant names yet.
- **`Diagnostic` needs secondary labels** (spec §3; the arena-escape error names two lifetimes).
- **Parser notes for `match` and generics.** `newlines.rs` drops a newline before `else` and `}`, so `match` arms like `Air { false }\n else { true }` arrive without a separator, and arm parsing must not require one. `array[int](n)` / `list[Chunk]()` look like indexing followed by a call, so treat `list`/`array`/`map` followed by `[` as type constructors.
- **Compound assignment** takes the operand type from `rhs.ty`. It should come from the place once `Place` exists.
- **Thin KIR-shape tests** (while/for/break/continue/and/or/unary/convert). Add them before refactoring the KIR shape.

## Found while building the Godot demo (2026-10-10)

- **A `pure` function can't call a helper that only changes a `var` argument rooted at its own local** (for example `quad(var out, ...)` filling a local list). This is observationally pure, but the checker rejects it because the helper has `MutatesParams`. Allow calls whose `var` arguments are all rooted at the caller's locals. Resolved (found 2026-10-12): it has worked since 4073379 when the helper is `pure` (`tests/programs/pure_var.kes`); this note was written earlier the same day. A helper that isn't marked `pure` stays refused by design.
- **`let x = opt else` always moves or copies.** Unwrapping an owning optional parameter is therefore an error; a view (like a `match` binding) would fix it. Done in the first language pass (2026-10-12): it reads in place when the optional is a place that can't change; `var`, a `var` root, or a whole owning local still move or copy.
- **Runtime errors don't name the file** in multi-file programs, only the line. Done in the first language pass: "on line 24 of geom.kes".
- **The Godot add-on is built and tested on Windows only.** The `.gdextension` file lists Linux and macOS library names, but they've never been built.

## Found while building strings anywhere (2026-10-10)

- **Read-only copies of strings copy their bytes.** `let w = words[i]` copies, which makes `bench/maps` string keys about 35% slower. A copy that is only read could share the bytes, once the escape check models it. This includes copies of literal-backed strings: a static-data check would skip them, since literals belong to no arena.
- **A `return` inside `arena { }` can dangle.** The value is an `if` whose branch has an owning local, and the result reads a place declared outside the arena. `block_value` always uses `owned(v, false)`, but the checker's `region(.., moving = true)` assumes a returned local isn't copied. Example: `return if c { let o = Own(); x } else { "lit" }`. Fix: make the two agree on which values `block_value` copies.
- **Binary vector operators don't evaluate operands in source order.** `p.pos + shift(var p)` reads `p.pos` after the call, and binary operators have no changed-later guard. Vector methods have the guard.
- **A literal or plain-local index is still copied before a vector operation** (`vs[0].length()`). `operand` copies any indexed vector place so the index is evaluated once. This is safe but costs a copy for indexes that have no effects.
- **Precision gotchas in the new builtins.**
  - `let x: f64 = sqrt(2.0)` computes in `float`, then widens.
  - An `i64` argument to a float builtin goes through `float`.
  - Named float constants widen from `float` (`let HALF = PI / 2` is not exact against an `f64`).
  - Builtin calls don't receive the expected type, so `let b: u8 = min(7, small)` fails while `min(small, 7)` works.
- **Vector methods aren't allowed in `let` constants** (`let L = vec2(3, 4).length()`), while `sqrt`, `max` and `lerp` are.
- **Cost of the trig and exponential functions.** `round` is a short inline instruction sequence. `sin cos tan asin acos atan atan2 exp log pow` each call their own runtime function (`kestrel_rt_sin`, `kestrel_rt_cos`, `kestrel_rt_tan`, `kestrel_rt_asin`, `kestrel_rt_acos`, `kestrel_rt_atan`, `kestrel_rt_atan2`, `kestrel_rt_exp`, `kestrel_rt_log`, `kestrel_rt_pow`), so a hot loop full of `sin` pays for the call.
- **The random generator is per thread.** `rand_seed` seeds the calling thread only. Random calls are an error in `pure` functions and `par for` bodies, so this only matters if they are ever allowed there (say, one stream per iteration for repeatable parallel noise).

## Found while building the window (2026-10-10)

From plan 2's reviews and the particles demo. The window spec is `docs/superpowers/specs/2026-10-10-window-design.md`; the demo numbers are in `kestrel/demos/particles/results.md`.

### Next steps

- **Textures, 3D and the voxel viewer** are the next spec (the window spec's "Not in this spec" list also has audio, gamepads, fonts and a windowed exe). Done in plans 1-3 of the 3D work (2026-10-10 and 2026-10-11): `docs/superpowers/specs/2026-10-10-3d-textures-design.md`, with "As built (plans 2 and 3)" at its end. Audio, gamepads, fonts and a windowed exe are still not built.
- **Bulk drawing.** The demo's draw time is one native call per particle (about 150-190 ns each: 8.7 ms for 50,000 and 30 ms for 200,000). It is the bottleneck, not `par for`. A call that takes a whole list (`draw.rects(positions, size, color)`) would skip the per-call work, and a textured batch would be the real answer. Done in plan 2 of the 3D work: `draw.rects` draws 200,000 particles in 4.6 ms, against 28-29 ms with one `draw.rect` each (`kestrel/demos/particles/results.md`). What is left of the cost is in the plan 2 section below.

### Platforms and build

- **Only Windows is built.** raylib's `build.rs` and the `kestrel build` link line name Windows libraries. On another platform the window crate fails with C compile errors, and a window program's link fails with undefined symbols, instead of a plain sentence. Say so up front.
- **The window must move to the main thread for macOS.** The window runs on the CLI's big-stack thread. GLFW needs the main thread on macOS; Windows is fine with any single thread that makes all the calls.
- **raylib compiles twice on a cold build.** `cli/build.rs` builds the two staticlibs in a nested cargo build with its own target directory, so raylib is compiled there and again for the workspace's own `kestrel-window` build. Build it once, or share the directory.
- **A missing window library error is only unit-tested.** `kestrel build` of a window program without `kestrel_window.lib` has a plain message; nothing runs it end to end.

### Language and checker

- **Natives' runtime errors don't name the line**, only the function ("draw.circle was called before window.open"). The call site's line isn't passed to the native. Done in plan 1 of the 3D work (Task 6): a call from the program's own files to a built-in function that isn't `pure` stores its line first, and a native's error ends with "on line N".
- **Hoist heap constants: decide in the 3D spec.** Done in plan 1 of the 3D work (Task 4): an array, list or map constant (or one that holds one) is built once, before `main`, and every use reads that one value in place; storing it copies. Original note: an array, list or map constant (or a struct constant that holds one) is re-lowered and re-allocated at every use (`kir/src/lower/expr.rs`, `const_value`). Measured: `let PALETTE = [draw.RED, draw.GREEN, draw.BLUE]` with `PALETTE[n % 3]` in a frame loop grew the program arena by 4.78 MB over 100,000 frames. Today the window spec tells the user to wrap the loop body in `arena { }` or copy the constant into a `let` before the loop. A real fix builds each such constant once (read-only, never freed, shared), but then a copy of it must still copy, and that is a design question for when 3D programs hold meshes and palettes in constants.
- **Struct-valued field defaults are unavailable** (`tint: Color = RED` is an error: "field defaults can't use struct values yet; set the field when you build it"). Constants may construct structs, and field defaults may construct only `vec2` and `vec3`. The check is conservative: `name: string? = NAME` is rejected too, though it would be safe. Done in the first language pass: defaults and constants take struct and enum values (not owning ones), with a loop check.
- **A native's type-error help text overstates the allowed struct fields.** It says "a struct of those", but a struct field may not be an enum. Done in plan 1 of the 3D work (Task 2): the help text now lists what a native can take and return, and says a struct field may be a `list[T]` but not an enum.
- **A field or variant called `native`** reports "native functions can't be methods", which doesn't say what is wrong. (`native` is a keyword, so the name can't be used; the message should say that.)

### Window runtime

- **Negative-size rectangles vanish.** `draw.rect` and `draw.rect_outline` with a negative width or height draw nothing, because raylib culls the winding. Triangles are already normalised; do the same for rectangles. Done in plan 2 of the 3D work: `rect`, `rect_outline` and `rects` with a negative width or height cover the rectangle the size spans back from `pos`.
- **`window.time()` is a `float`**, so its resolution drops as the program runs: under 1 ms for about 2 hours, 4 ms after 10 hours. Fine for frame times; a long-running clock wants an `f64` version. Done in plan 2 of the 3D work: `window.ticks_usec()` is an exact `i64` clock in microseconds since `window.open`; `time()` stays a `float` for frame code.
- **Headless strings aren't escaped.** A string containing `"` or a newline prints a line that is hard to read back.
- **`print` is buffered until the program ends.** `window.frame()` now flushes it, so a frame loop shows its lines live. A window program that prints from a long computation before its first frame still shows nothing until it ends. Flushing when stdout is a terminal would fix that too.
- **raylib keeps the window title pointer after `InitWindow`** (`CORE.Window.title`), but the title's `CString` is dropped after the call. It is benign in raylib 6.0, which reads the title only inside `glfwCreateWindow`, but the `SAFETY` comment doesn't say why, and a raylib upgrade could break it.
- **`headless::QUIET` is a second thread-local** that has to follow `state::STATE`. Both are set in `window.open`; keep them together if either moves.
- **The `real` and `state` modules call each other.** `real::init` needs only the `hidden` flag; passing it would remove the cycle.
- **`png_pixel` and `png_pixels` (test helpers) don't handle non-ASCII paths.** Done in plan 2 of the 3D work: they read the file through Rust (`image_file::load`), like `load_texture`.

### Tests

- **Smoke test.** Its scratch directories (`target/tmp/window_smoke/<program>_<jit or exe>`) aren't unique per run or removed. The hidden-window fallback has no test of its own (it can't be provoked on a machine where the hidden window works).
- **The harness's window check is textual.** `cli/tests/programs.rs` panics for a program that calls `window.open` (or an imported `open`) without `// env: KESTREL_HEADLESS=...`. A call hidden in a string would be a false alarm, and a window opened some other way (a future native) wouldn't be caught.
- **The Godot refusal test** can't tell the refusal of a built-in module from another load error. Check the message.

## Found while building plan 1 of the 3D work (2026-10-10)

From the whole-branch review of plan 1. It found no memory-safety bug, double drop or crash on a valid program.

- **Two shapes still leak an owning value** (and so a GPU object), against the spec's "freed when the block it was made in ends". Both were reproduced, and neither is unsafe. The spec's "GPU objects" section lists them as known gaps.
  - A mixed `if` used as a read-only argument: `show(if c { make(i) } else { t }, ..)`, or an `if` or `match` arm that has its own locals: `width(if c { let q = make(1); make(2) } else { make(3) })`.
  - A fresh owning value in a `while` condition: one is made each time round, and only the last is dropped.
  - The workaround is to store the value in a `let` first. Fix idea: zero-initialise the held temporaries at scope entry, and drop before reassigning in loops.
- **Moves are flow-insensitive.** `if i % 2 == 0 { a.add(c) } else { b.add(c) }` reports "`c` was moved earlier", because move checking ignores that the two branches are exclusive. This was already so before plan 1, and textures and meshes will make it an everyday pattern. Done in the first language pass (2026-10-12): moves follow the program's paths (branches, loops, `break`/`continue`, assignment).
- **`k_export_<name>` and `k_thunk_*` symbols** have the same collision shape against user names that `__kestrel_` names had (those are reserved now). Kestrel items are `k_<name>`, so a user function `export_foo` gets the symbol of the export thunk of `foo`, and `thunk___kestrel_par_0` the symbol of a `par for` body's thunk. Prefixing the generated symbols with `__`, as the init thunk's is, would fix both.
- **Each reload of a file with heap constants leaks the previous constants arena** in the Godot add-on. It happens only at dev time, when a file is reloaded.

## Found while building plan 2 of the 3D work (2026-10-11)

From plan 2's task reviews, its final whole-branch review (Opus), and the Rust comparison of the particles demo (`kestrel/bench/particles/results.md`). None of it is fixed. The spec is `docs/superpowers/specs/2026-10-10-3d-textures-design.md`.

### Drawing cost (from the Rust comparison)

- **`draw.rects`'s upload mostly waits for the graphics driver.** The slowdown the particles demo saw after a 16-thread `par for` is not `par for`'s: a Rust port on the same raylib shows it with no update at all. The best explanation (about 75% sure) is that the NVIDIA OpenGL driver handles each 14.4 MB upload on a thread of its own, so the next frame's upload waits for it, and a faster update only brings the next upload closer to the last. **Sending fewer bytes is the fix:** 4 vertices with shared indices (48 bytes a dot), or a vertex shader that expands one position per dot (8 bytes: 14.4 MB becomes 1.6 MB at 200,000).
- **Pinning the window thread to one CPU** before the graphics context is made halved the Rust port's frame (5.6 to 2.75 ms). It is a hack that depends on this driver, and it was not tried in the window crate.
- **Kestrel's own loops are slower than Rust's: inputs for the stronger backend (sub-project 5).** The particle step takes 1.5 times Rust's time on one thread with the demo's `pow` per particle (4.89 against 3.27 ms at 200,000), and 3.4 times with the `pow` hoisted out (14.8 ns a particle against 4.3). The loop that copies positions into a list takes 2-3 times as long (0.44 ms against 0.15-0.24). Autovectorising and bounds checks are the candidates; neither has been looked at. In the whole frame it is small: with bulk drawing Kestrel is 2-7% behind (5.58 ms against 5.22-5.46), with one `draw.rect` per dot 14-16% behind (28.3-29.3 ms against 24.7).
- **Hidden windows were held at 60 fps** for about 40 minutes that afternoon, whatever the particle count; cause unknown. `measure.py` warns about such runs and `--retry-capped` repeats them. Plan 3 saw it for a whole session (next section).

### `draw.rects`

- **It uploads 72 bytes per rectangle** (6 vertices). See above for the 48-byte and 8-byte versions.
- **Its mesh and buffer never shrink** (up to 72 MiB each after one huge call), and one buffer is rewritten per call.
- **Its chunk loop over 2^20 rectangles is untested.**

### The camera check

- **A camera with NaN or infinite vectors passes `check_camera` and draws nothing.** The spec has no message for it.
- **A `pos` and `target` that differ by a hair pass too.** The check is exact, so a gap of 1e-30 goes through and raylib gets a degenerate view. Godot's `look_at` uses an approximate check.
- **A zero `up` gets "needs an up that doesn't point along the view",** which is true but vague. "needs an up that isn't zero" would be clearer.

### Textures and meshes

- **A texture file over 2 GiB is reported as "isn't a PNG, JPG or BMP image".**
- **Indexed meshes rely on a marker pointer.** The stored mesh parts keep every CPU pointer null, but the copy passed to raylib's `DrawMesh` carries a pointer to a static marker in `indices`, because its GL 3.3 path picks indexed drawing by `indices != NULL` and never reads it (checked in `rmodels.c`). If a raylib update reads `indices` there, the marker needs to become a real buffer.
- **Ids dropped after the window closed stay in the drop queue** (8 bytes each, bounded): `gpu::drain` takes only the ids this thread's registry holds, and there is no process-wide "closed" flag, because tests share the static queue across parallel threads.

### Tidy-ups

- `span` lives in `bulk.rs`, but sprites use it too.
- `float_text` lives in `headless.rs`, but a camera error uses it.
- `upload_part` converts counts with `as c_int` (bounded).
- `Ready::Real(Option<Texture>)` mixes "untextured" with "missing texture".
- `pass.rs` has duplicated test helpers.

### Test gaps

- No end-to-end test of a relative `window.screenshot`.
- A `paths.rs` test relies on no lib test calling `set_program_dir`.
- A real mesh freed mid-run (not at close) isn't exercised.
- The white vertex colour of a mesh without colours relies on raylib's default attribute value.
- The GPU smoke tests' visible-window retry is opt-in (`KESTREL_SMOKE_VISIBLE=1`), so on a driver that doesn't render hidden windows they fail with a message naming the variable.

## Found while building plan 3 of the 3D work (2026-10-11)

From the voxel viewer and its measurements (`kestrel/demos/voxels/results.md`). The viewer met the spec's success line (60 fps at `WIDTH` 8, 463,749 faces) and nothing it found is a bug. Not fixed:

- **Hidden windows were held at 60 fps for the whole session**, whatever the world size, until the work per frame passed 16.67 ms. `measure.py --retry-capped` didn't help. So there is no uncapped frame time for the real viewer at `WIDTH` 8, 16, 20 or 24: the frame cells in `results.md` say "capped", and the room is inferred from the CPU draw time (0.25 ms of 16.67 at `WIDTH` 8) and from growing the world (60 fps holds to `WIDTH` 24, 4.2 million faces; `WIDTH` 32, 7.3 million, is 16.95 ms). The cause is unknown (a guess: the NVIDIA driver's handling of vsync for a window with no output). `WIDTH` 28-30 wasn't run, so the crossing is only known to lie between 24 and 32.
- **The upload costs about 28 ns a vertex** (53-59 ms for the 1.85 million vertices of `WIDTH` 8, about 40% of start-up). By a standalone Rust copy and subtraction: colour conversion is about 6 ns a vertex (21%), per-call overhead about 5%, and the rest (about 73%) is raylib's `UploadMesh` and the driver. Two fixes, neither tried:
  - **A branch-free colour conversion,** `(c.clamp(0, 1) * 255 + 0.5) as u8`, took 3.0 ns a vertex against 6.1 for the `round` version, about 5.5 ms of the 53. It gave the same bytes for the world's 16 colours and for a million random values, and differed on 1 of 768 values chosen at the half-steps, so `to_raylib`'s tests need a look.
  - **Overlapping the upload with the build.** A `par for` that streams its results to the main thread, which uploads each chunk as it arrives, would hide most of the upload inside the build (about 125 ms down to about 75 ms). The language has no such loop.
- **A slow upload was seen once and not explained.** Earlier runs showed 290-320 ms; about 50 quiet runs later it was 46-120 ms, which agrees with plan 2's measure. Loading the CPU on purpose reproduces it (1.4 to 4.6 seconds with all 16 threads busy), so it was probably another job on the machine.
- **The build arena holds every chunk's lists until the upload is done, and the process keeps the memory after it.** At `WIDTH` 8 that is about 184 MiB of lists (measured as the program arena of a copy without the `arena { }`: lists grow by doubling and leave their old buffers behind) and a peak working set of 269 MiB, 875 MiB at `WIDTH` 16. A plain build at `WIDTH` 24 would need about 1.8 GiB (extended from those, not measured). The arena frees the CPU data for the program after the upload (the program arena stays about 64 KiB), but the process's working set at the end of `WIDTH` 8 is still 268 MiB. The cause is the runtime's spare cache, not the arena (next item): a headless exe of the viewer, with no graphics driver, held 172 MiB for the whole run after the build, and 249 MiB at `WIDTH` 16. Building and uploading in slabs, each in an `arena { }` of its own, keeps the peak down (the measured scratch variant peaked at 745 MiB at `WIDTH` 24); the committed viewer doesn't.
- **A big one-off arena leaves up to 256 MiB in the main thread's spare cache for the rest of the run.** `runtime/src/blocks.rs` keeps a freed arena's blocks in a per-thread list, up to `KEEP` = 256 MiB, so that code which makes and frees arenas over and over (every frame, every chunk) reuses warm memory. A `par for` inside an `arena { }` hands the worker threads' arenas to that arena (`runtime/src/par.rs`, `run_job`), so they are freed, and cached, on the thread that frees the `arena { }`: the main thread. The voxel viewer builds one 184 MiB arena at start and so holds about 172 MiB (working set, headless) until it exits; a bigger one-off arena holds about 250 MiB (`WIDTH` 16, headless). The limit is per thread, so by the code a thread that frees big arenas of its own can hold as much. That the memory is the cache is read from the code and fits the measurements; the cache was not instrumented. Fix ideas, neither tried: don't cache blocks above some size, or trim the cache when an arena much larger than usual is freed. This changes the runtime's reuse trade-off, so the user decides.
- **The viewer draws every mesh every frame.** From between 3 and 4 million faces the program waits inside its draw calls (1.2 ms at 3.0 million, 9.6 ms at 4.2 million, 14.2 ms at 7.3 million), while the calls themselves cost 0.4-0.9 microseconds each: the limit is the GPU and its driver (an inference from the waiting, not a GPU timing). The waiting is not proportional to the face count, so there is no cost a million faces to quote. Skipping meshes outside the camera's view, and drawing fewer faces far away, is the first thing to try.
- **Real bigger worlds cost more a face than the stress runs.** The `WIDTH` 8 world drawn 16 times a frame (7.4 million faces) still holds 60 fps, while the real `WIDTH` 32 (7.3 million) just fails. Candidates, untested: the real world is 560 MB of different vertex data against 35 MB read again and again, and more of it is in view. The first seconds of the big worlds are also slow (`WIDTH` 32: 66, 45, 31 ms, then 14-17); the cause is unknown.
- **The viewer's start view could be better.** It starts outside the world's corner at (-24, 100, -24), so the first view is of the outer walls from below the peaks. A higher start looking down onto the top surface would show the terrain better.
- **`measure.py`'s particles rule warns "held to 60 fps" for any median frame above 16.4 ms,** including a slow one such as 41 ms. It was left as it was; viewer runs use a band of 16.4-17.0 ms instead.
- **The viewer's overlay shows 0 fps and 0 ms for the first second,** until the first report interval ends (`fps` and `frame_ms` start at 0). It is harmless and gone after a second. A cheap fix: show `window.fps()` until the first report.
- **The viewer's bottom overlay line follows the window's height, but the particles demo's doesn't.** The window is always resizable (`real.rs` sets `FLAG_WINDOW_RESIZABLE`), so a constant `HEIGHT - 28` (`particles.kes:132`) leaves that line mid-screen in a maximised window. The viewer now uses `window.size().y - 28`; the particles demo was left as it was.
- **The voxel smoke test's exact pixel depends on the GPU's sample positions.** `window_smoke_voxels.rs` checks pixel (32, 30), where the column of peaks is about 7 pixels wide in a 64×64 window drawn with 4x MSAA (`FLAG_MSAA_4X_HINT`). It is stable on the development machine; a GPU or driver that places samples differently could put an edge on that pixel and fail the test with no bug (a guess; it was not tried on another GPU).

## Found while building scalar replacement (2026-10-11)

From `docs/superpowers/specs/2026-10-11-scalar-replacement-design.md` (its "As built" section) and `kestrel/bench/kernels/results.md`. The work met its three targets for the step (within 1.2x of Rust, within 5% of the hand-written version, `pow` within 1.15x). Not fixed:

**Not in the design (open, in the design's words):**
- **Vectorizing.** Rust's last bit of the step (about 7% in the spike: it pairs `x` and `y` in one register). This is the SIMD half of sub-project 3.
- **Bounds checks and list-header reloads.** A plain `for i` loop over `parts[i]` adds about 1 ns per particle (8 checks, with the list's length and data pointer re-read for each), and the copy loop takes about 4 times Rust's time (1.27 ns against 0.31 in the last run, both on a loaded machine; see the next section).
  - Tagging list elements and list headers as separate memory for Cranelift has a trap: a list's header can itself be an element of another list, so a careless split would let Cranelift reuse a stale header.
  - Promoting an inlined `var self` call on a checked indexed place (`parts[i].step()` in a plain `for i`) belongs with this work, because its bounds check would move.
  - **The bounds-check half is done (2026-10-20):** a counted `for i from 0 to xs.len` proves `xs[i]`, so `step_index.kes`'s loop and the draw copy have none left (`--dump bench`: no `trap user2`). The header reload and promoting `parts[i]`'s fields are still open (see the range analysis section).
- **Range analysis for overflow checks** (the 3D world's 2.4x cost with checks on; see "Performance" below). **Built 2026-10-20** (overflow, shift, division, narrowing and bounds checks, and `kestrel check --report`); what it still doesn't prove is in "Found while building range analysis" below; measured 2026-10-20 (the voxel world's checked build is now 1.09x its `--release` build).

**Found by this work:**
- **The draw copy got slower, 0.80 to 1.11 ns an element, and was fixed (Task 6, 2026-10-11).** `copy.kes`, and the same loop `positions[i] = p.pos` in the particles demo: 0.06 ms of its 5.4 ms frame at 200,000. `promote` keeps `p.pos` in two registers, so the copy is a store of a `StructLit` of locals, and the struct-store rule wrote it as one store per field, each with its own bounds check on `positions[i]` and its own reload of the list's length and data pointer. A plain struct stored to memory is now one `Assign` of a `StructLit` (`split.rs`, `store`; `assign` in `codegen/src/func/place.rs` reads the struct's values first, then finds the place once and stores each value), so the loop has one check again and takes the same time as before scalar replacement within noise (`results.md`, "After the one-check struct store"). What this does not do: it covers a struct value built from locals stored to one place. A plain `for i` loop's 8 checks, and a copy loop's reload of the list header for each check, are still the header-reload item above.
- **Where the compiler puts code moves timings by 6-30% on this machine.** Adding unused functions to a program, which only moves the code, shifted these: `bench/heightmap --release` (old compiler / new) went from 49.8 / 55.7 ms as written to 52.4 / 49.3 with two unused functions added, so the order flipped; the new compiler's `bench/2a` particles exe took 148-158 ms or 167-196 ms depending on the layout (and its `--release` exe was slower than the checked one on three of four layouts and faster on one), where the old compiler's took 351-366 ms on all four; `copy.kes` on the old compiler took 0.70-0.77 ns on three layouts and 0.94-1.06 on one. The loops in question are tiny, and the likely cause is the alignment of a hot loop or branch target (not checked at the instruction level). Two consequences: a difference under about 10% between two builds of a small program means nothing until it survives adding an unused function; and it may be worth seeing whether Cranelift can align loop headers (not looked at). All comparisons in `bench/kernels/results.md` were taken old against new, taking turns, for this reason.
- **Costs the design accepted, to revisit if a benchmark shows them:**
  - A call's struct result, and a whole struct passed or returned at a call boundary, still go through memory.
  - Plain structs have at most 16 leaves; a bigger one stays in memory.
  - `promote` loads every leaf it uses at the start of a region and writes back every changed leaf at every exit, including an early `return` that changed nothing. Not measured.
  - A panic in the middle of a promoted region loses the fields not yet written back. Nothing can see it today (a panic ends the process, or the Godot add-on throws the memory away); a future way to recover from a panic and keep the program's memory would have to write back first.
- **`promote` leaves a loop element in memory when its region can return after freeing memory** (the review's segfault; "As built" in the design has the reason). The check is broader than it needs to be: any call with a `var` argument (a copy function, say) or a scratch-arena pop in a loop that returns turns promotion off. A narrower check would look at whether the freed memory could be the element's. And a map-value loop with a `return` never promotes (`MapUnlock` counts as a second route to the memory), which was true before the fix.
- **Review findings left as they were (minor):**
  - `visit_mut` visits a `par for`'s arguments before its count and `visit` visits the count first. `visit_mut` matches the order codegen uses, so nothing is wrong, but the two walkers disagree.
  - `kir/tests/promote.rs` counts `(2, 6)` loads and stores including the dead trailing write-back; it breaks if that append becomes conditional (a comment says so).
  - `region()` makes a local for every leaf but loads only the ones used; the rest are never referenced.
  - `promote_exits.kes`'s `int_loop` has no `break` (its `float_loop` has one).
  - Four older documents under `docs/superpowers/` still name `kestrel_rt_math`, which is gone. They are history.
- **`bench/2a` particles is now faster than Rust -O3** (163-209 ms against 322-329, C at 132). Not looked into; the Rust loop's dependency chain may be longer (LLVM may be using a select where Kestrel branches).
- **`bench/maps` was unreadable on 2026-10-11:** times swung from 305 to 2,557 ms for the same program in the same hour, against 229-242 ms in the quiet hour of the saved baseline. The scalar replacement passes can't be seen in it, but it should be run again on a quiet machine before its numbers are used.
- **The `collatz` JIT cell fell 13%** (318 to 278 ms) while its exe cells moved 2-4%; unexplained, probably noise.

## Found while building the first language pass (2026-10-12)

From `docs/superpowers/specs/2026-10-12-language-pass-1-design.md` (its "As built" section) and the task reviews. None blocks anything. Not fixed:

**Moves (part A):**
- A later "some paths" move of a local overwrites an earlier "every path" record, so a use after an already-reported error may say "may have been moved". Cosmetic.
- After a reported loop error, a `for` loop's exit state is the plain entry state, so a later use isn't flagged again. The program is already rejected.

**Views (part B):**
- Each `let ... else` view still gets an addressed KIR local that nothing uses (like `for` items).
- Exclusivity is whole-variable: `p.x.m(var p.y)` and `f(p.x, var p.y)` are refused though the fields are disjoint. This is an older rule.
- `check_exclusive`'s doc comment doesn't mention the receiver case (`c.poke(var c)`).

**Defaults and constants (part C):**
- A non-owning variant of an enum that has an owning variant (`E2.Plain(1)`) is reported as owning.
- Defaults that allocate nothing but have a heap type (`Kit.Empty`, a struct holding only a string literal) still count as allocating.
- Missing regression tests: `builds_owning`'s negatives (`[]`, `none`, a fieldless variant); cycles through an enum payload, an optional or a map.

**File names (part E):**
- No automated test for a native error from a built exe, the arena messages' file, the map-changed panic's file, an array literal's allocation, `LineIndex::file`, or the base name of `<builtin>/draw.kes`. All were run by hand and are correct.
- `of_file` would print " of " for an empty name.
- A synthesized line-0 expression would name file 0. Unreachable today.
- The record layout assumes a 64-bit little-endian target.
- `kestrel_rt_call_file` could point into freed JIT memory after a hot reload if window natives ever reach the Godot add-on. They can't today.

**From the final review's fix round (2026-10-17):**
- **Nested range loops compile slowly.** `for i from 0 to 1` nested 100 deep takes 0.6 s to `run` in a debug build, 200 deep 3.3 s, 400 deep 21 s, and 1,000 deep over 300 s; `kestrel check` takes 0.27 s on the 1,000 case, so the time is in lowering or codegen. Nested `while` loops are instant. Older than this pass; only generated code nests this deep.
- **The depth limit is checked on big stacks only.** 1,000 levels run on the CLI's 256 MiB thread in a debug build. In the Godot add-on (Godot's own, smaller stack) only parentheses, sums and a 1,000-term function were tried by hand, not nested blocks or types, and no test runs a 1,000-level program on a default 2 MiB debug test thread.
- After a too-deep error the parser drops its own later errors, but lexer errors later in the file (an unterminated string) are still reported.
- A `let ... else` view over a loop item of a `let` list reports "it's inside `hs`" (the list), not the item it follows.
- `let files =files` is missing a space (`driver/src/lib.rs:108`).

**File sizes** (the 400-line guide): `check/src/body/mod.rs` is 397 lines, `check/src/body/call.rs` 391, `check/src/collect.rs` 389, `kir/src/ir.rs` 405 and `runtime/src/map.rs` 494.

**Known limits, by design:**
- Views over a `var` root still copy. They would need guards until the end of the block and on every write.
- A "may have been moved" message can't point at the move, because the checker has no line numbers and diagnostics have no secondary labels.
- `var x: T` with no value (starting at zero, like GDScript) is a syntax question put to the user on 2026-10-12.

## Found while building list and string basics (2026-10-17)

From `docs/superpowers/specs/2026-10-17-lists-strings-design.md` (its "As built" section) and the task reports. None blocks anything. Not fixed:

**Follow-ups to build:**
- **String views as an optimisation.** `s[i]`, `slice`, `trim`, `split` pieces and the characters of `for c in s` are copies in the current arena (spec ruling 2). Views into `s` would avoid the allocation (`for c in s` allocates one small string per character), but need a stricter region rule: the design review found six ways a view outlives its bytes (returns from owning roots, `arena { }` blocks, loops whose string the body frees, later `var` arguments).
- **`remove` inside a `par for` over a chunk's own list** would be safe but is refused (any `par for` around the statement is).
- **`for x in xs { f(var xs) }` where `f` shrinks `xs`** is accepted by the checker. The runtime index panic is the safety net.

**Gaps and rough edges:**
- **The checker decides on a native's scratch arena before it knows the arguments' types,** so it treats any argument shaped like a `+`, a method call or an index as built in the scratch arena; lowering pushes one only when the argument really builds a string (see `wants_scratch`). A list read like `draw.text(names[if c { last = tag(n); 0 } else { 0 }], ...)` is then refused, though it would be safe. Only a block inside an argument can show the difference.
- A `par for` over a map gets the `par for` message, not the map one.
- A `u64` index above `i64::MAX` shows as a negative index in the panic text. `str_char_len` wraps a negative position (only generated code calls it, with positions of 0 or more).
- `to_float("1e-999")` (an `f32` underflow) gives 0 rather than `none`: "doesn't fit" means overflow.
- `text()` and `str_case` use lossy UTF-8 conversion, while `str_trim` returns bytes unchanged (unreachable for valid strings).
- `find` and `split` search in O(n*m).
- `list_insert` zeroes the slot it opens and codegen then overwrites it. Harmless.
- The program-test harness reportedly hung on a test program that fails to compile (the compiler exits 1; the harness wasn't investigated).
- A `.pop()` on a read-only parameter or `self` gets two errors: "isn't `var`" and "it may point into the caller's arena". The first is the useful one.
- A kept `pop` of an owning item works only on a local list; a helper like `take(var self) -> T?` that pops owning items is refused (return the index instead). Lifting this needs a region for the popped value tied to the caller's owner.
- `kestrel/godot/build.ps1` fails under Windows PowerShell 5.1, because cargo writes its progress to stderr. Run its two steps by hand meanwhile.
- Error messages for a clash inside the entry file now come from `link`, so that file's other checker errors aren't shown until the clash is fixed.

**File sizes** (the 400-line guide): `kir/src/ir.rs` is 415 lines and `check/src/tables.rs` 401 (`check/src/body/mod.rs` went back to 389 in the `remove b` task).

## Found while building closures (2026-10-18)

From `docs/superpowers/specs/2026-10-18-closures-design.md` (its "As built" section) and the task reviews. None blocks anything. Not fixed:

**Closed since:** the full suite (835), both clippies and the final whole-round review (Opus) ran, its fix wave landed (8650f44..29f802a), and the kernels were measured (3aea21a, `kestrel/bench/kernels/results.md`).

**From the measurements (ns per item, `--release`, one thread):**
- `map` matches the hand loop (1.39 vs 1.35); both are 1.8x Rust (0.77), which vectorises.
- `filter` is 1.23x the hand loop (4.82 vs 3.91; Rust 2.67): the IR shows the closure inlined, but the loop reloads the list header from the stack twice per item, loads the kept item twice, and keeps a dead load of the environment word. The bounds-check and list-header item (Performance) is the likely fix.
- `sort_by` is 6.1x Rust's `sort_by` (139.5 vs 22.8) and 1.43x a hand-written Kestrel merge sort: the key runs on both sides of every comparison through the runtime callback. Caching the keys (number keys first) is the fix.

**Parked by the final review (fix first next round):** inside `for k, v in hs[0].m`, a `var self` method on `hs[0]` that replaces `self.m` inside `in self.mem { }` runs with no error and gives mixed key/value pairs (no freed memory is read; the fix wave's "the path must contain an index" narrowing in `map_loop_inside`, `check/src/body/lists.rs`, lets it through). Fix: refuse `var` places and `var self` receivers at or above the looped map.

**Follow-ups to build:**
- **Devirtualising.** After inlining, an indirect call of a value known to be a particular function is still `CallIndirect`: a named function passed through a function-typed parameter, or a closure stored in a local and called once. Turning it into a direct call (and inlining that) is the next step. Literals passed straight to the list methods already run as direct calls into inlined code.
- **Methods, built-in functions and natives as values** (`e.take_damage`, `min`, `draw.rect`). They are refused with a "wrap it in a closure" message.
- **Optional function types** (`((int) -> bool)?`). They need a grouping syntax, since `(int)?` reads as a function taking an optional.
- **`reduce`, `reverse`, `min_by`, `max_by`, `find_by` and a comparator sort** (`sort_custom`). Left out on purpose; add them when asked.
- **Calling a function field of a `match` binding** (`r.go(5)`), `m[k](x)` and `xs.pop()(x)` (optionals) are not supported.
- **`promote` doesn't keep a capture in a register.** Captures are `RefParam`s that aren't parameters, so `promote` never sees them. This may matter for the closure bodies inlined into `filter` and `map` loops.
- **Capture-copy chains.** A `compose(f, g)` chain N deep copies N levels on the native stack, and each `h = compose(h, inc)` copies the whole chain (O(N^2)). Accepted by ruling 7b; a long chain could overflow the stack.
- **A codegen stack slot is per `MakeFn` site.** The same site evaluated again while its previous stack environment is live would share the slot. Calls nest strictly and the value never outlives its call, so nothing does this today; a pass that moves a stack environment out of its call would.
- **The exclusivity rule relies on** no mutable globals, read-only captures, and a callee changing only what its `var` arguments and `var self` give it. A feature that breaks one (mutable statics, mutable captures, methods that change their receiver without `var self`) must update `changes_in` and `call_changes`. The unchecked indexing in the list-method loops relies on the same facts.
- **Poison mode catches reads, not writes.** A write into a freed block isn't seen. A check that the `0xDB` bytes are still intact at some later point (when the block is dropped for good, or at exit) would catch it. The `codegen` tests that call thunks in-process run without poisoning; a public `set_poison` would cover them.

**Gaps and rough edges:**
- **Error lines in closure code.** A native's error line was wrong when a list-method loop (which calls user code) sat in its arguments; that is fixed (`runs_user_code`). An indirect call still counts as possibly setting a line, even of a `pure` type, which only costs a spill of the other arguments. The `sort_by` helpers aren't treated as running user code, which is right because the key is `pure`. No program test checks a native's error line through a closure; the KIR tests cover the lowering.
- Inside `in w.mem { (if c { w.cb } else { h })(var w) }` the copy of the callee lands in `w.mem`, which the call replaces. The run aborts with "an arena was freed while allocating into it" (safe, but a compile error would be kinder).
- A stack environment is checked by a debug-only walk (`check_stack_envs`); a release build and a pass that runs after it don't. A structural check in place of the counting one was left out.
- Stack-environment tests rely on stack slot reuse showing up as garbage; poisoning doesn't cover stack frames.
- The owning-capture message says "an Arena" without backticks; the other messages have them.
- `check_capture` takes two bool parameters.
- A must-be-pure closure that calls `zs.any(noisy)` with a named non-pure function says "which isn't a pure function type" (the `Impure::Value` wording, `check/src/body/list_fns.rs`), where "isn't `pure`" would be right. A named non-pure function in a `pure` function says "declare the parameter as `pure (int) -> bool`", which suits a parameter better.
- A constant of function type used as a field default counts as allocating (`default_allocates`). A forward reference to a function-typed constant called by name gives the old "is a constant" message.
- Several test assertions are weak or missing: `check/tests/fn_values.rs` (a few `contains` checks), no tests for a side-effecting `var` index through a value, a runtime `remove` on `list[(int)]`, or a function value returning a function value through `sret`. No committed end-to-end test that a real program's freed blocks get poisoned (checked by hand: 1,008 blocks in `arenas.kes`).
- `local_place` duplicates `place()`'s `Ident` arm (`kir/src/lower/fnvalue.rs`, `lower/call.rs`).
- The `start_*` unit tests in `runtime/src/start.rs` reset the global poison flag from the environment (use `start_with(entry, false)`), and `start_with` is `pub` only for tests (`#[doc(hidden)]`).
- A `par for` job failure (`par.rs`) leaks the thread arenas without poisoning them.
- `sort`'s compare function returns an `i8` that codegen reads from `al`; Cranelift leaves the upper bits unspecified and the runtime reads only the byte. JIT and exe pass. A key that is a temporary copy of a function value read from the list's own item is refused even though the runtime copies the 16 bytes before sorting.
- A `sort_by` key that is a native or any indirect call keeps its scratch arena (a push and pop per comparison) because `builds_memory` can't see inside it.
- Closure parameters can't shadow an outer local, so `(e) { ... }` inside `for e in ...` is refused; someone coming from GDScript may hit it often.
- Rust-side formatting: `cargo fmt --check` wasn't clean before this round, so it wasn't run.

**File sizes** (the 400-line guide): the new files are under 340 lines (`check/src/tables.rs` is 339, `kir/src/lower/sort.rs` 246, `check/src/body/list_fns.rs` 225); `check/src/body/lists.rs` is 295.

## Found while building interfaces (2026-10-19)

From `docs/superpowers/specs/2026-10-19-interfaces-design.md` (its "As built" section), the task reviews and the measurement. None blocks anything. Not fixed:

**Owed from the build:** one full `cargo test --workspace`, the whole `programs` target and the Godot clippy, none run since Task 3 (light mode; Task 4 and the final review's fixes ran focused targets and workspace clippy).

**Fixed by the final review's fix wave** (and so removed from the list below): an `if`/`match` of places passed as an argument or receiver read freed memory when a later argument freed it (the checker now refuses it, and `print`'s arguments, operators and interpolation get the same rule); the per-call copy of a non-place interface receiver (removed); `q.add(take(var q))` and `self.ids.add(self.alloc())` refused (the built-in check is narrowed); interface constants boxed again at each use (now hoisted); the missing promote, enum-copy and constant-copy tests; the stale `table_for` pointer and the misnamed KIR test. The re-review then found that removing the receiver copy let a map `get` receiver read freed memory (`ws[0].m.get("a", hd).plus(drop_first(var ws))`); a map `get` of a heap value now counts as reading the map's place, which also closes the argument and `print` forms.

**From the measurement (ns per item, `--release`, one thread; `kestrel/bench/kernels/results.md`, "Interface calls"):**
- A call through `list[Damageable]` (two types in a random order) takes 5.2 ns for `hp()` and `step(var self)`, against 0.5-0.7 ns for the inlined direct call over `list[Goblin]` (7-11x), and 14-19% more than Rust's `Vec<Box<dyn Trait>>` (4.4 and 4.5-4.6 ns). With the types in two blocks it takes 2.2 ns, so about 3 ns of the 5.2 is the mispredicted jump and not the call.
- The likely extra cost against Rust: every method is a full function with a frame (`push rbp` ... `pop rbp`) around one load, and the loop reloads the list's data pointer from its header each item. A frameless leaf function and the header kept in a register would close part of it. Not measured separately.
- Against Rust's direct loop over a `Vec<Goblin>` the gap is 7-10x (0.07 ns, vectorised); that is the general "one item at a time" gap seen in `map`, not an interface cost.
- Devirtualising (a `CallVirtual` whose `MakeIface` is visible after inlining becoming a direct call) is not done. It would remove the call only where the type is visible, not in a mixed list.

**Follow-ups to build:**
- **`array[[Damageable]](n)` is over-refused.** `zero_fill_breaks` refuses an array of arrays holding an interface; an empty inner array holds none. Allow it when the innermost items are optional or lists, or refuse only when the array's own items are (or hold) interface values.
- **`is` tests** (narrowing an interface value to its struct) wait, as the user said. Until then an interface value can't get its struct back, and an interface value doesn't convert to another interface.
- **`iface_memory.rs` measures only the inlined path.** Its `show` is small enough to be inlined into `main`; a callee too big to inline would also exercise a lent argument passed to a real call.
- **Over-refusals the checker accepts for now:** `use2(if c { g1 } else { g2 }, change(var g1))` with a concrete `g1` (the `if`'s value is boxed as a copy in its turn, so it is safe), `print(stack, stack.pop())` (the rule refuses any later change to a printed value's variable), and a map `get` of a heap value from a borrowed map (`m.get("a", [n])` with `var m` a parameter, then an argument that changes `m`), whose value KIR copies. Telling these apart needs the checker to know which values KIR copies in their turn.
- **"function values can't be compared"** doesn't say what to do, unlike the interface message.
- **The promote rule for a borrowed place** (a `MakeIface` of the root anywhere in the region makes it non-exclusive) shows in no program's output; only a KIR test holds it.
- **A call's receiver is read before its arguments** in `call_virtual`: if an argument replaces the receiver place, the old box is still called. Boxes are freed only with their arena, so this reads freed memory only if an argument frees that arena. `check_changed_later` refuses an argument that changes the receiver's variable, and since the final fix also when the receiver (or an argument) is an `if` or `match` of places; before that, this line wrongly implied arguments were covered.
- **`Stmt::Trap` and `Stmt::Fail` carry no line.** The abstract-method trap and the owning copy error print without one ("on line 0" for the trap).
- **A `pub` function with an interface in its signature** compiles but isn't callable from Godot.
- **A stored interface value costs an allocation,** like a string, so `self.foes.add(Goblin())` in a method needs `in self.mem { }`, which `list[Goblin]` doesn't.

## Found while building range analysis (2026-10-20)

From `docs/superpowers/specs/2026-10-19-range-analysis-design.md` (its "As built"), the task reviews and the ledger. This closes "Range analysis for overflow checks" and the bounds-check half of "Bounds checks and list-header reloads" above. None of the rest blocks anything. Not fixed:

**Measured, and owed from the build:**
- **Compile time missed its target.** The analysis adds 30% (+42 ms) to a checked build of the voxel world and 43% (+48 ms) to a `--release` build, and 32% and 44% to the voxel viewer; the particles demo is +2%. The target was under 10%. About 45 ms of it is the analysis on the one function that holds the inlined noise code. Not profiled: look at the fixed-point passes over the nested loops, the facts between up to 64 terms (`MAX_TERMS`, the fact-work budget) and the statement budget. See `kestrel/bench/voxels3d/results.md`, "Range analysis".
  - **The cheapest win didn't help (2026-10-20, night).** Builds no longer make report text for the checks left (`mark_proven`); the analysis of `demos/voxels/voxels.kes` went from 77 to 75 ms checked and 50 to 51 ms `--release` (medians of 9, timed inside the compiler), and `kestrel build` didn't move (249 against 251.5 ms, 224 against 224). The text was never the cost.
  - **Where the time goes** (the same timing, per function): the `par for` body with the inlined noise code is 46 of the 75 ms: 411 statements, 25 terms, 26,562 statement visits, 65 per statement; `world.solids` and `world.chunk_mesh` are about 13 ms each (about 400 statements, 20 terms, about 8,700 visits, 22 per statement). About 1.8 microseconds a visit.
  - **The final review's analysis, by reading (no quadratic-in-size bug; the costs multiply):** (a) a `State` clone carries a dense `n x n` `i128` matrix whenever any fact exists (65 nodes: about 67 KB; 25 nodes: 10 KB), cloned at every `Seq`, value `if`, condition (`refine.rs`) and `for` round, and joined and widened with an O(n^3) `close` (`facts.rs`); (b) each level of loop nesting costs 2-3 passes, so the body of a 3-deep nest runs about 10-27 times (the 65 visits a statement above); (c) `writes::of_operands`, `of_stmt`, `touched_nodes` and `rebase`'s `of_stmts` walk each statement's whole subtree again on every visit and every round, and an inlined `Seq` is walked once per level around it.
  - **Next, in order:** profile the voxel `par for` body; cache each statement's writes (and touched nodes) by its address for the whole function, since a statement's writes never change between visits; make `Facts` copy-on-write (`Rc`), or `i64` and sparse, so a clone of a state with few facts is cheap; then see whether an inner loop can take fewer passes when the loop around it repeats (not looked into).
- **Measured 2026-10-20** (`kestrel/bench/kernels/results.md` and `kestrel/bench/voxels3d/results.md`, "Range analysis"): the voxel world's checked build 1,415 ms to 643 (1.09x `--release`, target at most 1.2x: met; `--release` unchanged); `step_index.kes` 11.1 to 8.3 ns (26% faster, still 1.43x `step.kes`); `copy.kes` 0.95-1.20 to 0.91-0.93 ns (0-24% depending on code layout; still 3.4x Rust); no other kernel, `bench/2a` workload or `--release` heightmap slower beyond layout noise; the checked heightmap 82 to 61 ms.
- **One full `cargo test --workspace --no-fail-fast`** (including the exe pass without the proof check), workspace clippy and the Godot clippy as a whole. Every task ran focused targets only (light mode). The proof check over every program in three passes was run at the end of Tasks 2 and 3, clean.

**The analysis does not prove (each is its own change):**
- **Hoisting loop-invariant checks.** A check that doesn't depend on its loop's variable (the chunk offsets `cx * 32 + x - 1`, `seed + 1` in the voxel world) stays in the loop. Moving it out, with its value, is the next step for the voxel target, and merging several checks into one belongs with it.
- **Facts across calls.** A function's result range or the length of a list it returns is unknown to its caller (`mesh` can't know `solids` returns 39,304 items, so `solid[i + 1]` keeps its check). Round B's `@assume` covers some; a summary pass would cover more.
- **Products in facts.** `xs[y * w + x]` from `y < h`, `x < w` and `xs.len == w * h` keeps its check until `@assume`. Sums of two variables are not facts either.
- **Integer fields in memory as terms.** A field read from memory (a non-promoted struct, an integer list's element, a map value) has its type's range; guards on it prove nothing. A field `promote` keeps in a register is a local, so `var self` methods are fine.
- **More than 64 interesting values in a function** lose some proofs (`MAX_TERMS`; and `FACT_WORK` past which no new facts between terms are made); a function that exceeds the budget (200,000 visits or 100 per statement) or 256 nesting levels gives up and keeps every check (two of the 183 test programs do).
- **The list-header reload.** With its bounds check gone, `xs[i]` no longer reads the length, but it still re-reads the data pointer after a store, because Cranelift can't prove the store didn't change it. Promoting the fields of `parts[i]` into registers (scalar replacement of an indexed place) also waits. This is the rest of "Bounds checks and list-header reloads" and the reason `copy.kes` will stay behind Rust.
- **Checks inside the runtime's own functions** (`remove_at`, `insert`, a string's `s[i]`) and the map-changed check of a loop over a map: the report lists them as ways to fail; they stay.
- **Precision left from Task 3:** a map value's list in a loop that writes the map (lowering re-indexes it each time); facts through an inlined helper's parameter copy, kept only when `rebase` finds an equal outside term; a length set by `SetLen` to an expression over a written term is forgotten; after 8 rounds a loop's top keeps only ranges.

**For Round B (`@prove` and `@assume`): closed 2026-10-21** (built; see "Found while building `@prove` and `@assume`" below and the spec's "As built (Round B)"):
- **Block end:** decided and built: an assumption ends at its block's `}` (the user's pick, 2026-10-20); the code after the block is analysed as if it weren't there.
- **What an assumption removes:** found by running its block twice in a deciding pass (with it, and as if no assumption in the block were written) and comparing; `refine` is used now.
- **Loops are named by the line of their header in the report:** still true for the report. Contracts don't need a loop id: each annotated loop has its own site and `Prove` block, found by id, so two loops on one line are told apart.
- **A function the analysis gave up on, or code the report's walker cut, fails every contract:** built (a cut counts as a deep give-up; the report still doesn't print `What::Cut`).
- **Report data the contracts read:** built. Running out of memory and the item limit on growth don't count for `no_panic` (`no_alloc` rules growth out), and `m.init` is charged to `main`'s contracts ("it builds the program's constants first, which ..."). The report's own totals still double-count inlined bodies, and contracts never read them.

**Deferred review notes (minor):**
- *Report:* totals count an inlined body in the callee's own entry and in each caller that inlined it (not distinct counts); the runtime's out-of-memory failure is not listed; `m.init`'s allocations aren't charged to `main`; one element-copy line ("makes a copy of the string", the elements of `tags`) is unnamed; a function that makes a closure counts the closure's checks even if the function value never reaches a call; a chain takes the nearest reason (a function's own before a call that leads to one), not strictly the first event in the code; statements with no line of their own (a `Dup`, a drop, `FreeArena`, `SetLen`) take the latest line seen in the walk; a function the analysis gave up on lists its checks with no loop information, in walk order; `Stmt::Trap` and `Stmt::Fail` still carry no line.
- *Analysis:* when a function gives up, the text for every site is built, and `show::write_expr` has a length limit but no depth limit (it recurses to the expression's depth; the other passes do too, but a small-stack host such as Godot is the one that would notice); `debug_assert!(false)` on a site that was never decided panics a debug compiler (a release compiler keeps the check; intentional, it catches gaps in the walk); the budget counts statement visits only, so one huge expression in nested loops costs rounds times its size beyond the budget (a compile-time cost; measure it with the rest).
- *Tests and harness:* the release pass's 30-second timeout is printed and listed in failure messages but doesn't fail the test (consider failing on a timeout in the proof run); `KESTREL_TEST_ONLY=` (set but empty) now means all programs, and the division proof-failure path is now tested through codegen to a running program (`codegen/tests/proven.rs`), as overflow was and bounds is by the `range_keep` programs (both done 2026-10-20); `split` over the item limit is untested (it needs a 2 GB string), and the Godot bridge's `write_arg` is tested only through `too_long`; the proof check's exe link of `kestrel_rt_proof_failed` is exercised by the verify run but by no focused test; forgetting at a `Bind` and refreshing a loop's hidden end are kept as defence: nothing observable fails without them.
- *From the final review (2026-10-20):* a list statement's length change relies on codegen evaluating the operands before it finds the place (documented at `change_len` and the codegen sites, pinned by `range_order_list_place.kes`); the stricter alternative is to forget, not move, a length whose index locals the operands wrote. Two holes the review and its fix wave found are fixed and listed in the spec's "As built" (a whole loop element assigned; a lent interface moved into a local by the inliner). The checker refuses a whole assignment to a `var` list parameter and to a matched value inside its `match`, so those forms of the first hole can't be written today; the fix covers them anyway.

## Found while building `@prove` and `@assume` (2026-10-21)

From `docs/superpowers/specs/2026-10-18-at-annotations-design.md` (its "As built (Round B)"), the task reviews and the ledger (`.superpowers/sdd/2026-10-20-round-b-annotations/`, git-ignored). This closes the "For Round B" items of "Found while building range analysis" above. None of the rest blocks anything. Not fixed:

**Owed from the build:**
- One full `cargo test --workspace --no-fail-fast` (including the exe pass without the proof check), workspace clippy, and clippy in `kestrel/godot` and `kestrel/bench/kernels`: every task ran focused targets in light mode (Task 1b ran 175 of the 212 programs, Task 2 the whole programs target in its proof-check passes); no window, GPU or demo tests (`window_smoke*`, `voxels_demo`, `demos`, `window_print_flush`) ran. Run when the user says the machine is free.
- **Done (2026-10-21): the final whole-round Opus review and its fix wave** (the spec's "As built", "The final review and its fix wave"): the plain run forgets its conditions' writes, a returned local keeps its length past a `@prove` or an `@assume`, the vacuous test reworded, the `Arena`-after-`@assume` program added.
- **`print` isn't counted by `no_alloc` or `no_panic`** (a ruling; spec, syntax spec and README say so). **Follow-up: make print not allocate in steady state:** a fixed thread-local buffer instead of the heap `BufWriter` the runtime makes on a thread's first print (`runtime/src/print.rs`), and `take_output` copying out and keeping its capacity (`clone` + `clear`, not `mem::take`), since the Godot add-on takes output after every call. Also under the ruling: printing or copying a value of a recursive type recurses in generated code no contract sees (no call edges for the generated print and copy functions or `MethodTable.print`/`.copy`), so a deep value can overflow the stack in a `no_panic` function.

**Compile time and precision (measured 2026-10-21, `kestrel/bench/voxels3d/results.md`, "`@prove` and `@assume`"):**
- **The build with an assumption is +65-69% on the voxel world** (296 ms against 175 checked, 255 against 155 `--release`); `@prove` alone is +4.6% and +2.6% (target met), and `kestrel check` with the assumption is 2.84 and 3.76 times the contracts-only time (target 2.2: missed). The assumption wraps all of `solids`, which holds the inlined noise code, so its block runs twice in a deciding pass over the hottest function. Not profiled; accepted for this round (the spec's rulings: about four times for `check` is what analysing both builds, each running the block twice, implies). Candidates: pick an assumption's terms last (above); the plain run could skip the checks it doesn't need to record (it exists for what leaves the block); a real run that finds nothing to remove (no site left in its block that the assumption's terms touch) could be skipped; the facts matrix grows with the terms an assumption adds (Round A's list: make `Facts` copy-on-write or sparse); `kestrel check` analyses both builds for the warning, and could analyse the second only when the first found no removal.
- **Fixed in the fix wave: an assumption that wrapped the end of a function lost what is known about the list the function returns** (a passing `@prove` did too). Repro, now `kir/tests/range_returned.rs` (before the fix, deleting the `@assume` line made `s[i]`'s bounds check go away):
  ```
  pure make(n: int) -> [u8] {
      @assume(n >= 0)
      var out = array[u8](100)
      out[n & 63] = 1
      out
  }
  pure use_it(n: int) -> int {
      let s = make(n)
      var t = 0
      for i from 0 to 100 { t += int(s[i]) }
      t
  }
  main() { print(use_it(3)) }
  ```
  The tail went through a temporary inside the assumption's body, and Round A gives a list a length term only when the function indexes it or reads its `.len`, so the length didn't follow. A read of a whole local now gets no temporary (lowering's `as_written`, the inliner's `marked`). **Still open, follow-up:** let a list local assigned from another list local get a length term (follow copies in `terms.rs` `pick`); that would also cover a temporary with a cleanup and Round A's `let tmp = out; tmp`.
- **The assumption on the voxel world still brings back 24 overflow checks** (`base + 1` to `base + 3` in the six inlined `Mesh.quad` copies of `mesh`; checked builds only), from Round A's term cap: the pre-scan takes terms in order of first appearance and stops at `MAX_TERMS` = 64 (`range/terms.rs`); an assumption's names become terms early (with their copies), and with `solids`, `density`, `octave`, `lattice` and `hash` inlined ahead of the `quad` copies, `base` and `m.vertices.len` fall past 64. An `@assume(seed >= 0)` at the top of `solids`' `y` loop body, not wrapping the tail, brings back the 24 alone. Fix: pick an assumption's terms after the main pre-scan, or rank terms by use count (which would also cut the fact work below).
- **An annotated loop adds one node to the inliner's size count,** so it can flip an `inline` contract at `MAX_NODES` (256).
- `--report` builds 8 chains and a recursion pass though it reads 2, and `implementations(m)` is computed twice (`summary.rs`); contracts use their own. Cheap to trim.

**Not built (as the addendum says, or the design left out):**
- **Contracts on function types** (`@prove(no_alloc) (int) -> int`). A call through a function value breaks all four contracts until they exist; inline it or use an interface method.
- **Natives marked safe.** A native breaks `no_alloc` and `no_panic`, so no contracted code can call `draw.*` or `window.*`; marking a native "doesn't allocate or fail" would need a syntax or a table.
- **`@prove` on a native in a built-in module** is a parse error ("`@prove` can't go on a native function").
- **Warnings in `kestrel run` and the Godot add-on.** Only `kestrel check` shows the "removes no check" warning (the add-on has `godot_warn!` wired but asks for none); contract errors do stop both.
- **Facts across calls.** An assumption at a call site doesn't reach the callee, and a callee's facts don't reach the caller, so a function-level `@prove(no_panic)` and an `@assume` in it can't both hold in a checked build, and moving the assumption to the caller doesn't help.
- **Indexes that are a constant or a local in an assumption** (`grid[y].len`, `parts[i].items.len`). The restricted form (names, fields, numbers, `.len`, `+ - *`) can't state them; allowing an index that is a constant or a local, evaluated only when the assumption is checked, would recover them. A condition that could divide, index or narrow would also let a condition teach the plain run something, which is why the hole in the must-fix above is latent today.
- **Products as terms of a fact:** `@assume(xs.len == w * h)` is accepted, but Round A's "products in facts" gap is still there, so `xs[y * w + x]` keeps its check; the voxel world's `out[i]` and `i + 1` (14 checks a sample, per Round A) are not helped.

**Syntax picks the user hasn't ruled on** (the addendum's "(1)" and "(2)", also in the syntax spec; each built in one place, so a change is cheap):
1. The annotated item may follow `@prove(...)` on the same line or on the next. At most one line break: a blank line or a comment line between is an error (a comment line counting as the break was the implementer's reading of "at most one line break"; say if comments should be allowed). One constant: `MAX_LINE_BREAKS` in `kestrel/syntax/src/parser/annot.rs`.
2. `@assume` conditions allow only names, fields of them, integer literals, `.len`, `+ - *`, unary `-` and parentheses (`assume_side` in `kestrel/check/src/body/assume.rs`). A disallowed side such as `a / 2 < b` gets the generic form error and not a hint about sides (a minor).

**Deferred minor findings:**
- *Parsing and checking:* `annot.rs` has an `at` closure written twice, and its helpers return empty without a source (add a `debug_assert`); `prove_run`'s `in_body` flag mixes the item and body cases; untested: a comment line between an annotation and its target, `@assume(a) foo()` on one line, a blank line before an interface method's annotation.
- *Lowering:* an `if` or `match` arm's heap value with an assumption in it is always copied (`owned(v, false)`) even when there is no cleanup (the brief required it; a move would do when the scope has no cleanup); the inlined result is read with `Load`, which costs an extra copy for an aggregate, only in annotated code (a read of a whole local now needs no temporary at all); no test for a function-body tail with cleanup (`lower/assume.rs`) beyond `assume_owning_body.kes`.
- *Analysis and contracts:* an outer assumption goes `Check` whenever any bounds site depends on any inner assumption (sound, conservative); `KESTREL_CHECK_PROOFS=1` can't catch a wrong `Trust` (it shows as "assumption is false"); a debug-only check that every loop target in `Program::proves` produced a site is missing; copy numbering is spread over `prove_copies`, `Regions::step` and the lookups (a comment linking them), and the address-keyed copies map would be fragile if bodies were cloned mid-analysis; the mutation table's rows for a check's own copy and for loop headers are imprecise; no test for a function site that is lost (`region.rs`, "lost track" for a function); the help line for a promised interface call is only given for a direct call, and chains through calls get the default help.
- *Over-reporting (conservative, not unsound):* a map key set counts for `no_panic` even on a local map that is never iterated; dead copies are still checked (a closure's code left behind after its literal was inlined); a given-up callee names its stop before its first check when both are equally near; the chain through an inlined copy can show the inliner's real call lines, which can surprise ("which can call itself (line 17)" for mutual recursion after `b` was inlined into `a`).
- *Wording:* "comparison for `sort`" in a `sort_by` recursion message; an ambiguous "it" in another function's give-up (`words.rs`).
- *Runtime:* `panic.rs` strips `" of "` from `of_file`'s output for the assumption message; use a bare-file-name helper.
- *Tests:* a false trusted assumption's divide fault under `--release` isn't exercised end to end (documented in the syntax spec; the proof check reports it as the user's assumption).

## Found by the tuned voxel copy (2026-10-20)

`kestrel/bench/voxels3d/voxels3d_tuned.kes` makes the same world as `voxels3d.kes` 24-29% faster (level with a straight Rust port) by hand; each line is a hand change the compiler should make itself. Measurements and details: `kestrel/bench/voxels3d/results.md`, "Tuned copy".
- **No loop-invariant code motion (the big one).** Cranelift's e-graph places pure values at their use and never hoists them out of Kestrel's loops, so even a `let k = i64(seed) * C` written before a `while` loop is recomputed in it (five-line repro in results.md). Root cause not found (opt level is `speed`, the e-graph runs); dump the CLIF and check `elaborate_licm_hoist`. Hand workarounds: a row's values in a small array (-12% `--release`, -14% checked) and loop-carried no-op `p |= 0` variables (-6%, -5%).
- **Reassociate xor/add chains** so the loop-invariant operands are combined outside the loop (`x * A ^ y * B ^ z * C ^ s * D`); measured with the array workaround above.
- **Demanded bits:** drop a mask whose cleared bits nothing reads, and move a mask through xor onto operands that can be hoisted (-7% `--release`, -4% checked).
- **Ranges for array elements** (or `@assume`): a value read from an array has its type's whole range, so a bounded value stored and read back keeps the overflow checks it feeds (24 per sample here, +13% checked) unless it is masked again at the read.
- **Bitwise rules for signed operands:** xor, or and and of values with `|a|`, `|b|` below 2^k should be below 2^k; today only non-negative operands are bounded (an `int` x table cost +30% checked).
- **Float division by a power-of-two constant** should become a multiply (exact); not measured alone.
- **Facts across calls, again:** with `solids` too big to inline (822 nodes), `mesh` keeps six bounds checks per block, removed by hand with a never-taken `if solid.len != ... { return m }` (no measurable effect here).
- **Constants can't use conversions:** `let INV = f64(1.0) / f64(65535.0)` is refused; `let INV: f64 = 1.0 / 65535.0` works.
- **SIMD:** the tuned sampling loop is 465 instructions per sample, about 420 of them the 24 independent hash chains and 21 mixes, near the core's dispatch width; going further needs `vec4`/`ivec4` or SLP vectorisation.

## Maps (built 2026-10-09)

- **Not yet:**
  - values that own an arena;
  - struct, enum and vector keys (they need `==` first);
  - `==` on maps;
  - maps across the Godot bridge (a load error says so).
- **Map constants are rebuilt at every use** (all constants are re-lowered at each use). Done in plan 1 of the 3D work (Task 4): a map constant is built once, before `main`. Original note: a lookup-table constant used inside a hot function builds a fresh map each call: a review measured 616 MB for 2M calls. Hoist constants into the program arena, or build each once.
- **Lookups are runtime calls.** Inlining the probe for integer keys into generated code would remove the call; on a map that fits in the cache, the call is most of the remaining cost.
- **In-order integer keys miss the cache** where C#'s identity hash walks memory in order (2.5x behind in `kestrel/bench/maps`). The scattering hash is the safer default; probing that tolerates identity hashes (as Python's does) is the alternative.
- **`print(m, m.remove(1), m)`** printed the first `m` after the removal (print arguments are evaluated first, and the first is a header copy sharing entries). Done (interfaces' final fix): `print`'s arguments get `check_changed_later`'s rule, so this is "`m` is passed here, but a later argument changes it".
- **`m.get(k, d)` copies a heap-holding default twice** (once as an argument, once when the result is stored), even when the key is found. Wasted arena memory, not wrong.
- **A loop nested over the same map** makes the outer loop re-index its value on every use (`MapLock` counts as a write). Correct, but slower.

## `par for` (built 2026-10-09)

- **Not yet:** `par for` over maps, and SIMD.
- **Reading the collection conservatively:** any read of the collection (even `xs.len`) or of an alias of what it belongs to is an error while the body changes items. A finer rule would allow reads that can't see other items.
- **A failing pool thread parks for good** (unwinding through compiled code isn't safe), and the pool starts a replacement. A failure while holding the growth lock would deadlock the others; it needs an allocation near `isize::MAX`, so it isn't reachable in practice.
- **Growth in shared arenas takes one lock per job.** It's rare (growth is geometric), but a body that grows many lists made outside the loop will contend on it.

## Do in 2c

- **Strings (2b-ii scope).** Interpolated strings work anywhere now and live in arenas (see "Found while building strings anywhere"). `==` works on strings. Length, indexing, concatenation and the string methods came with the list and string basics (2026-10-17). Strings nested in a printed struct are quoted, but `"` inside them isn't escaped.
- **Exclusivity** only checks bare identifiers (`f(var a, a + 1)` passes; 3+ `var` uses of one local give duplicate diagnostics). Redesign it around place paths once aggregates are passed by pointer.
- **Cleanup stack in codegen.** `break`/`continue`/`return` jump directly, but `arena { }` exits and scope-end drops need cleanup on every exit edge.
- **Runtime stdout buffer is thread-local.** It must change before `par for` (sub-project 3).

## Cheap, fix when the area is next touched

- `-2147483648` / `i64::MIN` literals can't be written (special-case `Neg(Int)`).
- Constants are re-lowered at every use (exponential on chained constants; an overflow in a constant traps at each use). Heap constants (arrays, lists, maps and what holds them) are now built once, before `main` (plan 1 of the 3D work, Task 4), so for them this is done. Scalar, string, vector and plain-struct constants are still re-lowered at each use.
- Runtime: `print_str` should guard `len <= 0`; `eprintln!` in `kestrel_rt_panic` can abort on a broken pipe.
- CLI: `-o` swallows the next flag; `run`/`check` silently accept `-o`/`--release`; unknown `--` flags are ignored; the temp object file has a predictable name.
- Each runtime function is registered in three places (runtime, `RtFuncs::declare`, `jit_symbols`): generate the last two from one table. The native-libs list in `link.rs` is a hardcoded snapshot of `rustc --print native-static-libs`.
- Harnesses: `//expect:` (no space) and empty `// error N:` messages aren't caught; the caret test only checks that `^` exists; `errors.rs` duplicates `programs.rs` helpers.
- Residual f32 rounding for non-literal float expressions widened to f64 (`0.1 * 2.0 * x`). This follows the typing rules.
- Diagnostics polish: CRLF caret overcount, tab misalignment, `0x` says "too large", `42abc` lexes silently as two tokens, `pure fn foo` misses the "no `fn`" hint, the loop-variable assignment message suggests `var`.
- Add `rust-version` to the workspace (`is_none_or` needs Rust 1.82+).
- A module file that exists but isn't UTF-8 (say UTF-16, from PowerShell's `>`) is skipped while loading, and then reported as missing. Say it can't be read, and why.

## Performance (from kestrel/bench/2a/results.md)

- **Division and remainder by a constant** emit a real `idiv` behind the zero and `MIN/-1` checks; Rust/C use shifts or magic multiplies. Collatz is 5.2x slower than Rust because of this (Rust with a hidden divisor: 407 ms vs Kestrel 518 ms). Fix in codegen: a non-zero constant divisor other than -1 skips the checks and strength-reduces. Power-of-two shifts + check elision done in 59dd0f2 (collatz 518 → 237 ms). Still open: magic-number multiply for other constants; `x % 2^k == 0` as a mask test.
- **Call overhead** on recursive code (fib 2.3x vs Rust): Cranelift doesn't inline or transform recursion. The KIR inliner (below) took small functions and methods, but not recursion. A release backend (a second code generator, sub-project 5) is the open route.
- **Checks cost 2.4x on the 3D voxel world** (kestrel/bench/voxels3d/results.md: 1.2 s checked vs 0.49 s `--release`). Every overflow check is a branch that keeps its work in the loop. Range analysis would drop many (`h & 0x7FFFFFFF` times a 31-bit constant can't overflow an `i64`), and a loop-invariant check could be hoisted with its value. **Range analysis is built and measured (2026-10-20)**: the voxel world's checked build is 1.09x its `--release` build (was 2.4x), and 14 checks per sample remain, mostly loop-invariant ones, so hoisting them is still open.
- **Scalar replacement is done (2026-10-11):** small structs and the fields of `var` parameters and loop elements stay in registers, so the particle step is within 15% of Rust (it was 3.4x). What is left of sub-project 5 is listed under "Found while building scalar replacement" above: bounds checks and list-header reloads (the draw copy is still about 4x Rust; the 0.3 ns an element it lost to scalar replacement was won back by Task 6), vectorizing, and range analysis for overflow checks.
- **The KIR inliner** takes callees up to 256 nodes, at most 8 deep, with a budget of 4,096 added nodes per caller. A smarter heuristic (bonus for constant arguments, single call sites) may beat a flat limit.

## Measured

- `kestrel run tests/programs/fib.kes` (release CLI, PowerShell, median of 7) takes 16.6 ms total, against 15.8 ms for bare process start. Git Bash adds about 60 ms of spawn overhead to any measurement.
- `kestrel build` (codegen + MSVC link) takes about 100 ms beyond process start.
