# Plan 2a follow-ups

Carried forward from plan 2a's task reviews and final whole-branch review.
Input for writing plans 2b and 2c.

## Language decisions (resolved 2026-10-10)

- **Narrowing integer conversions** (`u8(x)`, `int(x)` from a wider type) panic when the value doesn't fit and checks are on; `--release` truncates. A literal that doesn't fit is a compile error. Float->int still saturates.
- **`while true` with no `break`** counts as never finishing, so it can end a value-returning function.

## Do before or at the start of 2b

Done in 2b-i: labelled arguments evaluate in source order; `func/mod.rs` split; parenthesised spans include the parens; KIR has `Place` + aggregates (struct layouts, return slots, field-by-field copies); compound assignment takes its type from the place; exclusivity works on place roots. Still open from this list: recursion depth limits, interned `Ty` ids for enums, `Diagnostic` labels, `match` arm newline note, `list`/`array`/`map` type constructors, thin KIR-shape tests.


- **Labelled arguments evaluate in parameter order, not source order** (`kir/src/lower/expr.rs` `call_parts`). The ignored test `labelled_arguments_evaluate_in_source_order` in `codegen/tests/jit.rs` documents the expected behaviour. The fix needs synthetic temporaries, and the locals list must be able to grow during lowering. Struct construction with labels will hit the same problem.
- **Split `codegen/src/func/mod.rs`** (378 lines) before adding aggregates: move print/string/call helpers into their own files.
- **Parenthesised expression spans exclude the parens**, which misplaces carets on parent nodes. Fix when postfix `.`/`[]` parsing lands.
- **Recursion depth.** The CLI and runtime now run on 256 MiB stacks, but the parser, checker, lowering and codegen are still recursive with no depth limit.
- **KIR needs places and aggregates.** `Stmt::Assign(LocalIdx, _)` and `Arg::Ref(LocalIdx)` can only target a local. Fields and indexes need a `Place` (Local/Field/Index/Deref). `KTy`, `cl_type` and codegen's `Slot` model are scalar-only, and `expr() -> Value` can't return aggregates (needs destination-passing). `LocalKind::RefParam` is tied to `var`, but spec §2 also passes read-only aggregates by pointer.
- **`Ty` needs interned ids** for structs/enums. `Ty::name() -> &'static str` and `resolve_type` must take the type table, and `collect` needs a two-phase declare. The AST has no ids for patterns, field labels or variant names yet.
- **`Diagnostic` needs secondary labels** (spec §3; the arena-escape error names two lifetimes).
- **Parser notes for `match` and generics.** `newlines.rs` drops a newline before `else` and `}`, so `match` arms like `Air { false }\n else { true }` arrive without a separator, and arm parsing must not require one. `array[int](n)` / `list[Chunk]()` look like indexing followed by a call, so treat `list`/`array`/`map` followed by `[` as type constructors.
- **Compound assignment** takes the operand type from `rhs.ty`. It should come from the place once `Place` exists.
- **Thin KIR-shape tests** (while/for/break/continue/and/or/unary/convert). Add them before refactoring the KIR shape.

## Found while building the Godot demo (2026-10-10)

- **A `pure` function can't call a helper that only changes a `var` argument rooted at its own local** (for example `quad(var out, ...)` filling a local list). This is observationally pure, but the checker rejects it because the helper has `MutatesParams`. Allow calls whose `var` arguments are all rooted at the caller's locals.
- **`let x = opt else` always moves or copies.** Unwrapping an owning optional parameter is therefore an error; a view (like a `match` binding) would fix it.
- **Runtime errors don't name the file** in multi-file programs, only the line.
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
- **Cost of the trig and exponential functions.** `round` is a short inline instruction sequence. `sin cos tan asin acos atan atan2 exp log pow` call `kestrel_rt_math`, so a hot loop full of `sin` pays for the call.
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
- **Struct-valued field defaults are unavailable** (`tint: Color = RED` is an error: "field defaults can't use struct values yet; set the field when you build it"). Constants may construct structs, and field defaults may construct only `vec2` and `vec3`. The check is conservative: `name: string? = NAME` is rejected too, though it would be safe.
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
- **Moves are flow-insensitive.** `if i % 2 == 0 { a.add(c) } else { b.add(c) }` reports "`c` was moved earlier", because move checking ignores that the two branches are exclusive. This was already so before plan 1, and textures and meshes will make it an everyday pattern.
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
- **The build arena holds every chunk's lists until the upload is done.** At `WIDTH` 8 that is about 184 MiB of lists (measured as the program arena of a copy without the `arena { }`: lists grow by doubling and leave their old buffers behind) and a peak working set of 269 MiB, 875 MiB at `WIDTH` 16. A plain build at `WIDTH` 24 would need about 1.8 GiB (extended from those, not measured). The arena frees the CPU data after the upload (the program arena stays about 64 KiB), but the process's working set at the end of `WIDTH` 8 is still 268 MiB. Building and uploading in slabs, each in an `arena { }` of its own, keeps it down (the measured scratch variant peaked at 745 MiB at `WIDTH` 24); the committed viewer doesn't.
- **The viewer draws every mesh every frame.** From about 4 million faces the program waits inside its draw calls (9.6 ms at 4.2 million, 14.2 ms at 7.3 million), while the calls themselves cost 0.4-0.9 microseconds each: the limit is the GPU and its driver, roughly 2 ms for each million faces (an inference from the waiting, not a GPU timing). Skipping meshes outside the camera's view, and drawing fewer faces far away, is the first thing to try.
- **Real bigger worlds cost more a face than the stress runs.** The `WIDTH` 8 world drawn 16 times a frame (7.4 million faces) still holds 60 fps, while the real `WIDTH` 32 (7.3 million) just fails. Candidates, untested: the real world is 560 MB of different vertex data against 35 MB read again and again, and more of it is in view. The first seconds of the big worlds are also slow (`WIDTH` 32: 66, 45, 31 ms, then 14-17); the cause is unknown.
- **The viewer's start view could be better.** It starts outside the world's corner at (-24, 100, -24), so the first view is of the outer walls from below the peaks. A higher start looking down onto the top surface would show the terrain better.
- **`measure.py`'s particles rule warns "held to 60 fps" for any median frame above 16.4 ms,** including a slow one such as 41 ms. It was left as it was; viewer runs use a band of 16.4-17.0 ms instead.

## Maps (built 2026-10-09)

- **Not yet:**
  - values that own an arena;
  - struct, enum and vector keys (they need `==` first);
  - `==` on maps;
  - maps across the Godot bridge (a load error says so).
- **Map constants are rebuilt at every use** (all constants are re-lowered at each use). Done in plan 1 of the 3D work (Task 4): a map constant is built once, before `main`. Original note: a lookup-table constant used inside a hot function builds a fresh map each call: a review measured 616 MB for 2M calls. Hoist constants into the program arena, or build each once.
- **Lookups are runtime calls.** Inlining the probe for integer keys into generated code would remove the call; on a map that fits in the cache, the call is most of the remaining cost.
- **In-order integer keys miss the cache** where C#'s identity hash walks memory in order (2.5x behind in `kestrel/bench/maps`). The scattering hash is the safer default; probing that tolerates identity hashes (as Python's does) is the alternative.
- **`print(m, m.remove(1), m)`** prints the first `m` after the removal: print arguments are evaluated first, and the first is a header copy sharing entries. Function calls reject this through `check_changed_later`; `print` isn't covered.
- **`m.get(k, d)` copies a heap-holding default twice** (once as an argument, once when the result is stored), even when the key is found. Wasted arena memory, not wrong.
- **A loop nested over the same map** makes the outer loop re-index its value on every use (`MapLock` counts as a write). Correct, but slower.

## `par for` (built 2026-10-09)

- **Not yet:** `par for` over maps, and SIMD.
- **Reading the collection conservatively:** any read of the collection (even `xs.len`) or of an alias of what it belongs to is an error while the body changes items. A finer rule would allow reads that can't see other items.
- **A failing pool thread parks for good** (unwinding through compiled code isn't safe), and the pool starts a replacement. A failure while holding the growth lock would deadlock the others; it needs an allocation near `isize::MAX`, so it isn't reachable in practice.
- **Growth in shared arenas takes one lock per job.** It's rare (growth is geometric), but a body that grows many lists made outside the loop will contend on it.

## Do in 2c

- **Strings (2b-ii scope).** Interpolated strings work anywhere now and live in arenas (see "Found while building strings anywhere"). `==` works on strings. Still missing: length, indexing and concatenation. Strings nested in a printed struct are quoted, but `"` inside them isn't escaped.
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
- **Call overhead** on recursive code (fib 2.3x vs Rust): no inlining in Cranelift. Sub-project 5 (release backend) or a small KIR inliner for pure leaf functions.
- **Checks cost 2.4x on the 3D voxel world** (kestrel/bench/voxels3d/results.md: 1.2 s checked vs 0.49 s `--release`). Every overflow check is a branch that keeps its work in the loop. Range analysis would drop many (`h & 0x7FFFFFFF` times a 31-bit constant can't overflow an `i64`), and a loop-invariant check could be hoisted with its value.
- **The KIR inliner** takes callees up to 256 nodes, at most 8 deep, with a budget of 4,096 added nodes per caller. A smarter heuristic (bonus for constant arguments, single call sites) may beat a flat limit.

## Measured

- `kestrel run tests/programs/fib.kes` (release CLI, PowerShell, median of 7) takes 16.6 ms total, against 15.8 ms for bare process start. Git Bash adds about 60 ms of spawn overhead to any measurement.
- `kestrel build` (codegen + MSVC link) takes about 100 ms beyond process start.
