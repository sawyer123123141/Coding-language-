# The I/O round: bytes, files, replays and the small modules (design, 2026-10-29)

Determinism round 2 of `2026-10-27-determinism-proposal.md`, widened by `2026-10-29-roadmap-proposal.md` (step 2). The user ruled the order on 2026-10-29 and opted out of design questions ("start now"), so every decision below is the controller's under the user's standing delegation, except names, which are the user's and are listed at the end with the picks that are built.

## Why

A Kestrel program today cannot read or write a file, read the clock without a window, read its command line or the environment, or turn a value into bytes. So the voxel engine cannot save, Pong's replay lives only in memory, and no program can be replayed from a file. Everything that makes Kestrel's determinism useful outside one process is missing.

## Goals

- G1. `to_bytes(x)` and `from_bytes[T](b)` turn any snapshot-able value into bytes and back, with the same bits after a round trip (`checksum` equal), never crashing on bad input.
- G2. `first_difference(a, b)` names the first field where two values differ, as a path with both values.
- G3. Built-in modules `file`, `time`, `args` and `env`, usable in `kestrel run`, built exes and the test harness, with no window.
- G4. Any window program can be recorded to a replay file and played back exactly, and the playback says where its output first differs. One self-contained file is the bug report.
- G5. The voxel engine saves and loads its world (seed, player, edits). Pong writes its replay to a file and checks it from a file.
- G6. `@prove(deterministic)` names the new effects in its own words ("reads a file", "reads the clock", "reads the command line").

## Non-goals

- JSON, HTTP, UDP, audio, hot reload (later rounds; hot reload is roadmap step 3 and reuses the bytes walk).
- Versioned save formats that read an older layout (the fingerprint refuses them; migration is a later round).
- A per-user save folder (Godot's `user://`); paths resolve next to the program for now.
- Recording `file` reads: a replayed program reads its own files again.

## Anti-goals (failures even if they "work")

- `from_bytes` that panics, allocates by a length a hostile file claims, or returns a value whose `checksum` differs from the original's.
- A replay that "matches" because output was not compared, or that drifts because a native's result was not recorded.
- New natives that need the window library linked into a program that opens no window.
- A new syntax form beyond the one the generics spec already has (`name[T](...)`).

## The design (decisions)

1. **The snapshot rule is the bytes rule.** `to_bytes`, `from_bytes` and `first_difference` accept exactly what `checksum` accepts (`kestrel_check::snapshot`): no `Arena`, GPU object, interface value or function value anywhere in the type. Lists, arrays, maps, strings, optionals, enums and structs are fine. The refusal messages are the checksum ones with the function's name ("`to_bytes` can't take a `World`, which holds an `Arena`; save the fields that hold the game's state instead").

2. **The byte format** (little-endian throughout, no padding, defined so a Rust reference can write and read it):
   - A header: the 4 bytes `KESB`, a `u8` format version (1), then an `i64` **type fingerprint**: the `checksum` of the type's canonical description string, built at compile time: `struct Game{ball:vec2,vel:vec2,pad1:float,...,rng:struct Rng{seed:i64}}`, `list[T]`, `array[T]`, `map[K,V]`, `T?`, `enum State{Alive,Dead{since:int}}`, scalars by name, a type met again inside itself written by name only. Renaming a field or reordering fields changes the fingerprint, so a save of one shape is never read as another.
   - A `bool` is one byte, 0 or 1. `u8` one byte, `int` four, `i64` eight, `float` four raw bits, `f64` eight raw bits (NaN payloads kept, so the round trip's `checksum` is equal), vectors their lanes in order.
   - A string is a `u32` byte length then its UTF-8 bytes. An array or list is a `u32` length then its items. A map is a `u32` live count then each live key and value in insertion order. An optional is one byte, 0 or 1, then the payload when 1. An enum is a `u32` variant number then that variant's fields. A struct is its fields in order.
   - `to_bytes` runs two generated passes, size then write, so the result is one `list[u8]` with `cap == len`, allocated once in the current arena. It is `pure`; under `@prove(no_alloc)` it counts as that one allocation ("`to_bytes` allocates its result").
   - `from_bytes[T](b) -> T?` returns `none` on: a wrong magic, version or fingerprint; bytes that end early; a `bool` or optional byte that is not 0 or 1; a variant number past the enum; a string that is not valid UTF-8; a length that exceeds `i32::MAX` or the bytes left (a claimed length is checked against `remaining / min_item_size` before any allocation; an item of an empty struct takes 0 bytes, so a list of those is read with no loop, and items much bigger in memory than in bytes grow their list as they are read instead of getting room for the claimed length up front); a duplicate map key; or trailing bytes. It never panics. A map is built by adding each key, and its hash is unseeded (determinism needs the same table everywhere), so a hostile file can pick keys that collide and make a large map slow to build. Values are built in the current arena; on `none` what was built so far stays in the arena until the arena is freed (documented). A read list gets `cap == len`.
   - The explicit type argument is the only form: `from_bytes[Game](b)`. The parser treats `from_bytes` like `array`, `list` and `map` (`primary.rs`: brackets after it are types), and `check_type_call` gains a `from_bytes` arm that resolves the type and judges it by decision 1. `from_bytes` without brackets says "`from_bytes` needs the type to read, like `from_bytes[Game](bytes)`".

3. **`first_difference(a, b) -> string?`**, `pure`. Both arguments have the same type (decision 1's rule). `none` when equal (bit-equal for floats, so `-0.0` and `0.0` differ, and two NaNs with the same bits are equal). Otherwise the path of the first difference in the walk order of decision 2, then both values as `print` prints them: `ball.x: 1.5 and 1.25`; `inputs.len: 3 and 4` (then nothing deeper); `inputs[2].p1.up: true and false`; `scores["ana"]: 3 and missing`; `state: Alive and Dead` for a variant difference; `target: none and a value`; a whole scalar gives `1.5 and 1.25`. Two floats that print alike but differ in bits get the bits appended: since `print` writes the shortest text that reads back to the same value, that only happens for NaNs, `nan and nan (bits 0x7FC00000 and 0x7FC00001)`; `-0.0` and `0.0` print as `x: -0 and 0`. Two maps with the same entries in another order differ (`scores: the same keys in another order`), because insertion order is the value's order for `to_bytes` and `checksum` too. It allocates the string in the current arena (`none` allocates nothing: the path is built only once a difference is found).

4. **Natives may return strings, lists and optionals.** `result_ok` in `check/src/natives.rs` grows to allow `string`, `list[T]` of plain items or strings, and `T?` of any allowed result. Codegen already returns every aggregate through the `out` pointer that comes first, so the Rust side writes the 16-byte string, 32-byte list (`cap == len`, the current arena) or the optional (tag byte at 0, payload at `align(T)`). A native `[u8]` parameter accepts a `list[u8]` as today's array rule does. A native whose result is a string, list or optional **never runs in a scratch arena** (`wants_scratch` in `check/src/natives.rs` excludes it, and the checker's region for its arguments is the enclosing one), so `file.read("save{n}.bin")` returns memory that lives in the current arena, not in a scratch freed at the call. Nothing else about natives changes.

5. **The `file` module** (natives in the runtime crate, exported `kestrel_file_*`, no window library; relative paths resolve next to the program under `run` and next to the exe when built, as `window.screenshot` does; absolute paths are used as given):
   - `file.read(path: string) -> list[u8]?`, `file.read_text(path: string) -> string?` (`none` when unreadable, or not UTF-8 for `read_text`)
   - `file.write(path: string, bytes: [u8]) -> bool`, `file.write_text(path: string, text: string) -> bool` (whole-file replace)
   - `file.exists(path: string) -> bool`, `file.list_dir(path: string) -> list[string]?` (names only, sorted bytewise, `none` when not a directory; `list` itself is a reserved built-in name, so a native cannot be called that), `file.make_dir(path: string) -> bool` (parents too), `file.remove(path: string) -> bool` (a file, never a directory)
   - `file.error() -> string`: why the last call failed, as a sentence that names the path ("could not open `save.bin`: the system cannot find the file specified"), empty after a success. Thread-local.

6. **`time`, `args`, `env`** (same home and linking as `file`):
   - `time.now() -> f64` seconds since the program started (monotonic), `time.unix() -> i64` seconds since 1970 UTC, `time.sleep(seconds: f64)`.
   - `args.all() -> list[string]`: under `kestrel run game.kes a b`, `["a", "b"]`; in a built exe, everything after the exe. `args.program() -> string`: the `.kes` path under `run`, the exe path when built. The CLI passes through every word after the file that is not one of its own flags, and `--` ends its flags.
   - `env.get(name: string) -> string?`.
   - All four modules are effectful natives: `pure` functions and `par for` bodies cannot call them. `Effect::of_native` gains `File` (", which reads a file" / "writes a file"), `Clock` for `time.now`, `time.unix` and `time.sleep` (", which waits on the clock" for sleep) and `Process` (", which reads the command line" / "the environment"), each with the `deterministic` message's help line ("pass the value in as a parameter").
   - The JIT finds them through `kestrel_runtime::native_symbols()`, chained after the window table in the CLI; exes find them in either staticlib. `needs_window` stays false for the four. The Godot add-on keeps rejecting built-in modules (unchanged).

7. **Record and replay** (G4), in the window crate's backend, where the headless backend lives:
   - `kestrel run game.kes --record run.replay` wraps the active backend (real, hidden or headless). **Every native of the `window`, `draw`, `draw3d`, `input` and `time` modules that returns a value** appends its result to a native log in call order (one byte naming the native, then its value in the decision 2 encoding; `draw.text_width` and `draw.load_texture`'s size included, so a real-window recording cannot drift); natives without a result are not logged. Every `print` line is copied into an output log. The file is written when the program ends: window closed, headless frames done, `main` returned, **or a runtime failure** (the runtime's `fail` path writes it before exiting 101, and the failure message is the log's last entry, flagged as a failure). Format: the 4 bytes `KESR`, a `u8` version (1), then as strings the program path, the compiler version (`CARGO_PKG_VERSION`), the build mode (`checked` or `release`), the window mode (`real`, `hidden`, `headless` or `headless-quiet`), then the program arguments (`args.all()`), the frame count (`window.frame` calls), the native log and the output log, each length-prefixed.
   - `kestrel replay game.kes run.replay [--release]` compiles the program and runs it on a **replay backend**: no window is opened; `args.all()` returns the recorded arguments; each logged native returns the next recorded value, failing with the call-order message below when the next entry names another native or the log has ended; `window.frame()` returns false once the recorded frame count is reached. Draw natives take the headless path, and print their headless lines only when the recording was `headless` (not quiet), so a non-quiet headless recording compares its draw calls too. The run's `print` lines are compared with the output log as they are printed. A recorded failure must happen again with the same message at the same place. The result is one of:
     - `replay matches (N frames, M lines)`, exit 0;
     - `replay differs at frame F, line L:` then `  recorded: ...` and `  this run: ...`, exit 1;
     - `replay differs at frame F: the program called input.mouse where the recording has window.delta` (or `where the recording has ended`), exit 1;
     - `replay stopped: the program failed with exit code N` when the run ends with a code of its own (a failure inside the runtime that the recording does not have), exit 1;
     - a file that cannot be read (missing, not `KESR`, a later format version, truncated): `run.replay is not a Kestrel replay file` or `... was written by a newer Kestrel`, exit 2.
     A different compiler version, build mode or program path is reported on stderr first (`run.replay was recorded from pong.kes by Kestrel 0.3.1 in a checked build; this is 0.4.0`) and the replay still runs, since the bits may still agree; the exit code is then the comparison's. Differences surface where the program prints: a program that prints a checksum every second finds a desync within a second, one that prints only at the end finds it at the end.
     A program that does not compile under `kestrel replay` exits 1, as under `kestrel run`. A replay file that is missing or that the system cannot read says `could not read the replay file run.replay: ...` with the system's words (exit 2), as the code words it. Under replay, `args.program()` is the program path as typed for `kestrel replay`, not the recorded one. A recording that cannot be written when the program ends normally exits 3 (`kestrel: could not write the replay file ...`); when the program fails, the reason is printed on stderr and the run still exits 101.
   - The replay backend is the third `Kind` beside `Real` and `Headless`. The recorder and player live in the runtime crate (so the `time` natives and `print` reach them), and each logged native wraps its own result (there is no single choke point: `state::view` only says where to read). `file` and `env` are read again at replay.

8. **Pong** writes its replay to `pong.replay` next to the program at the end of a run (`to_bytes` of `Replay`, then `file.write`), and when `args.all()` holds a path it loads that file instead of playing (`file.read`, `from_bytes[Replay]`) and prints the check's result (`replay matches (N frames)` or `replay differs at frame K`). A Kestrel program cannot set its exit code (`main` returns nothing; an `exit(code)` built-in is a follow-up with a name for the user), so the tests read the line. The demo's own replay check (R key) stays; the headless tests cover the write, the read and `kestrel replay` of a headless run.

9. **The voxel engine saves and loads.** `World` gains `edits: list[Edit]` (`Edit { cell: ivec3, id: u8 }`), appended by `edit_at`, and a column becoming Ready re-applies the edits that fall in it. `Save { seed: int, player: Player, edits: list[Edit] }` goes through `to_bytes` to `world.save` next to the program on F5 and comes back on F9: **the world is rebuilt from the save's seed** (`new_world(save.seed, CAPACITY)`, so this session's unsaved edits are gone and a save made with another seed loads), the edits list is the save's, the player is set, and streaming regenerates the columns around the player and re-applies the edits as each becomes Ready. No automatic load at start. The engine test saves after scripted edits, reloads in a fresh world and compares `checksum` of the player and of the blocks around the edits.

10. **Docs.** The syntax spec's "Built-in functions" gains `to_bytes`, `from_bytes` and `first_difference` in the `checksum` bullet's style; "Built-in modules" lists `file`, `time`, `args` and `env` with their tables; the README gets a short "Files and replays" section with `--record` and `kestrel replay`; the determinism results file gains the measurements below.

## Constraints

- No new syntax beyond `from_bytes[T](...)`, which is the generics spec's existing form. No new Cargo dependencies. Files under about 400 lines. Messages are sentences that say what to do.
- Natives never `println!`; they go through the runtime's `print` sink. A native failure that stops the program uses `fail_at_call`.
- Every new native and built-in is covered in the JIT and in a built exe (the program tests run both).
- Headless and hidden windows only in tests, as always.

## Tests

- Runtime: the encoding written by a Rust reference equals `to_bytes`'s bytes for a table of values; `from_bytes` on every truncation of a valid file, and on 10,000 random single-byte mutations, never panics and returns `none` or a value whose re-encoding is valid.
- Programs: round trips for every type kind (nested lists, maps with string keys, optionals, enums with fields, a self-containing type through a list) with `checksum` equal; each `none` reason in decision 2; `first_difference` on each path shape in decision 3; refusals (an `Arena` field, a function value, a missing type argument); `to_bytes` under `@prove(no_alloc)` naming its allocation.
- CLI (Rust tests in a scratch dir): every `file` native, including `error()` after each failure; `args.all()` through `kestrel run` and through an exe; `env.get`; `time.now` increasing and `time.sleep` waiting at least its argument.
- Record and replay: a headless Pong run recorded, replayed and matching; the same with one recorded input byte flipped, differing at the last frame (where Pong prints); a program whose call order changes between runs, reported as such; a log that ends early; a recording that ends in a failure, replayed to the same failure; a version mismatch reported and still compared; a non-quiet headless recording whose draw lines compare.
- Voxel: decision 9's test. Pong: decision 8's write, read and check, plus the injected-desync run differing.
- `deterministic` error tests naming each new effect.

## Measurements (when the PC is free)

In `kestrel/bench/determinism/results.md`: `to_bytes` and `from_bytes` of Pong's `Game` and of a 100k-item list against a snapshot copy of the same; the per-frame cost of `--record` on headless Pong and the voxel engine (median frame work time with and without); file read and write of 1 MB.

## Names (the user's call; the picks are built and renaming is cheap)

| Pick | Alternatives |
|---|---|
| `to_bytes(x)`, `from_bytes[T](b)` | `save`/`load`, `encode`/`decode`, `bytes(x)`/`T.from_bytes(b)` |
| `first_difference(a, b)` | `diff`, `where_differ` |
| `file` with `read`, `read_text`, `write`, `write_text`, `exists`, `list_dir`, `make_dir`, `remove`, `error` | `fs`; `read_bytes`/`read_string`; `entries` or `names` for `list_dir` (`list` is reserved); `last_error` |
| `time` with `now`, `unix`, `sleep` | `clock`; `seconds` |
| `args` with `all`, `program` | `process.args()`, `main(args)` |
| `env` with `get` | `process.env(name)` |
| `kestrel run --record x.replay`, `kestrel replay game.kes x.replay` | `--replay` on `run` |
| `world.save`, `pong.replay` as file names | the user's |

## Left to the builder

The exact shape of the generated size and write passes (one helper per type each, as `checksum` does, or one with a mode flag); whether `first_difference` formats through the existing `Format` ops or a runtime call; how the recorder captures `print` (a tee in the runtime's `OUT` sink is expected); the native log's per-native byte ids; the scratch-dir convention for file tests; where the CLI keeps the program arguments for the JIT (a runtime global set before `run_main` is expected).
