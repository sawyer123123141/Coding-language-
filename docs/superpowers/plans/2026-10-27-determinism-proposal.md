# Proposal: provable determinism and fast snapshots (2026-10-27)

Status: a proposal for the user. Not a spec, nothing built. Every new name below is the user's call; each has a recommendation.

## Why this, and why Kestrel

Kestrel needs something other languages don't have. It already proves performance (`@prove(no_alloc, no_checks)`). The next thing to prove is that **the same inputs give the same game, bit for bit**, and to make **saving and restoring the whole game state cheap**. Together those give rollback multiplayer, input-only replays, rewind and replayable bug reports.

The demand is real and people pay for it today:

- **Photon Quantum** sells a deterministic engine for Unity with predict/rollback netcode. To get determinism out of C# it replaces `float` with its own fixed-point type `FP` (48.16 bits in an `i64`), because C# runtimes don't promise identical floats ([Quantum intro](https://doc.photonengine.com/quantum), [`FP` API](https://doc-api.photonengine.com/en/quantum/current/struct_photon_1_1_deterministic_1_1_f_p.html)).
- **Factorio** runs multiplayer as deterministic lockstep (only inputs are sent); any difference between machines is a desync, and a desync report records the state but not the cause ([FFF #188](https://factorio.com/blog/post/fff-188), [wiki](https://wiki.factorio.com/Desynchronization)).
- **GGPO-style rollback** (fighting games) asks the game to save the whole state, load it back, and advance exactly one frame from given inputs ([GGPO callbacks](https://pkg.odin-lang.org/vendor/ggpo), [GGRS](https://docs.rs/ggrs/latest/ggrs/)).

Kestrel owns its compiler and runtime, so determinism can be a property of the language, not a discipline the programmer keeps.

## What Kestrel already gives

- **`pure` functions can't read the outside world.** Natives that aren't `pure` (`window.delta`, `window.ticks_usec`, `input.held`, `draw.*`) and the random built-ins (`randf`, `randi_range`, `rand_seed`) are effects, so a `pure` function can't call them. A `pure` function may still change its `var` parameters, so `pure step(var g: Game, i: Inputs)` is allowed today.
- **No globals.** Top-level `var` is refused ("global variables aren't supported yet"); the voxel engine keeps everything in `main`'s variables.
- **Maps keep insertion order**, and the hash doesn't depend on addresses or a random seed (`runtime/src/map.rs`: a fixed multiply for ints, FNV-1a for strings). Iterating a map is deterministic today.
- **`par for` has no reductions.** Its body may change only its own item and locals, so the result can't depend on which thread ran which chunk. (Two leftovers: when two iterations fail, which message wins depends on timing; and memory placement differs, which nothing can observe.)
- **Sorting is stable** and has no randomness. **Memory is zeroed** (list growth's unzeroed memory is written before it is read); only struct padding may hold stale bytes, which matters only to something hashing raw bytes.
- **Floats are plain IEEE:** Kestrel emits no fused multiply-add, Cranelift doesn't fuse or reassociate float math on its own, and `sqrt`, `floor`, `ceil`, `min`, `max`, `abs`, `round` are exact instructions or Kestrel's own code.
- **Values copy deeply.** `let saved = game` is already a full snapshot: lists, maps and strings are copied, not shared.

## What breaks determinism today

1. **Transcendental math goes through the platform's C library.** `sin cos tan asin acos atan atan2 exp log pow` call `kestrel_rt_*` in `runtime/src/math.rs`, which calls Rust's `f64::sin` and friends, which on Windows call the Microsoft C runtime. On x64 that runtime detects FMA3 at startup and switches to different implementations, and Microsoft says this can change results ([`_set_FMA3_enable`](https://learn.microsoft.com/cpp/c-runtime-library/reference/get-fma3-enable-set-fma3-enable?view=msvc-170)). So **the same exe can give a different `sin` on a CPU with FMA3 than on one without**, and Linux's glibc gives different answers again. This is the main real hole.
2. **Exes are compiled for the build machine's CPU.** `codegen/src/isa.rs` uses `cranelift_native::builder()`, which turns on every host feature (AVX, BMI ...). Float results don't change (no fusing), but an exe built on a new CPU may crash on an older one, and a replay can't play where the exe can't run.
3. **The floating-point control register.** Rounding mode and "flush denormals to zero" live in the CPU's MXCSR register. Nothing in Kestrel changes it, but a native library (a GPU or audio driver) can, and then the same code rounds differently.
4. **NaN bit patterns** differ between CPU families. Kestrel can't print them, but a checksum over raw bytes would see them.
5. **The global random generator** starts from the clock and a stack address. Fine for a single-player toy, useless for replays. `rand_seed` makes it repeat, but it is one hidden generator per thread, so it isn't saved with the game state.
6. **Variable time steps.** The voxel engine's `step_player(var p, w, i, window.delta())` moves the player by the frame's real duration, so two runs never match. Deterministic games step by a fixed `DT`.

## 1. What the user gets

**Rollback multiplayer that feels instant.** Your inputs apply at once. The other player's inputs arrive late, so the game guesses them (same as last frame). When the real ones arrive and differ, it restores the snapshot from that frame and re-runs the frames since, in one display frame, so the correction is invisible.

```
// A late input for frame f arrived and it differs from the guess.
if inputs[f % N] != guess[f % N] {
    g = ring[f % N]                    // restore the snapshot of frame f
    for k from f to now {
        step(var g, inputs[k % N])     // re-run up to the present
        ring[(k + 1) % N] = g          // and re-save each frame
    }
}
```

**Input-only replays.** A replay is the seed plus one small input struct per frame: 60 frames a second of a two-player `Inputs` is about 1 KB a second. Playing it back re-runs `step`.

```
var g = new_game(seed: replay.seed)
for k from 0 to replay.inputs.len {
    step(var g, replay.inputs[k])
    if k % 60 == 0 and checksum(g) != replay.sums[k / 60] {
        print("replay differs from the recording at frame {k}")
        return
    }
}
```

**Rewind** (Braid, Prince of Persia): keep the last 600 snapshots and walk back through them.

```
if input.held(Key.R) and history.len > 0 {
    let prev = history.pop() else { return }
    g = prev
} else {
    step(var g, read_inputs())
    history.add(g)                      // a ring in practice, see section 4
}
```

**Replayable bug reports.** On a crash or a "report bug" key, write the last snapshot and the inputs since it. The developer replays it and hits the same bug on the same frame, every time; a desync report can name the first field that differs (`first_difference`, section 6).

**Hot reload that keeps state.** Snapshot before reloading, then put the state back into the new code's types field by field, by name (needs serialization; a later round).

## 2. What "deterministic" means, in three levels

- **(a) Same exe, same machine.** True of Kestrel today, except the clock-seeded global random and `window.delta`. Costs nothing. Good for rewind and local replays; not for multiplayer.
- **(b) Same exe, any x86-64 machine.** What rollback and lockstep need between players on PCs. Costs: replace the platform math library with Kestrel's own; pin MXCSR; build exes for a fixed baseline CPU instead of the host's.
- **(c) Any build, any platform.** Same source, any compiler build, x64 and ARM (Steam Deck is x64; Switch, phones and Apple Silicon are ARM). IEEE basic operations are identical on ARM, so with Kestrel's own math library this is reachable, but it also needs every future optimization to keep float results bit-identical, and nobody can test it until Kestrel has an ARM backend.

**The float choices** (floats are the hard part):

| Option | What it costs | Verdict |
|---|---|---|
| Strict IEEE floats (no fusing, no reassociation; already true) + Kestrel's own math functions (a pure-Rust port of musl's libm) + MXCSR pinned + exes for a fixed baseline CPU | `sin` and the rest maybe 1.2 to 2 times slower than the Microsoft FMA3 versions (to measure); a few ns per frame for MXCSR; exes lose host-only instructions (likely nothing measurable: Kestrel doesn't use them for floats) | **Recommended** |
| Fixed-point number type (Photon's `FP`) | Every game rewritten in a new number type; slower `*` and `/`; no `sin` without tables; precision traps | Not as the main path. Photon needs it because C# can't promise floats; Kestrel can. Maybe later as a library type. |
| Software floats (every `+` in software) | 10 to 50 times slower | No |
| A "strict float" mode flag | Two kinds of build to explain and test | No: make strict the only mode. Kestrel has no fast-math today; keep it that way as a rule. |

`sqrt` stays an instruction: IEEE requires it exactly rounded, so it is the same everywhere. `f32` versions of `sin` and the rest already compute in `f64` and round once, which is deterministic. NaNs are canonicalised only where something reads raw bits (checksums, serialization), not on every operation (Cranelift's `enable_nan_canonicalization` would cost on every float op).

**Recommendation: promise (b), design for (c).** Concretely: same exe, any x86-64 CPU and any Windows version; also the JIT (`kestrel run`) and an exe from the same compiler and the same source agree, so two players can use either. A checked build and a `--release` build compute the same values unless the checked one stops on an overflow. (c) becomes a rule for the compiler ("no optimization may change a float result") with the promise waiting for an ARM target.

## 3. `@prove(deterministic)`

**Pick:** a fifth contract next to `no_alloc`, `no_checks`, `no_panic` and `inline`.

```
@prove(deterministic, no_alloc)
pure step(var g: Game, i: Inputs) { ... }
```

**What it means:** the function's result, and every change it makes to its `var` parameters, depends only on its arguments, at level (b).

**How it relates to `pure`:** after round 1 fixes the math library, **every `pure` function is deterministic** as a language rule, so a `pure` function always meets the contract. The contract still earns its place: it states the promise where a teammate sees it and fails the build with a chain when someone breaks it three calls deep; it covers functions that aren't `pure` only because they `print`; it goes on loops; and `kestrel check --report` lists it.

**What it checks** (on the events walk the other contracts already use, `kir/src/range/events.rs` and `range/contracts.rs`: natives already show up as `Unknown::Native`, function values as `Unknown::Indirect`, interface calls as `Unknown::Virtual` with the method's promises):

- **No clock or input reads:** no native outside a whitelist. The whitelist is the pure math natives and `print`. Everything in `window`, `input` and `draw` breaks it.
- **No shared random:** `randf`, `randf_range`, `randi_range`, `rand_seed` break it. Randomness comes from an `Rng` value passed in (section 6), which is snapshotted with the game.
- **Function values:** a `pure (A) -> R` value is fine; a non-pure one breaks it (its body isn't known). An interface method call keeps what the interface method promises, as today.
- **Maps:** nothing to check; iteration order is insertion order. This becomes a written rule ("map order never depends on hashing").
- **`par for`:** fine, because bodies can't share writes. A future `reduce` must combine in a fixed order by chunk index, with chunk sizes that don't depend on the thread count; the contract will check that when `reduce` exists. Two failing iterations should report the one with the lowest index (a small runtime change), so even the error is deterministic.
- **Uninitialised memory:** nothing to check at the language level (zeroed allocations); checksums and serialization walk fields, never padding.
- **Natives in built-in modules** can be marked deterministic later, as "natives marked safe" is already a follow-up for the other contracts.

**Error messages** (in the style of today's contracts):

- "`step` can't be proven `deterministic`: it calls `window.delta` (line 14), which reads the clock; pass the time step in as a parameter, like `step(var g, i, DT)`"
- "`step` can't be proven `deterministic`: `spawn_enemy` (line 30) calls `randi_range` (line 8), which uses the program's shared random generator; keep an `Rng` in the game state and call `g.rng.randi_range(1, 6)`"
- "`update` can't be proven `deterministic`: it calls `input.held` (line 5), which reads the keyboard; read input outside and pass it in an `Inputs` struct"
- "`step` can't be proven `deterministic`: it calls the function value `on_hit` (line 21), which isn't `pure`; give it a `pure` function type, like `pure (Fighter) -> int`"
- "`deterministic` isn't checked on natives; `@prove` goes before a function, a method or a loop"

**Rejected:** a `deterministic` keyword (keywords are for structure, `@` for promises: the user's own rule); making `pure` alone carry it with no annotation (invisible, and it can't cover a loop or a function that prints). **Open:** should `print` be allowed (it changes nothing the game reads, but a rollback re-runs it, so lines repeat)? Recommendation: allowed.

## 4. Snapshots

**What game state is in Kestrel today:** the values `main` owns (structs held in locals), which reach lists, arrays, maps and strings in the program arena or in an `Arena` field; library types like `Pool[T]`, `Grid[T]` and `Ring[T]` are ordinary structs of lists. GPU objects (`Texture`, `Mesh`) and `Arena` are owning types and are not game state; they belong to the renderer.

**A snapshot is a copy of a value**, and Kestrel already copies values deeply: `let saved = game`. Two options for how that copy is made:

- **A per-type generated copy (recommended for round 1).** What `let saved = game` already compiles to: the compiler generates the walk for each type (it already generates one for printing). It works for any state shape.
- **A byte copy of an arena** (copy its used bytes; restore them to the same addresses). Fast for big state, but all state must sit in one arena pointing nowhere else, and arenas never free, so a long game's checkpoint copies old list buffers too. A later optimization.

**The real gap is memory, not speed.** Today `ring[k] = game` copies into the current arena (the program arena), and the old snapshot's buffers become garbage that is only freed at exit. A rollback game that restores a few times a second leaks without end. **The fix needs no syntax:** assigning a copy over an existing value reuses the place's buffers when they are big enough (Rust's `clone_from`). A list of 50 items assigned over a list with room for 64 copies 50 items into the existing buffer and allocates nothing. Then a ring of N snapshots allocates once and never again, and a `@prove(no_alloc)` can cover the save and the restore.

**Checksums** for desync detection: a built-in `checksum(x) -> u64` that hashes a value's contents field by field (following lists and maps, skipping padding, canonicalising NaN). Two peers compare checksums every few frames; a replay stores one a second.

**Cost estimates** (memory copies run at roughly 10 GB/s on one core of the user's Ryzen 7 5800X; to measure, not measured):

- **Pong:** about 100 bytes. A save is well under a microsecond.
- **A 2D fighter:** two fighters, projectiles, an `Rng`: 1 to 4 KB. A worst-case rollback of 8 frames is 8 restores and 8 steps; the copies cost a few microseconds. `step` itself dominates.
- **The voxel engine:** the `World`'s pool holds 441 columns (`CAPACITY`, 21 by 21) of 32 x 128 x 32 blocks = **57.8 MB**. One deep copy is about 6 ms: fine for a save file or a rewind keyframe every second, far too slow every frame (60 a second would be 3.5 GB/s of copying). But almost all of it is **derived**: generated from the seed. The voxel game's real state is the seed, the player and the list of edits, a few KB. The lesson: **put the simulation in its own struct and treat generated data as a cache rebuilt after a restore.** That is a program structure, not a language feature; dirty-region copies are a later optimization if a game truly needs huge mutable state.

**Save and load (the file gap raised on 2026-10-26).** A snapshot in memory is a copy; a save file is the same snapshot turned into bytes. The same generated per-type walk that copies and checksums can write bytes (`to_bytes`) and read them back (`from_bytes`), like Godot's `var_to_bytes` and `bytes_to_var`. With a small `file` module that reads and writes bytes, that is save/load, replays on disk and bug-report files. Round 2.

## 5. The round's goal: a rollback demo

**Two-player Pong with rollback, in one process.** Two simulated peers, each with its own `Game`, exchange inputs through a simulated network that adds a delay (say 100 ms), jitter and lost packets. The screen shows peer 1's view; a key toggles peer 2's. Each peer predicts the other's input (repeat the last one), rolls back on a wrong guess and re-runs. Both peers compare `checksum` every 30 frames and stop with "peers differ at frame 1,204" if they ever disagree. A key saves the input history as an in-memory replay and plays it back, checking the checksums. Real networking (UDP) is a later round: it needs new natives, and one process shows the whole idea without it.

What it needs, and what exists:

| Need | Today |
|---|---|
| Input as a value: `struct Pad { up: bool, down: bool }`, `struct Inputs { p1: Pad, p2: Pad }` | Exists (the voxel engine's `Intent` is this shape). Comparing two structs with `==` may need checking. |
| A fixed time step: `step` takes `DT = 1.0 / 60`, and `main` runs `step` when enough real time has built up (Godot's `_physics_process`) | Exists as ordinary code; `window.delta` stays for drawing. |
| `pure step(var g: Game, i: Inputs)` with `@prove(deterministic)` | `pure` exists; the contract is new. |
| A seeded random value in the state | New: `Rng`. |
| A ring of snapshots that doesn't leak | Storage-reusing assignment: new, no syntax. |
| `checksum` | New built-in. |
| Deterministic `sin`/`atan2` (the ball's bounce angle) | New runtime math. |

```
struct Game {
    ball: vec2
    vel: vec2
    p1: float
    p2: float
    score1: int
    score2: int
    rng: Rng
}

@prove(deterministic, no_alloc)
pure step(var g: Game, i: Inputs) {
    g.p1 += pad_move(i.p1) * PADDLE_SPEED * DT
    g.p2 += pad_move(i.p2) * PADDLE_SPEED * DT
    g.ball += g.vel * DT
    if hits_paddle(g) {
        let a = g.rng.randf_range(-0.6, 0.6)   // the bounce angle
        g.vel = vec2(-sign(g.vel.x) * cos(a), sin(a)) * BALL_SPEED
    }
}
```

A fighter is the same with bigger state; Pong first because it is small enough to finish and still shows prediction, rollback, a checksum and a replay.

## 6. New syntax and names: all the user's call

| Name | What | Recommendation | Alternatives |
|---|---|---|---|
| `@prove(deterministic)` | The fifth contract | `deterministic` | `repeatable`, `same_everywhere` |
| `Rng` | A seeded random generator value, kept in the game state; methods like Godot's `RandomNumberGenerator`: `randf()`, `randf_range(lo, hi)`, `randi_range(lo, hi)` (all `var self`, `pure`) | `Rng(seed: 42)` in a built-in module `random`, written in Kestrel (SplitMix64 with `*%`), so it is `pure` and snapshotted for free | `Random`; reusing the global names as methods is the GDScript-like part |
| `checksum(x) -> u64` | A deterministic content hash of any value without owning types or function values | `checksum`, a built-in | `hash`, `x.checksum()` |
| `first_difference(a, b) -> string?` | The path of the first field that differs ("`p2.pos.x`: 103.25 and 103.2500076") for desync reports | Round 2 | `diff` |
| `to_bytes(x) -> list[u8]`, `from_bytes[T](b: list[u8]) -> T?` | Serialization for saves, replays and bug reports | Round 2; Godot's `var_to_bytes` meaning | `save`/`load`, `encode`/`decode` |
| A `file` module | `file.read(path) -> list[u8]?`, `file.write(path, bytes) -> bool` | Round 2, natives | Part of a larger I/O module |
| A CLI command for replays | Re-run a replay file and compare its checksums | `kestrel replay game.kes run.replay`, round 2 | A flag on `run` |
| Storage-reusing assignment | `a = b` reuses `a`'s buffers | No new syntax: just how assignment works | An explicit `a.copy_from(b)` |
| A deterministic-float rule | Kestrel never fuses or reorders float math | A written rule, no flag | A `--strict-float` flag (rejected above) |
| `fixed` | A fixed-point number type | Not now | Photon's approach |

## 7. Risks, open questions and rounds

**Risks.**

1. **Floats agreeing across machines can't be proven on one PC.** Kestrel's own math library closes the known hole, but "any x86-64 machine" needs runs on several CPUs (Intel and AMD, with and without FMA3 and AVX) and Windows versions, and a driver changing MXCSR could still break it. One unexplained desync after advertising "proven determinism" would hurt the language's name. Mitigation: pin MXCSR at the start of `main`, each `par for` worker and after `window.frame()`; publish a replay-with-checksums test anyone can run; word the promise exactly.
2. **Snapshot memory is harder than snapshot speed.** Storage-reusing assignment touches the copy, move and arena rules, where past rounds found real bugs (double drops, aliasing), and it must keep the escape rules when a value's lists live in different arenas. Big state (the voxel world) needs a cache-and-rebuild structure or dirty tracking, not plain copies.
3. **Scope creep and false promises.** The full feature wants networking, files, serialization, versioned save formats and hot reload. A replay only plays on the same exe (level (b) is per exe), so a game patch breaks old replays unless serialization grows versioning. And "proven" covers the language, not compiler bugs: a wrong optimization could still change a float. Mitigation: a test that runs every program-test with checksums in the JIT, the checked exe and the `--release` exe and requires the same values.

**Open questions.** Should `print` be allowed under the contract? Should `@prove(deterministic)` also require the `var` parameters' types to be snapshot-able (no owning types, no non-pure function values), so it doubles as "rollback-ready"? Recommendation: yes, as a second check with its own message. Should the global `randf` family stay at all once `Rng` exists? Recommendation: keep them for toys; the contract refuses them. How fast is Kestrel's own `sin` against the Microsoft one? Measure in round 1.

**Rounds.**

- **Round 1: determinism and the Pong demo.** (1) Kestrel's own math functions in the runtime, with tests that pin exact bits for a table of inputs, and a speed comparison. (2) MXCSR pinned. (3) Exes built for a baseline CPU (SSE4.1, the SIMD floor), reported in the build output. (4) `@prove(deterministic)` on the events walk, with its messages and error tests. (5) `Rng` in a built-in module. (6) `checksum`. (7) Storage-reusing assignment, with `@prove(no_alloc)` tests over a snapshot ring. (8) The lowest-index failure in `par for`. (9) The Pong rollback demo with a simulated network, checksums between peers, and an in-memory replay. (10) A test that the JIT, checked and `--release` builds give the same checksums.
- **Round 2: saves, replays and bug reports.** `to_bytes`/`from_bytes`, the `file` module, replay files, `kestrel replay`, `first_difference`, a bug-report bundle; the voxel engine's save as seed, player and edits.
- **Round 3: real multiplayer and the rest.** UDP natives and the demo over a real network (or a 2D fighter), hot reload that keeps state (fields matched by name), arena checkpoints or dirty tracking for big state, a `reduce` with a fixed order, and level (c) once there is an ARM target.

**What the first round builds:** everything in round 1 above, ending in a Pong game where two simulated players 100 ms apart both feel instant, never desync, and every match can be replayed exactly.
