# Determinism Round 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The same inputs give the same game, bit for bit (level b): Kestrel's own math, a pinned float mode, exes for a fixed baseline CPU, `@prove(deterministic)`, a seeded `Rng`, `checksum`, storage-reusing assignment, a deterministic `par for` failure, and a rollback Pong demo that proves it end to end.

**Architecture:** Runtime changes (`libm` math, `fpu::pin`, `par for` failure order, `kestrel_rt_copy_items`, `kestrel_rt_map_copy_into`, hash helpers); one codegen ISA (`target_isa()`, SSE4.1 baseline) for JIT and exes; a fifth contract decided on the existing events walk (`What::Effect`, a summary chain) plus a snapshot check recorded at lowering; a built-in module `random` written in Kestrel; a built-in `checksum` lowered to generated per-type helpers over a new `ExprKind::Hash`; storage reuse lowered to generated `__kestrel_assign_N` helpers over new `ExprKind::Cap`, `Stmt::CopyItems` and `MapOp::CopyInto`.

**Tech Stack:** Rust (Cargo workspace `kestrel/`), Cranelift 0.137, `libm` 0.2.16; Kestrel tests in `kestrel/tests/programs`, `kestrel/tests/errors`, `kestrel/cli/tests`, `kestrel/kir/tests`, `kestrel/codegen/tests`.

**Spec:** `docs/superpowers/specs/2026-10-27-determinism-design.md` (read it whole; decisions 1-12 are numbered there). Background: `docs/superpowers/plans/2026-10-27-determinism-proposal.md`.

## Global Constraints

- **The PC may be in use for gaming. Build nothing, run no test and no benchmark until the controller says the PC is free.** Reading and writing code is fine at any time. When building: light mode always (`CARGO_BUILD_JOBS=2`, `--test-threads=2`, focused test targets, the idle-priority watcher from the handoff's "Environment and gotchas"); ask the controller before the full suite.
- Never a visible window: window programs run with `KESTREL_HEADLESS=<frames>` (and `KESTREL_HEADLESS_QUIET=1`) or `KESTREL_HIDDEN=1`. No GPU or window smoke tests.
- Never call a bare `link.exe` (Git's coreutils shadows the MSVC linker; use a short `CARGO_TARGET_DIR` such as `C:/kt/...` on LNK1104).
- Source files under about 400 lines; split by responsibility. Messages and comments are plain sentences that say what to do.
- No new syntax. New names (all the user's, accepted): `@prove(deterministic)`, the module `random` with `Rng` and `next`, `randf`, `randf_range`, `randi_range`, the built-in `checksum`.
- Exact texts (tests pin them):
  - build line: `built <exe path as given> for x86-64 CPUs with SSE4.1 (every one since about 2011)`;
  - no SSE4.1: `this program needs a CPU with SSE4.1 (every x86-64 CPU since about 2011); this one doesn't have it`;
  - unknown contract: ``unknown contract `{name}`; `@prove` takes `no_alloc`, `no_checks`, `no_panic`, `inline` and `deterministic` ``;
  - contract messages and help lines: spec decisions 5 and 6, verbatim;
  - checksum refusals: spec decision 8, verbatim.
- MXCSR value `0x1F80`. Checksum constants: start `0x243F6A8885A308D3`, multiplier `0x9E3779B97F4A7C15`, SplitMix64 finaliser (`0xBF58476D1CE4E5B9`, `0x94D049BB133111EB`, shifts 30, 27, 31).
- Never stage `kestrel/check/src/body/enums.rs`, `kestrel/test.kes`, `voxels.exe`; never edit `kestrel/bench/voxels3d/` sources. Stage by explicit path. Commits on `main`, plain-sentence messages ending with the line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; the controller pushes.
- Docs gotcha: a backslash followed by the letter v typed into a tool call becomes a vertical tab. Write Windows paths with forward slashes, and search changed docs for control characters before committing.

## Review Focus

1. **Overlap in storage reuse:** `a = a`, `a = a.child`, `n.kids[0] = n`, `ring[i] = ring[j]` with `i == j` and `i != j`, `for s in g.history { g = s }`, same-root paths that overlap (`p.game = p.game.sub`, `p.ring[k] = p.ring[j]`) against field-disjoint ones that reuse (`p.ring[k] = p.game`), a target index with a call in it, and the assign helper inlined into a loop over the same list. Every one must fall back to today's copy or be provably disjoint by `PathKey::overlaps` (Tasks 8, 9).
2. **Stale memory in storage reuse:** a list of lists that shrinks, then grows back (slots past the old length must be copied fresh, never recursed into), arrays, a string literal as the target, a map assigned while it is looped over, a target inside an `Arena` field (`in h.mem { h.snap = s }`) under `KESTREL_POISON=1`, and a `par for` body (Tasks 8, 9).
3. **A failing `par for`:** failures on the caller thread and on pool threads at once, a nested `par for`, `KESTREL_THREADS` 1, 2 and 16, and the Godot failure hook (Task 3).
4. **`@prove(deterministic)` chains:** `input.axis` (Kestrel code calling a native), a non-`pure` helper that only prints (holds), `randf` two calls deep, `pure` and non-`pure` function values, an interface method that is `pure`, promised or neither, and the snapshot check refusing exactly what `checksum` refuses (Tasks 4, 5, 7).
5. **Math bits:** the `float` path (widen, `libm`, narrow), subnormals, NaN, infinities, `1e22` argument reduction, `fmod` with zero and infinite operands, and agreement of the JIT and both exes on all of them (Tasks 1, 10).

## Commands (from `kestrel/`, bash, once the PC is free)

```bash
export CARGO_BUILD_JOBS=2
cargo test -p kestrel-runtime <filter> -- --test-threads=2
cargo test -p kestrel-kir --test <file> -- --test-threads=2
cargo test -p kestrel-codegen --test <file> -- --test-threads=2
KESTREL_TEST_ONLY=<prefix,prefix> cargo test -p kestrel-cli --test programs -- --test-threads=2
cargo test -p kestrel-cli --test errors -- --test-threads=2
cargo test -p kestrel-cli --test <file> -- --test-threads=2
cargo clippy -p <crate> --all-targets -- -D warnings
```

---

### Task 1: Kestrel's own math

**Model: Opus; review: yes (the bit tests).**

**Files:**
- Modify: `kestrel/runtime/Cargo.toml` (add `libm = { version = "=0.2.16", default-features = false }`), `kestrel/runtime/src/math.rs` (the ten `kestrel_rt_*` math functions call `libm::sin` ... `libm::pow`; the module comment says why), `kestrel/runtime/src/panic.rs` (`kestrel_rt_fmod_f32` calls `libm::fmodf`, `kestrel_rt_fmod_f64` calls `libm::fmod`)
- Create: `kestrel/runtime/src/math/bits.rs` (test module, `#[cfg(test)] mod bits;` in `math.rs`), `kestrel/runtime/src/math/speed.rs` (`#[cfg(test)] mod speed;`, ignored), `kestrel/tests/programs/det_math.kes`
- Docs: `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md`, "Built-in functions", Math: one sentence that the transcendental functions are Kestrel's own (a port of musl's libm) and give the same bits on every x86-64 CPU; and the float rule of spec decision 1.

**Interfaces:**
- Produces: the same `extern "C"` signatures as today (`kestrel_rt_sin(a: f64) -> f64`, ..., `kestrel_rt_atan2(y: f64, x: f64) -> f64`, `kestrel_rt_pow(a: f64, b: f64) -> f64`, `kestrel_rt_fmod_f32(a: f32, b: f32) -> f32`, `kestrel_rt_fmod_f64`). Codegen (`codegen/src/rt.rs` `MATH`, `codegen/src/func/math.rs`) is unchanged.

- [ ] **Step 1: Write the tests.** In `math/bits.rs`, a table `const TABLE: &[(&str, &[f64], u64)]` (function name, inputs, expected result bits) over exactly these inputs:
  - `sin`, `cos`, `tan`: `0.0, -0.0, 1e-300, 5e-324, 0.5, 1.0, 2.0, 3.0, -3.0, 0.7853981633974483, 100.0, 1e6, 1e22, 1e300, inf, -inf, NaN`
  - `asin`, `acos`: `-1.0, -0.5, -0.0, 0.0, 0.3, 0.5, 0.99999, 1.0, 1.5, NaN`
  - `atan`: `0.0, -0.0, 0.5, 1.0, -1.0, 1e10, -1e300, inf, NaN`
  - `atan2(y, x)`: `(0, 1), (0, -1), (-0.0, -1), (1, 0), (-1, 0), (1, 1), (-1, -1), (0, 0), (-0.0, -0.0), (inf, inf), (1, inf), (NaN, 1)`
  - `exp`: `0.0, 1.0, -1.0, 0.5, 10.0, 709.78, 710.0, -745.1, -746.0, inf, -inf, NaN`
  - `log`: `1.0, 2.0, 0.5, 10.0, 1e-310, 1e300, 0.0, -0.0, -1.0, inf, NaN`
  - `pow(a, b)`: `(2, 0.5), (2, 10), (10, -3), (-8, 1.0/3.0), (-8, 3), (0, 0), (NaN, 0), (1, NaN), (1.0000001, 1e7), (2, 1024), (2, -1075), (0.5, -1074)`
  - `fmod(a, b)`: `(7.5, 2), (-7.5, 2), (1e300, 3), (5e-324, 1), (1, 0), (inf, 1), (1, inf), (-0.0, 1)`
  Tests:
  - `every_function_gives_its_pinned_bits`: for each row, `kestrel_rt_<f>(inputs).to_bits() == expected`; a row whose expected value is a NaN asserts `is_nan()` only (payloads aren't part of the promise).
  - `pinned_values_are_within_one_ulp_of_the_platform`: for each finite expected value, the distance in ulps to Rust's `f64` method (`x.sin()`, `y.atan2(x)`, `a.powf(b)`, `a % b`) is at most 1; for infinite, zero and NaN results the class and sign are equal. (This checks the table, not `libm`: today's functions are the Microsoft C runtime's.)
  - `fmod_f32_matches_f64_for_exact_cases`: `kestrel_rt_fmod_f32(7.5, 2.0) == 1.5`, `(-7.5, 2.0) == -1.5`, `(1.0, 0.0)` is NaN.
  - `print_table` (`#[ignore]`): prints the table's rows with today's results as Rust source, so the expected bits are generated once (Step 3) and pasted.
  - In `math/speed.rs`, `own_math_against_the_platform` (`#[ignore]`, run only in Task 12): for each function, ns per call over 10,000,000 inputs spread over a realistic range, for `libm` and for Rust's `f64` method, in `--release`; prints one line per function.
  - `tests/programs/det_math.kes`: prints every row of `TABLE` (each function through Kestrel's own `sin`, `fmod` through `%` on floats, and so on), as `f64`, plus `sin`, `cos`, `atan2`, `exp`, `log`, `pow` of six inputs as `float`; its `// expect:` lines are the pinned values as Rust prints them (`{}` is the shortest round-trip form, so text equality is bit equality). This pins the JIT's runtime copy of `libm` (built inside the compiler, where Cargo turns `libm`'s `arch` feature back on through `cranelift-codegen`; only `sqrt` and `fma` change, both unused or exact) against the exes' copy (without it), row by row; Task 10 runs it in all four builds.
- [ ] **Step 2 (PC free):** `cargo test -p kestrel-runtime math -- --test-threads=2`. Expected: `every_function_gives_its_pinned_bits` FAILS (no table values yet).
- [ ] **Step 3: Implement.** Add the dependency (it is already in `Cargo.lock` and the registry: `cargo build` must not download; if it tries to, stop and report). Switch the functions. Run `cargo test -p kestrel-runtime print_table -- --ignored --nocapture`, paste the rows into `TABLE`, run the two table tests: both PASS. If a pinned value is more than 1 ulp from the platform's, stop and report it with the input (that is a finding, not something to paste over).
- [ ] **Step 4:** `cargo test -p kestrel-runtime -- --test-threads=2`, then `KESTREL_TEST_ONLY=det_math,random,math cargo test -p kestrel-cli --test programs -- --test-threads=2` (JIT, exe and release pass). Expected: PASS. `cargo clippy -p kestrel-runtime --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `kestrel/runtime/Cargo.toml`, `kestrel/Cargo.lock` (only if it changed), `kestrel/runtime/src/math.rs`, `kestrel/runtime/src/math/bits.rs`, `kestrel/runtime/src/math/speed.rs`, `kestrel/runtime/src/panic.rs`, `kestrel/tests/programs/det_math.kes`, the syntax spec: "Compute sin, cos and the other transcendental functions with Kestrel's own math (libm, a port of musl) instead of the C runtime, pin their bits for a table of inputs, and do the float remainder the same way".

### Task 2: The float mode and one baseline ISA

**Model: Sonnet; review: no.**

**Files:**
- Create: `kestrel/runtime/src/fpu.rs`
- Modify: `kestrel/runtime/src/lib.rs` (`pub mod fpu;`), `kestrel/runtime/src/start.rs` (`run_entry` pins first and marks its thread as started; `kestrel_rt_start` checks SSE4.1 first), `kestrel/runtime/src/par.rs` (`participate` pins first on a pool thread, and on the caller thread only when that thread is marked started, so a host such as the Godot add-on keeps its own mode), `kestrel/codegen/src/func/call.rs` (after every call to a `native` function, a call to the runtime's `kestrel_rt_fpu_pin`; only built-in modules declare natives, and today all of them are the window library's), `kestrel/codegen/src/rt.rs` (declare `kestrel_rt_fpu_pin`; JIT symbol), `kestrel/codegen/src/isa.rs` (`host_isa` becomes `target_isa`), `kestrel/codegen/src/jit.rs`, `kestrel/codegen/src/object.rs`, `kestrel/codegen/src/dump.rs`, `kestrel/codegen/src/rt.rs` (its test), `kestrel/cli/src/main.rs` (the build line; the JIT's SSE4.1 check before compiling)
- Test: `kestrel/runtime/src/fpu.rs` (unit tests), `kestrel/runtime/src/par.rs` tests (or `par/tests.rs` if the file passes 400 lines), `kestrel/codegen/src/isa.rs` tests, `kestrel/codegen/tests/dump.rs`, `kestrel/cli/tests/build.rs`

**Interfaces:**
- Produces: `pub const PINNED: u32 = 0x1F80;` `pub fn pin()` (sets MXCSR to `PINNED` on this thread); `pub fn read() -> u32`; `pub fn has_sse41() -> bool`; `pub const NO_SSE41: &str` (the exact sentence in Global Constraints); `#[no_mangle] pub extern "C" fn kestrel_rt_fpu_pin()` (calls `pin`); `pub(crate) fn mark_started()` and `pub(crate) fn started() -> bool` (a thread-local flag set by `run_entry`) in `fpu.rs`. `pub(crate) fn target_isa() -> Result<OwnedTargetIsa, String>` in `isa.rs` (flags as today plus `cranelift_native::builder_with_options(false)` and the `sse41` preset enabled; on a non-x86-64 host it falls back to the host builder); `pub const TARGET_CPUS: &str = "x86-64 CPUs with SSE4.1 (every one since about 2011)"` exported from `kestrel-codegen` for the CLI's line.

- [ ] **Step 1: Write the failing tests.**
  - `fpu::pin_restores_round_to_nearest_without_flushing`: set MXCSR to `0x7FC0` (round toward zero, flush-to-zero, denormals-are-zero) through the same assembly, call `pin()`, assert `read() == 0x1F80` and that `black_box(f64::MIN_POSITIVE) * black_box(0.5)` is not 0.
  - `fpu::this_machine_has_sse41`: `has_sse41()` is true (every machine this is built on has it).
  - `start::start_pins_the_float_mode_for_main`: an entry that stores `fpu::read()` into a static; after `kestrel_rt_start(entry)` the static is `0x1F80`.
  - `par::pool_threads_run_with_the_float_mode_pinned`: job 1 (100 iterations, 4 threads) sets every participating thread's MXCSR to `0x7FC0`; job 2, run from a test thread marked started, records `fpu::read()` per index; all 100 are `0x1F80`.
  - `par::a_host_callers_float_mode_is_left_alone`: on a fresh test thread not marked started, set `0x7FC0`, run a job whose body records `fpu::read()` on the caller only (pool threads skip); the caller's record is `0x7FC0`, and so is `read()` after the job; restore `0x1F80` at the end.
  - `dump.rs::every_native_call_is_followed_by_a_pin`: dump of a function calling `draw.clear` and `window.frame` (compiled with the window natives as imports) shows a call to `kestrel_rt_fpu_pin` after each of them, and none after a call to a Kestrel function.
  - `isa::the_target_isa_is_the_sse41_baseline`: `has_sse3`, `has_ssse3`, `has_sse41` true; `has_sse42`, `has_popcnt`, `has_avx`, `has_avx2`, `has_fma`, `has_bmi1`, `has_bmi2`, `has_lzcnt` false (read through `isa_flags()` as `check_simd` does).
  - `dump.rs::a_float_multiply_add_is_never_fused`: dump of `pure f(a: f64, b: f64, c: f64) -> f64 { a * b + c }` (and the `float` version) contains `fmul` and `fadd` and contains neither `fma` nor `vfmadd`.
  - `build.rs::build_says_which_cpus_the_exe_runs_on`: `kestrel build prog.kes -o <dir>/prog.exe` prints exactly `built <dir>/prog.exe for x86-64 CPUs with SSE4.1 (every one since about 2011)` plus a newline on stdout (the path as given to `-o`, displayed), exit 0.
- [ ] **Step 2 (PC free):** run them: FAIL (no `fpu`, host ISA, no line).
- [ ] **Step 3: Implement.** `fpu.rs` with `core::arch::asm!` `ldmxcsr`/`stmxcsr` on x86-64 (no-ops elsewhere). `kestrel_rt_start` checks `has_sse41()` first and, without it, prints `kestrel: ` + `NO_SSE41` and exits 101. In the CLI, `run` checks the same before compiling and exits 3 with the sentence. Replace every `host_isa` use; keep `check_simd` and `emit_object_for` as they are. Print the line after a successful link.
- [ ] **Step 4:** `cargo test -p kestrel-runtime -- --test-threads=2`, `cargo test -p kestrel-codegen -- --test-threads=2`, `cargo test -p kestrel-cli --test build -- --test-threads=2`, `KESTREL_TEST_ONLY=par_,simd,vec4,ivec4 cargo test -p kestrel-cli --test programs -- --test-threads=2`. Then, only if the controller agrees (it builds raylib): a headless program check through the CLI: `KESTREL_TEST_ONLY=window,draw,input cargo test -p kestrel-cli --test programs -- --test-threads=2` (window programs run headless there; outputs unchanged). Expected: PASS. Clippy on the touched crates.
- [ ] **Step 5: Commit** the files above: "Pin the float control register at start, in par for threads and after every call into the window library, and compile the JIT and exes for one baseline CPU (SSE4.1), which `kestrel build` now names".

### Task 3: A failing `par for` reports its lowest failing index

**Model: Sonnet; review: no.**

**Files:**
- Modify: `kestrel/runtime/src/par.rs` (move its tests to `kestrel/runtime/src/par/tests.rs` if the file passes 400 lines), `kestrel/runtime/src/panic.rs` (`fail` reports what `fail_in_job` returns)
- Create: `kestrel/tests/programs/par_for_lowest_failure.kes`
- Test: `kestrel/runtime/src/par.rs` tests, `kestrel/cli/tests/par.rs`

**Interfaces:**
- Produces: `CURRENT` holds `(Arc<Job>, bool, Cell<i64>)` (the chunk start being run); `Job::failure: Mutex<Option<(i64, String)>>` keeps the smallest chunk start; `pub(crate) fn fail_in_job(msg: &str) -> Option<String>`: `None` outside a job; on a pool thread it records and parks forever (as today); on the caller it records, stops new chunks, waits until every other participant has finished, and returns the winning message. `fail` prints that message.

- [ ] **Step 1: Write the failing tests.**
  - `par::the_lowest_failing_chunk_wins` (runtime): count 100, 4 threads; the body fails (through `fail_in_job`) at index 5 after sleeping 50 ms and at index 90 at once, on pool threads; a caller-thread failure stores the returned message in a static and returns. Over 20 runs at 2, 4 and 16 threads the reported message is always `"iteration 5"`.
  - `par::a_later_failure_does_not_stop_an_earlier_chunk`: index 0's chunk sleeps 50 ms then fails; index 60 fails at once; the result is `"iteration 0"`.
  - `tests/programs/par_for_lowest_failure.kes`: `// env: KESTREL_THREADS=8`, `// expect-exit: 101`, `// expect-stderr: index 1003 out of bounds for length 1000 on line <N> of par_for_lowest_failure.kes`. A `par for i, x in xs` over 1,000 items: when `i == 3` it spins (`for k from 0 to 30_000_000 { s +%= k }`) and then reads `xs[i + 1000]`; when `i == 900` it reads `xs[i + 1000]` at once.
  - `cli/tests/par.rs::the_lowest_failing_iteration_reports_at_any_thread_count`: runs that program with `kestrel run` 5 times each at `KESTREL_THREADS` 1, 2, 8 and 16; every standard error is the same single line, the one above.
- [ ] **Step 2 (PC free):** run them. Expected: the runtime tests and the CLI test FAIL (the message depends on timing).
- [ ] **Step 3: Implement** per spec decision 10. A chunk already taken runs to its end; no chunk is taken after a failure; the caller's wait uses the job's `state`/`done` (a failed pool thread already calls `finish`). With the failure hook (the Godot add-on) the caller still waits, then calls the hook with the winner.
- [ ] **Step 4:** `cargo test -p kestrel-runtime par -- --test-threads=2`, `cargo test -p kestrel-cli --test par -- --test-threads=2`, `KESTREL_TEST_ONLY=par_ cargo test -p kestrel-cli --test programs -- --test-threads=2`. Expected: PASS (including today's `par_for_panic.kes`).
- [ ] **Step 5: Commit**: "Make a failing par for report its lowest failing iteration, whatever the thread count, by keeping the failure with the smallest chunk start and letting a failing caller wait for the other threads".

### Task 4: `@prove(deterministic)`: what breaks it

**Model: Opus; review: yes.**

**Files:**
- Modify: `kestrel/syntax/src/ast.rs` (`Contract::Deterministic`, `name` "deterministic", `from_name`), `kestrel/syntax/src/parser/annot.rs` (the unknown-contract message), `kestrel/kir/src/contract.rs` (`ContractSet::DETERMINISTIC = ContractSet(16)`, `of`), `kestrel/kir/src/ir.rs` (`KSig` gains `pub pure: bool`), `kestrel/kir/src/lower/iface.rs` (line 159), `kestrel/kir/src/lower/subst.rs` (`known_ksig`, `ksig`: set `pure` from the signature), every other `KSig { .. }` construction the compiler finds, `kestrel/kir/src/range/events.rs` (`What::Effect(Effect, Reason)`, `pub enum Effect { Clock, Keyboard, Mouse, Window, SharedRandom, ImpureFnValue, ImpureMethod }`; `RandFloat`, `RandInt`, `CallIndirect` with `!sig.pure`, `CallVirtual` whose method isn't `pure` and whose `keeps` lacks `DETERMINISTIC`), `kestrel/kir/src/range/events/stmts.rs` (`Stmt::RandSeed`, `Stmt::CallIndirect`, `Stmt::CallVirtual`), `kestrel/kir/src/range/events/reasons.rs` (`call`: a native pushes `Effect` as well as today's `Unknown::Native`; a small table from native name to `Effect`), `kestrel/kir/src/range/summary.rs` (`Summary::effects: Option<Chain>`), `kestrel/kir/src/range/contracts.rs` (bit `DETERMINISTIC`; `event_failures` for `What::Effect`; `chains` follows `effects`), `kestrel/kir/src/range/contracts/words.rs` (new `Help` variants with the decided help lines), and the report if it lists contracts by name (grep `no_panic` in `kir/src/range/render.rs` and `report.rs`)
- Create: `kestrel/kir/tests/contracts_deterministic.rs`, `kestrel/tests/errors/prove_deterministic_clock.kes`, `prove_deterministic_random.kes`, `prove_deterministic_input.kes`, `prove_deterministic_fn_value.kes`, `prove_deterministic_iface.kes`, `prove_deterministic_loop.kes`, `kestrel/tests/programs/prove_deterministic_holds.kes`
- Docs: the syntax spec's `@prove` bullet (a fifth contract, what breaks it, `print` allowed, every `pure` function holds it).

**Interfaces:**
- Produces: `Contract::Deterministic`; `ContractSet::DETERMINISTIC`; `KSig::pure`; `What::Effect(Effect, Reason)`; `Summary::effects`. Consumed by Task 5 (`ContractSite` gains its snapshot fault there) and Task 11 (the demo's `step`).

- [ ] **Step 1: Write the failing tests.** Each error file ends with `// error <line>: <message>` lines, the messages of spec decision 5 verbatim with this file's lines:
  - `prove_deterministic_clock.kes`: `use window`, `@prove(deterministic) step(var g: Game) { g.x += window.delta() }` → "`step` can't be proven `deterministic`: it calls `window.delta` (line 5), which reads the clock". A second function calling `window.ticks_usec` gives the same words.
  - `prove_deterministic_random.kes`: `spawn_enemy` calls `randi_range(1, 6)`; `@prove(deterministic) step(...)` calls `spawn_enemy` → "... it calls `spawn_enemy` (line N), which calls `randi_range` (line M), which uses the program's shared random generator". A second site calls `rand_seed(1)` directly → "... it calls `rand_seed` (line K), which uses the program's shared random generator".
  - `prove_deterministic_input.kes`: `use input`; `update` calls `input.held(Key.W)` → "... it calls `input.held` (line 5), which reads the keyboard"; `steer` calls `input.axis(...)` → "... it calls `input.axis` (line N), which calls `input.held` (...), which reads the keyboard" (the builder pins how today's chains write a line in a built-in module); `aim` calls `input.mouse()` → "... which reads the mouse".
  - `prove_deterministic_fn_value.kes`: `@prove(deterministic) hit(var f: Fighter, on_hit: (Fighter) -> int)` calling `on_hit(f)` → "... it calls the function value `on_hit` (line 2), which isn't `pure`"; the same with `pure (Fighter) -> int` gives no error.
  - `prove_deterministic_iface.kes`: three interfaces: a `pure` method (no error), a method with `@prove(deterministic)` (no error), a plain method → "... it calls a method of the interface value `s` (line N), which isn't `pure` and doesn't promise `deterministic`".
  - `prove_deterministic_loop.kes`: `@prove(deterministic)` on a `for` loop that calls `randf()` → "the loop at line 4 can't be proven `deterministic`: it calls `randf` (line 5), which uses the program's shared random generator".
  - `kir/tests/contracts_deterministic.rs` (build modules as `contracts_calls.rs` does):
    - `effects_follow_calls_into_a_chain`: `step` → `spawn_enemy` → `randi_range`; `summaries[step].effects` has one step (`spawn_enemy`) and its reason's `via` is "calls `randi_range`".
    - `print_and_math_keep_a_function_deterministic`: a non-`pure` function that prints and calls `sin`, `atan2`, `pow`, `sqrt`, sorts a list and runs a `par for`: `check` returns no error for its `@prove(deterministic)`.
    - `a_pure_function_always_holds`: several `pure` functions (one calling a `pure` function value, one calling a `pure` interface method, one recursive) each with `@prove(deterministic)`: no error.
    - `every_help_line_says_what_to_do`: the `help` of the clock, random, keyboard and function-value errors equals spec decision 5's help lines exactly.
    - `the_other_contracts_ignore_effects`: a function calling `randf()` with `@prove(no_alloc, no_checks, no_panic)` gives the same errors as before this task (none).
  - `prove_deterministic_holds.kes` (program; JIT, exe, release): a `pure` step and a non-`pure` one that prints, both `@prove(deterministic)`, with `sin`, a `pure` function value, a `pure` interface method and a `par for`; prints fixed lines (`// expect:`).
  - Update `tests/errors/annot_*.kes` or wherever the unknown-contract message is pinned to the new text.
- [ ] **Step 2 (PC free):** `cargo test -p kestrel-kir --test contracts_deterministic -- --test-threads=2`, `cargo test -p kestrel-cli --test errors -- --test-threads=2`. Expected: FAIL (`deterministic` is an unknown contract).
- [ ] **Step 3: Implement** spec decision 5. Native words: `window.delta`, `window.time`, `window.ticks_usec`, `window.fps` → Clock; `input.held`, `input.pressed`, `input.released` → Keyboard; `input.mouse`, `input.wheel`, `input.mouse_delta` → Mouse; any other native → Window ("talks to the window, which a deterministic function can't depend on"). The other four contracts must not change: `Effect` events are ignored by `no_alloc`, `no_checks`, `no_panic` and `inline` (natives keep their `Unknown::Native` event for those).
- [ ] **Step 4:** the two test commands again, plus `cargo test -p kestrel-kir -- --test-threads=2`, `cargo test -p kestrel-cli --test contracts -- --test-threads=2`, `KESTREL_TEST_ONLY=prove cargo test -p kestrel-cli --test programs -- --test-threads=2`, `cargo test -p kestrel-cli --test voxel_engine -- --test-threads=2` (contract probes unchanged). Expected: PASS. Clippy on `kestrel-syntax`, `kestrel-kir`, `kestrel-codegen`.
- [ ] **Step 5: Commit**: "Add @prove(deterministic): a native, the shared random generator, a function value that isn't pure or an unpromised interface method breaks it, with the chain and a help line that says what to do".

### Task 5: `@prove(deterministic)`: snapshot-able `var` parameters

**Model: Sonnet; review: yes.**

**Files:**
- Create: `kestrel/check/src/snapshot.rs` (the one rule shared with `checksum`: `pub fn snapshot_fault(t: &Tables, ty: Ty, root: &str) -> Option<SnapshotFault>`, walking the type depth first, fields in order, items as `[i]`, payloads and variants by field name, and returning the first `Arena`, GPU handle, interface value or function value, `pure` or not; a type that contains itself is walked once), `kestrel/tests/errors/prove_deterministic_snapshot.kes`
- Modify: `kestrel/check/src/lib.rs` (export it), `kestrel/kir/src/contract.rs` (`ContractSite` gains `pub snapshot: Option<SnapshotFault>`; `SnapshotFault { pub param: String, pub path: String, pub kind: FaultKind }` and `pub enum FaultKind { Arena, Gpu, Iface, Fn }` live in the check crate and are re-used here), `kestrel/kir/src/lower/prove.rs` (for a function site with `deterministic`: `snapshot_fault` over each `var` parameter and `var self`), `kestrel/kir/src/range/contracts.rs` (report the fault as the `deterministic` error before the effects), `kestrel/kir/src/range/contracts/words.rs`
- Test: `kestrel/kir/tests/contracts_deterministic.rs`, unit tests in `check/src/snapshot.rs`

**Interfaces:**
- Consumes: Task 4's `Contract::Deterministic`. Produces: `ContractSite::snapshot`; `kestrel_check::snapshot_fault`, which Task 7's `checksum` refusal uses too (spec decisions 6 and 8 are one rule).

- [ ] **Step 1: Write the failing tests.**
  - `prove_deterministic_snapshot.kes` (every message ends "which a snapshot can't copy and checksum"):
    - `struct World { mem: Arena, n: int }` with `@prove(deterministic) step(var w: World)` → "`step` can't be proven `deterministic`: its `var` parameter `w` holds an `Arena` (`w.mem`), which a snapshot can't copy and checksum";
    - `struct Fighter { on_hit: (int) -> int }` with `hit(var f: Fighter)` → "... its `var` parameter `f` holds a function value (`f.on_hit`), ...";
    - `struct Safe { on_hit: pure (int) -> int, xs: list[int] }` with `ok(var s: Safe)` → **refused** the same way, "... `s` holds a function value (`s.on_hit`), ..." (one rule with `checksum`: a `pure` function value is refused too);
    - `struct Stage { scene: Scene }` (an interface) with `play(var s: Stage)` → "... holds an interface value (`s.scene`), ...";
    - a list of worlds, `many(var ws: list[World])` → "... `ws` holds an `Arena` (`ws[i].mem`), ...";
    - a plain struct holding an `Arena` two levels down (`struct Game { level: Level }`, `struct Level { mem: Arena }`) with `tick(var g: Game)` → "... `g` holds an `Arena` (`g.level.mem`), ...";
    - a read-only (not `var`) `World` parameter: no snapshot error.
  - `check/src/snapshot.rs` unit tests: `the_first_fault_is_found_depth_first` (fields in order, the earlier field wins), `a_type_holding_itself_terminates` (`struct Node { kids: list[Node], f: () -> int }` gives the path `n.f` and returns), `plain_data_has_no_fault` (numbers, vectors, strings, lists, maps, optionals, enums of those).
  - `contracts_deterministic.rs::the_snapshot_fault_has_help`: the help for an `Arena` fault is "keep arenas, textures, meshes, interface values and function values outside the state a deterministic function changes".
- [ ] **Step 2 (PC free):** errors test: FAIL.
- [ ] **Step 3: Implement** spec decision 6.
- [ ] **Step 4:** `cargo test -p kestrel-check snapshot -- --test-threads=2`, `cargo test -p kestrel-cli --test errors -- --test-threads=2`, `cargo test -p kestrel-kir --test contracts_deterministic -- --test-threads=2`, `cargo test -p kestrel-kir --test contracts_sites -- --test-threads=2`. Expected: PASS.
- [ ] **Step 5: Commit**: "Make @prove(deterministic) also check that the state a function changes can be copied and checksummed: no arena, GPU object, interface value or function value".

### Task 6: The `random` module

**Model: Sonnet; review: no.**

**Files:**
- Create: `kestrel/driver/std/random.kes`, `kestrel/tests/programs/rng_sequence.kes`, `kestrel/tests/programs/rng_ranges.kes`, `kestrel/tests/programs/rng_contract.kes`, `kestrel/cli/tests/rng_reference.rs`
- Modify: `kestrel/driver/src/builtin.rs` (`MODULES` gains `("random", include_str!("../std/random.kes"))`; `pub fn needs_window(name: &str) -> bool`, true for `window`, `draw`, `draw3d`, `input`; its tests list `random`), `kestrel/driver/src/lib.rs` (re-export `needs_window` if the CLI needs it), `kestrel/cli/src/main.rs` (`fn lib_for(builtins: &[&str]) -> Lib`: the window library only when one of them `needs_window`), `kestrel/driver/tests/builtin.rs`
- Docs: the syntax spec's "Built-in modules" section: `random`, `Rng`, its methods, GDScript's `RandomNumberGenerator` as the comparison.

**Interfaces:**
- Produces (Kestrel): `pub struct Rng { seed: i64 }`; `pure next(var self) -> i64`; `pure randf(var self) -> float`; `pure randf_range(var self, lo: float, hi: float) -> float`; `pure randi_range(var self, lo: int, hi: int) -> int`. Consumed by Tasks 10 and 11.

- [ ] **Step 1: Write the failing tests.**
  - `cli/tests/rng_reference.rs::next_matches_splitmix64`: a program prints `r.next()` 8 times for `Rng(seed: 42)`, then 8 times for `Rng(seed: -1)` and for `Rng(seed: 0)`; the test computes SplitMix64 in Rust (`state += 0x9E3779B97F4A7C15`, then the finaliser) and asserts every printed `i64` equals the reference's bits as `i64`.
  - `rng_reference.rs::randf_matches_the_reference`: 8 `randf()` values for seed 7 equal the reference `((next >> 11) as f64 * 2^-53) as f32` with values that round to 1 replaced by the largest `f32` below 1.
  - `rng_sequence.kes`: the same seed gives the same 100 draws of each method (`true`); `let saved = r` then 10 draws from each give equal values (`true`); two different seeds differ (`true`).
  - `rng_ranges.kes`: `randi_range(1, 6)` stays in 1..6 and hits each face in 1,000 draws; `randi_range(6, 1)` stays in 1..6; `randi_range(-2147483648, 2147483647)` runs 1,000 times without an overflow in a checked build; `randf()` in `[0, 1)` over 10,000 draws; `randf_range(2, 3)` in `[2, 3]`.
  - `rng_contract.kes`: `@prove(deterministic, no_alloc, no_panic) pure roll(var r: Rng) -> int { r.randi_range(1, 6) }` builds and runs (needs Task 4).
  - `driver/tests/builtin.rs::random_is_a_built_in_module_that_needs_no_window`: `use random` compiles with `builtins == ["random"]`, and `needs_window("random")` is false while `needs_window("draw")` is true.
  - `cli/src/main.rs` test `only_window_modules_link_the_window_library`: `lib_for(&["random"])` is `Lib::Runtime`, `lib_for(&["random", "draw"])` is `Lib::Window`.
- [ ] **Step 2 (PC free):** run them: FAIL.
- [ ] **Step 3: Implement** spec decision 7. Builder check: if a method named `randf` (a global built-in name) is refused or resolves to the global, stop and report: the names are the user's.
- [ ] **Step 4:** `cargo test -p kestrel-driver -- --test-threads=2`, `cargo test -p kestrel-cli --test rng_reference -- --test-threads=2`, `KESTREL_TEST_ONLY=rng_,random cargo test -p kestrel-cli --test programs -- --test-threads=2`, the CLI's unit tests. Expected: PASS.
- [ ] **Step 5: Commit**: "Add the built-in module random with Rng, a seeded SplitMix64 generator written in Kestrel that lives in the game state, and link the window library only for the window modules".

### Task 7: `checksum`

**Model: Opus; review: no.**

**Files:**
- Modify: `kestrel/check/src/builtins.rs` (`Builtin::Checksum`, in `GLOBALS`, so the name is reserved), `kestrel/check/src/body/builtins.rs` (one argument of any type; the refusals of spec decision 8, found with Task 5's `snapshot_fault` (one rule: what a deterministic function's state may not hold is what `checksum` refuses); result `i64`), `kestrel/kir/src/ops.rs` (`pub enum HashOp { Mix, Finish }`), `kestrel/kir/src/ir_expr.rs` (`ExprKind::Hash(HashOp, Vec<Expr>)`, type `i64`), `kestrel/kir/src/lower/builtins.rs` (lowers `checksum(x)` to `Hash(Finish, [Call(checksum_fn(T), [H0, x])])`), `kestrel/kir/src/lower/helpers.rs` (field `checksum: HashMap<KTy, FuncIdx>`), every visitor and pass the compiler lists for the new `ExprKind` (`visit.rs`, `visit_mut.rs`, `rename.rs`, `inline/descend.rs`, `range/eval.rs` (any `i64`), `range/show.rs`, `range/events.rs` (nothing to list)), `kestrel/codegen/src/func/expr.rs` (dispatch), `kestrel/codegen/src/rt.rs` (`hash_bytes`)
- Create: `kestrel/kir/src/lower/checksum.rs` (`Helpers::checksum_fn(ty, t) -> FuncIdx`: `__kestrel_checksum_N(h: i64, x: T) -> i64`, the walk of spec decision 8, maps over live entries in order like `map_helpers.rs`'s `each_entry`), `kestrel/codegen/src/func/checksum.rs` (`Mix` on a bool, `u8`, `int`, `i64`, `float`, `f64` (NaN canonicalised), a vector lane, or a string (calls `kestrel_rt_hash_bytes`); `Finish`), `kestrel/runtime/src/checksum.rs` (`pub const START: u64`, `pub fn mix(h: u64, w: u64) -> u64`, `pub fn finish(h: u64) -> u64`, `#[no_mangle] pub unsafe extern "C" fn kestrel_rt_hash_bytes(h: i64, ptr: *const u8, len: i64) -> i64`), `kestrel/tests/programs/checksum_values.kes`, `kestrel/tests/errors/checksum_refused.kes`, `kestrel/cli/tests/checksum_reference.rs`
- Docs: the syntax spec's "Built-in functions": `checksum(x) -> i64`, what it reads, what it refuses, that it returns `i64` until Kestrel has `u64`.

**Interfaces:**
- Produces: the built-in `checksum`; `runtime::checksum::{START, mix, finish}` (the tests' reference uses them). Consumed by Tasks 9 to 11.

- [ ] **Step 1: Write the failing tests.**
  - `runtime/src/checksum.rs` tests: `mix_follows_the_definition` (for 5 pairs, `mix(h, w) == { let x = (h ^ w).wrapping_mul(0x9E3779B97F4A7C15); x ^ (x >> 32) }`); `bytes_hash_as_padded_words` (`hash_bytes(START, b"hello, world!")` equals `mix` over `13`, the little-endian word of `b"hello, w"` and of `b"orld!\0\0\0"`).
  - `cli/tests/checksum_reference.rs::checksum_matches_the_reference_fold`: a program prints `checksum` of `0`, `i64(0)`, `true`, `u8(200)`, `1.5`, `f64(-0.0)`, `vec3(1, 2, 3)`, `"hello, world!"`, `[1, 2, 3]`, `{"a": 1, "b": 2}` (after `m.remove("a")` and `m["c"] = 3`), `none` and `some(4)` as an `int?`, an enum `Shape.Circle(r: 2.5)` (variant 1), and a struct `P { a: int, b: f64, s: string, xs: list[int] }`; the test builds each word list by hand in Rust from spec decision 8 and asserts each printed value equals `finish(fold(START, words)) as i64`.
  - `checksum_values.kes` (JIT, exe, release; `// expect:` lines): `checksum(nan_a) == checksum(nan_b)` for two NaNs of different sign is `true`; `checksum(0.0) == checksum(-0.0)` is `false`; `checksum(1) == checksum(i64(1))` is `true`; equal structs built two ways give equal checksums (`true`); changing one list item changes it (`true`); `@prove(deterministic, no_alloc, no_panic) pure sum(g: Game) -> i64 { checksum(g) }` holds and runs.
  - `checksum_refused.kes`: a `struct World { mem: Arena }` → "`checksum` can't take a `World`, which holds an `Arena`; checksum the fields that hold the game's state instead"; a struct with a function field → "`checksum` can't take a `Fighter`, which holds a function value, whose address differs between builds; checksum the other fields instead"; a struct with an interface field → "`checksum` can't take a `Scene`, which holds an interface value; checksum the fields that hold the game's state instead"; `checksum()` and `checksum(1, 2)` → "`checksum` takes one value".
- [ ] **Step 2 (PC free):** run them: FAIL.
- [ ] **Step 3: Implement** spec decision 8. `checksum` makes no allocation and no failure event, so `no_alloc`, `no_panic` and `deterministic` hold through it. A generic: decide whether `checksum(x)` on a placeholder is checked at each copy or refused, and pin it with a test (`checksum_refused.kes` or a program).
- [ ] **Step 4:** `cargo test -p kestrel-runtime checksum -- --test-threads=2`, `cargo test -p kestrel-cli --test checksum_reference -- --test-threads=2`, `KESTREL_TEST_ONLY=checksum cargo test -p kestrel-cli --test programs -- --test-threads=2`, `cargo test -p kestrel-cli --test errors -- --test-threads=2`, `cargo test -p kestrel-kir -- --test-threads=2`, `cargo test -p kestrel-check -- --test-threads=2`. Expected: PASS. Clippy.
- [ ] **Step 5: Commit**: "Add checksum(x), a content hash of any value without owning types, function values or interface values, folded field by field in a fixed order with NaN made canonical".

### Task 8: Storage-reusing assignment: lists, arrays, strings, structs, enums

**Model: Opus; review: yes.**

**Files:**
- Modify: `kestrel/kir/src/ir_expr.rs` (`ExprKind::Cap(Place)`: a list's capacity, an array's length, as `i64`), `kestrel/kir/src/ir_stmt.rs` (`Stmt::CopyItems(Place, Place)`: with room already checked, copies the source's items' bytes into the target's buffer and sets its length), `kestrel/kir/src/lower/helpers.rs` (field `assign: HashMap<KTy, FuncIdx>`), `kestrel/kir/src/lower/stmt.rs` (the plain-`=` branch: when spec decision 9's conditions hold, emit the helper call instead of `owned`), `kestrel/kir/src/lower/collections.rs` (`may_move` and `origin` are reused; confirm the helper call's target counts as written in `visit::places`), the overlap test (in `assign_into.rs`: both places through `origin`, then the same rule as the checker's `PathKey::overlaps` in `check/src/body/regions.rs`, field steps compared and any index matching any index; reuse it if the KIR side can reach it, else mirror it with a comment naming it), and the index test (fall back when any index expression in the target place contains a call, a `Seq`, or anything else `visit` reports as a write), every visitor and pass the compiler lists for the two new nodes (`visit.rs`, `visit_mut.rs`, `rename.rs`, `inline/descend.rs`, `scalarize.rs`, `promote.rs`, `range/writes.rs` (`CopyItems` writes the target's length), `range/stmt.rs`, `range/eval.rs` (`Cap` is any `i64` from 0), `range/events/stmts.rs` (no event), `range/show.rs`), `kestrel/kir/src/range/report.rs` (`callee_name` for `__kestrel_assign_` helpers: "assigning a `Game`"), `kestrel/codegen/src/func/collections.rs` (`Cap`: load `CAP`, or `LEN` for an array; `CopyItems`: call the runtime), `kestrel/codegen/src/rt.rs`, `kestrel/runtime/src/list.rs` (`kestrel_rt_copy_items`)
- Create: `kestrel/kir/src/lower/assign_into.rs` (`Helpers::assign_fn(ty: KTy, t: &Tables) -> FuncIdx`; `Helpers::assign_stmts(dst: Place, src: Place, ty: KTy, t: &Tables) -> Vec<Stmt>` for the inline struct, optional and enum parts), `kestrel/kir/tests/assign_reuse.rs`, `kestrel/tests/programs/assign_reuse_lists.kes`, `assign_reuse_self.kes`, `assign_reuse_arena.kes`, `assign_reuse_par.kes`, `kestrel/tests/errors/assign_reuse_scoped_arena.kes`, `kestrel/cli/tests/snapshot_memory.rs`
- Docs: the syntax spec's "Memory" section: assigning over an existing value reuses its storage when it has room; no syntax; the copy is still a deep copy.

**Interfaces:**
- Produces: `#[no_mangle] pub unsafe extern "C" fn kestrel_rt_copy_items(dst: *mut List, src: *const u8, src_len: i64, elem_size: i64)` (`List` is arena.rs's header; an array uses only its first two fields; `ptr::copy` so overlap can't corrupt); `ExprKind::Cap`; `Stmt::CopyItems`; `__kestrel_assign_N(var dst: T, src: T)` helpers (`is_pure: true`, `mutates_params: true`, lines `(0, 0)`). Consumed by Task 9.

- [ ] **Step 1: Record today's outputs (PC free).** Write the four program tests (below) without `// expect:` lines; with the compiler at HEAD (before any change in this task), record each program's JIT output as its `// expect:` lines. These are the behaviour the change must keep.
- [ ] **Step 2: Write the failing tests.**
  - `assign_reuse_lists.kes`: a `list[int]` assigned from lists of 5, 2, 8, 0 and 5 items (printed after each); a `list[list[int]]` whose inner lists shrink and grow (3 then 1 then 4 inner lists, inner lengths 0 to 6); an array `[f64]` assigned from arrays of equal, shorter and longer lengths; `var s = "abc"` assigned from an equal string, then a different one, then a literal; a struct `State { xs: list[int], name: string, hp: int, pos: vec2, opt: int?, shape: Shape }` assigned from states that differ in each field, with the enum switching variant and the optional switching between `none` and a value. Each print shows the target equals the source and that the source is unchanged after the target is changed again.
  - `assign_reuse_self.kes`: `a = a`; `a = a.kids[0]` and `n.kids[0] = n` for `struct Node { v: int, kids: list[Node] }`; `ring[i] = ring[j]` with `i == j` and `i != j`; `for s in g.history { g = s }`; same-root overlapping fields, `p.game = p.game.sub` and `p.ring[k] = p.ring[j]` for `struct Pair { game: State, ring: list[State] }`; same-root disjoint fields that reuse, `p.ring[k] = p.game` then `p.game = p.ring[j]`; a target index with a call in it, `ring[next(var k)] = g` (where `next` changes and returns `k`) and `ring[pick(ring)] = g`; each followed by prints of the whole value (and of `k`). (If today's checker refuses one of these shapes, leave it out and say so in the task report: a refused shape can't overlap.)
  - `assign_reuse_arena.kes` (`// env: KESTREL_POISON=1`, `// env: KESTREL_ARENA_COUNT=1`, `// expect-stderr: arenas alive at exit: 0`): a source built `in a { }` (an `Arena` local), copied into an outer target, then `a` replaced (freed) and the target printed; `struct Holder { mem: Arena, snap: State }` with `in h.mem { h.snap = s }` over states of growing and shrinking sizes, then `h` replaced and the old one's arena freed.
  - `assign_reuse_par.kes` (`// env: KESTREL_THREADS=8`): a `par for` over 64 items where the body assigns into a body-local `State` (reuse) and an item's field from a template the body reads (also reuse: the body is lowered before it becomes the pool's function, so the template is not a closure capture; corrected in the final review's fix wave); prints a sum of every item's list lengths and a checksum-free fingerprint (sum of items).
  - `assign_reuse_scoped_arena.kes` (error): `var outer: list[int] = []` then `arena { let tmp = [1, 2, 3]; outer = tmp }` → today's message, "this memory is freed at the end of the `arena { }` block, but `outer` lives longer".
  - `kir/tests/assign_reuse.rs`:
    - `a_snapshot_assignment_calls_the_assign_helper`: `ring[k] = g` for a `State` with a list: the lowered `main` holds a `Stmt::Call` to a function whose name starts with `__kestrel_assign_`, with `Arg::Ref` target then source.
    - `overlapping_places_keep_the_old_copy`: each overlapping shape of `assign_reuse_self.kes` (`a = a`, `a = a.kids[0]`, `n.kids[0] = n`, `ring[i] = ring[j]`, the history loop, `p.game = p.game.sub`, `p.ring[k] = p.ring[j]`): no `__kestrel_assign_` call; an `Assign` whose value is a `Seq` with the copy (today's lowering).
    - `field_disjoint_places_reuse`: `p.ring[k] = p.game` and `p.game = p.ring[j]`: each a `__kestrel_assign_` call.
    - `a_target_index_that_can_write_keeps_the_old_copy`: `ring[next(var k)] = g`, `ring[pick(ring)] = g` and an index holding an `if` that assigns: no helper call, and the statements are in today's order.
    - `plain_data_needs_no_helper`: `ring[k] = g` for a struct of numbers, vectors and an `Rng`-shaped struct: a plain `Assign`, no helper.
    - `fresh_values_and_lets_are_unchanged`: `let x = y`, `g = make()`, `g = State(...)`, `g.xs += ys`: no helper.
    - `a_length_fact_dies_at_the_assignment`: `@assume(xs.len == 4)`, then `xs = ys`, then `xs[3]`: the range report keeps a bounds check on `xs[3]`.
    - `a_loop_over_the_target_reindexes`: `for x in g.xs { g = h; print(x) }`: the loop's element pointer is not used (`may_move` is true), as with today's assignment.
  - `cli/tests/snapshot_memory.rs::a_snapshot_ring_stops_allocating_after_its_first_lap`: a program with `State { xs: list[int], name: string, hp: int }`, a ring of 16 and a constant `PERIODS`. Each frame `f` refills `g.xs` in place (`clear`, then `add` up to length `f % 51`), saves `ring[f % 16] = g`, and every 7th frame restores `g = ring[(f + 11) % 16]`; `name` never changes (an equal string is kept). A period is 816 frames (16 slots times 51 lengths), after which every slot has held the longest list. The test writes four copies of the source: `PERIODS = 2` and `PERIODS = 200`, each with and without the save and restore lines (the control). Under `KESTREL_ARENA_REPORT=1` the two control runs print equal `kestrel: program arena N bytes` lines (the program's own work allocates nothing after the first period; if not, fix the program, not the compiler), and so do the two ring runs. Nothing is printed or interpolated inside the loop.
- [ ] **Step 3 (PC free):** run the new tests: `snapshot_memory` and the `kir` tests FAIL; the programs PASS (they pin today's behaviour).
- [ ] **Step 4: Implement** spec decision 9 (not maps: a map field is copied as today until Task 9). Reuse only when the places don't overlap (`PathKey::overlaps` after `origin`; field-disjoint paths of one root reuse) and the target's indexes can't write; otherwise today's lowering, untouched. The value's place is evaluated (stabilised) before the target's. The helper for a list or array: `old = len(dst)`; `if len(src) <= Cap(dst)`: plain items, `CopyItems(dst, src)`; heap items, `SetLen(dst, len(src))`, recursive assign over `0 .. min(old, len(src))` with `IndexCheck::Known` (`i < min` holds), fresh shallow copy plus `fixup` over `old .. len(src)`; else today's shallow copy plus `copy_fn`. Strings: `if not Str(Eq)`: shallow copy plus `Dup`. Struct fields: plain fields assigned, heap fields recursed. Optional and enum: same tag recurses into the payload or fields, else today's copy.
- [ ] **Step 5:** the new tests, then `cargo test -p kestrel-kir -- --test-threads=2`, `cargo test -p kestrel-cli --test programs -- --test-threads=2` (all programs: outputs unchanged), `cargo test -p kestrel-cli --test errors -- --test-threads=2`, `cargo test -p kestrel-cli --test closure_memory --test text_memory --test iface_memory -- --test-threads=2`, `cargo test -p kestrel-cli --test voxel_engine -- --test-threads=2`. Then `KESTREL_POISON=1` over the `assign_reuse_` programs in an exe. Expected: PASS. Clippy.
- [ ] **Step 6: Commit**: "Reuse the target's storage when a value is assigned over an existing one and its buffers have room, so a ring of snapshots stops allocating; places that may overlap keep today's copy".

### Task 9: Storage-reusing assignment: maps, and `no_alloc` over save and restore

**Model: Opus; review: yes.**

**Files:**
- Create: `kestrel/runtime/src/map_copy.rs` (`#[no_mangle] pub unsafe extern "C" fn kestrel_rt_map_copy_into(dst: *mut Map, src: *const Map, entry_size: i64, entry_align: i64) -> i8`: 1 when `dst` isn't locked, its entry capacity is at least `src.count` and its index table is at least the table `src.count` live keys need, after compacting `src`'s live entries into `dst`'s entries in order and rebuilding `dst`'s index; else 0 and nothing changed), `kestrel/tests/programs/assign_reuse_maps.kes`, `kestrel/tests/errors/prove_no_alloc_snapshot_list.kes`
- Modify: `kestrel/runtime/src/lib.rs`, `kestrel/kir/src/ops.rs` (`MapOp::CopyInto`, a `bool`), `kestrel/kir/src/lower/assign_into.rs` (maps: `if Map(CopyInto, dst, [src])` then fresh copies of heap keys and values over the entries, else today's copy), `kestrel/codegen/src/func/maps.rs`, `kestrel/codegen/src/rt.rs`, `kestrel/kir/src/range/events/*` (no event for `CopyInto`), `kestrel/cli/tests/snapshot_memory.rs`, `kestrel/cli/tests/contracts.rs`
- Test: map_copy.rs unit tests

**Interfaces:**
- Consumes: Task 8's helpers. Produces: `kestrel_rt_map_copy_into`, `MapOp::CopyInto`.

- [ ] **Step 1: Record today's outputs (PC free)** for `assign_reuse_maps.kes` as in Task 8 Step 1, with the compiler before this task: a `map[string, int]` assigned from maps of 3, 1, 6 and 0 keys (with removed keys in the source; printing shows insertion order kept); a `map[int, list[int]]`; a struct holding a map; and `for k, v in m { m = other; print(v) }` (today's outcome, error or not, recorded exactly).
- [ ] **Step 2: Write the failing tests.**
  - `map_copy::copies_in_place_when_there_is_room`: a destination with room gets the source's live entries in order, its `count`, `used == count`, the same `entries` pointer; lookups of every key succeed.
  - `map_copy::refuses_a_locked_or_small_map`: `locks > 0` → 0 and the destination is byte-for-byte unchanged; capacity too small → 0, unchanged.
  - `snapshot_memory.rs::a_map_in_a_snapshot_ring_stops_allocating`: as Task 8's ring test (with its control runs) with a `map[int, int]` field whose key count cycles 0 to 20; the builder shapes the map's own updates so the control runs are equal.
  - `contracts.rs::save_and_restore_of_plain_state_prove_no_alloc`: a program with `struct Game { ball: vec2, vel: vec2, p1: float, p2: float, score1: int, score2: int, rng: Rng }`, separate variables passed to `@prove(no_alloc) save(var ring: list[Game], k: int, g: Game) { ring[k % 16] = g }` and `@prove(no_alloc) restore(var g: Game, ring: list[Game], k: int) { g = ring[k % 16] }`, plus assignment statements on one root under a `@prove(no_alloc)` loop (`p.ring[k % 16] = p.game` and `p.game = p.ring[(k + 11) % 16]` for `struct Pair { game: Game, ring: list[Game] }`): `kestrel check` exits 0 with no output, and so does `kestrel check --release`. (`save(var p.ring, k, p.game)` stays refused by the exclusivity rule, so the test doesn't use it.)
  - `prove_no_alloc_snapshot_list.kes`: the same `save` for a `State` with a `list[int]` → "`save` can't be proven `no_alloc`: ..." naming the assignment and saying that it copies a list when the one there has too little room (the builder pins the exact chain text; it must read as a plain sentence).
- [ ] **Step 3 (PC free):** run them: the unit tests and the map ring test FAIL; `save_and_restore_of_plain_state_prove_no_alloc` should already PASS (heap-free state never allocated); if it fails, stop and report why.
- [ ] **Step 4: Implement** the map part of spec decision 9.
- [ ] **Step 5:** `cargo test -p kestrel-runtime map -- --test-threads=2`, `cargo test -p kestrel-cli --test snapshot_memory --test contracts --test errors -- --test-threads=2`, `KESTREL_TEST_ONLY=assign_reuse,map cargo test -p kestrel-cli --test programs -- --test-threads=2`, `cargo test -p kestrel-kir -- --test-threads=2`. Expected: PASS. Clippy.
- [ ] **Step 6: Commit**: "Reuse a map's entries and index when a map is assigned over one with room and not being looped over, and check that saving and restoring plain game state proves no_alloc".

### Task 10: The cross-build test

**Model: Sonnet; review: no.**

**Files:**
- Create: `kestrel/cli/tests/determinism.rs`, `kestrel/tests/programs/det_rng_checksum.kes`, `det_sim.kes`, `det_snapshot.kes`, `det_par.kes`

- [ ] **Step 1: Write the programs** (each with `// expect:` lines recorded from the JIT once it runs):
  - `det_rng_checksum.kes`: 20 draws of each `Rng` method from 3 seeds, and `checksum` of a struct holding every kind of value (numbers, vectors, string, list of structs, map, optional, enum, a NaN).
  - `det_sim.kes`: 200 bodies with `vec2` positions, 1,000 steps of a fixed `DT` with `sin`, `cos`, `atan2`, `exp`, `log`, `pow`, `sqrt` and a `par for`; prints the `checksum` of the state every 250 steps.
  - `det_snapshot.kes`: a ring of 16 snapshots; 300 frames; every 7th frame rolls back 5 frames and re-runs with the same inputs; prints the final `checksum` and the `checksum` of a straight run without rollbacks (equal).
  - `det_par.kes`: a float-heavy `par for` over 10,000 items, then `checksum`.
- [ ] **Step 2: Write the tests** in `determinism.rs`:
  - `the_jit_and_both_exes_print_the_same`: for each `det_*.kes`, `kestrel run`, `kestrel run --release`, a checked exe and a `--release` exe (built into `CARGO_TARGET_TMPDIR`); the four standard outputs are identical and non-empty.
  - `the_jit_and_exe_math_agree_bit_for_bit`: `det_math.kes` (every row of Task 1's table) in the four builds: identical output, equal to its `// expect:` lines, which are the runtime's pinned bits. This pins the JIT's copy of `libm` (`arch` on through `cranelift-codegen`) against the exes' (off).
  - `the_thread_count_changes_nothing`: `det_par.kes` and `det_sim.kes` with `KESTREL_THREADS` 1, 2 and 16 print the same.
- [ ] **Step 3 (PC free):** `cargo test -p kestrel-cli --test determinism -- --test-threads=2`, `KESTREL_TEST_ONLY=det_ cargo test -p kestrel-cli --test programs -- --test-threads=2`. Expected: PASS. A difference is a real finding (a compiler or runtime bug): stop and report it with the first differing line; don't change the program to hide it.
- [ ] **Step 4: Commit**: "Check that the JIT, a checked exe and a release exe print the same checksums and math results for the determinism programs, at any thread count".

### Task 11: The rollback Pong demo

**Model: Opus; review: no (the final review covers it).**

**Files:**
- Create: `kestrel/demos/pong/pong.kes` (main: the window, the fixed step from `window.delta()`, peers and network each frame, drawing, Tab and R, the headless ending), `kestrel/demos/pong/game.kes` (`Pad`, `Inputs`, `Game`, constants, `@prove(deterministic, no_alloc) pure step(var g: Game, i: Inputs)`, `pure bot(g: Game, side: int) -> Pad`), `kestrel/demos/pong/net.kes` (`Packet { from: int, frame: int, inputs: [Pad] (the last 8), sum_frame: int, sum: i64 }`, `Net` with its own `Rng`, `send`, `deliver(now) -> list[Packet]`; 6-frame delay, up to 2 frames of jitter, 5% loss), `kestrel/demos/pong/peer.kes` (`Peer`: its `Game`, a ring of 16 snapshots, confirmed and predicted input histories, `confirm`, `predict`, `rollback`, the checksum record and compare), `kestrel/demos/pong/replay.kes` (`Replay { seed: int, inputs: list[Inputs], sums: list[i64] }`, `record`, `check -> int` (the first differing frame or -1)), `kestrel/cli/tests/pong_demo.rs`. Each file under 400 lines.

- [ ] **Step 1: Write the tests** in `pong_demo.rs` (headless only, `KESTREL_HEADLESS=600`, `KESTREL_HEADLESS_QUIET=1`, hermetic environment):
  - `pong_runs_headless_and_the_three_builds_agree`: `kestrel run`, a checked exe and a `--release` exe: exit 0; standard output identical; it contains a line starting `frames 600 confirmed ` with a checksum, a line `replay matches (<F> frames)` with the same `F`, and no line containing `differ`.
  - `pong_rolls_back_and_recovers_lost_packets`: the output's `rollbacks <R> deepest <D> lost <L>` line has `R > 0`, `1 <= D <= 12` and `L > 0`.
  - `pong_reports_an_injected_desync`: the test writes a copy of the demo (all five files, into a temp folder) with `let DESYNC_AT = -1` replaced by `let DESYNC_AT = 300`; standard output contains `peers differ at frame 330` and the run stops there (no `replay matches`). The nudge (`ball.x += 0.001`) goes into peer 2's confirmed snapshot of frame `DESYNC_AT` when that frame is confirmed, so no rollback can undo it, and the first compared frame after it is 330.
  - `pong_memory_stays_flat`: under `KESTREL_ARENA_REPORT=1`, headless runs of 600 and 3,000 frames report program-arena sizes less than 64 KiB apart (each frame's drawing text is built inside `arena { }`; packets and histories are rings, not growing lists).
  - `pong_proves_its_contracts`: `kestrel check demos/pong/pong.kes` and `kestrel check --release ...` exit 0 with no output (`step` holds `deterministic` and `no_alloc`; `save` and `restore` hold `no_alloc`).
- [ ] **Step 2: Write the demo** per spec decision 11. Rules to keep it deterministic and testable: the bots read only a peer's own `Game`; the network's `Rng` is seeded with a constant; every random value in the game comes from `g.rng`; the step runs a fixed number of times per frame in a headless run (the headless clock is 60 frames a second); the headless ending's lines are the only output; a peer records and compares checksums of confirmed states only. Drawing uses `draw` only for the shown peer.
- [ ] **Step 3 (PC free):** `cargo test -p kestrel-cli --test pong_demo -- --test-threads=2`. Expected: PASS.
- [ ] **Step 4: Commit**: "Add the rollback Pong demo: two peers in one process over a simulated network with delay, jitter and loss, prediction and rollback, checksums every 30 frames and an in-memory replay, tested headless in three builds".
- [ ] **Step 5:** Tell the controller the user's play test command (from `kestrel/`): `cargo run -q -p kestrel-cli -- run demos/pong/pong.kes` (a real window: the user's to run, never an agent's), with what to check: W/S feels instant, the ball never jumps visibly, Tab shows peer 2's view, R says "replay matches".

### Task 12: Measurements (only when the user says the PC is free)

**Model: Opus; review: folded into the final review (honesty of the numbers).**

**Files:**
- Create: `kestrel/bench/determinism/results.md`, `kestrel/bench/determinism/snapshot.kes`, `kestrel/bench/determinism/measure.py`

- [ ] **Step 1:** Build the compiler from the commit before Task 1 in a worktree at `C:/kt/det-before` and HEAD's in `C:/kt/det-after`; run old and new in turn on an idle machine; record the load beside every number; medians and ranges of at least 9 runs.
- [ ] **Step 2: Measure:** (a) `cargo test -p kestrel-runtime --release own_math_against_the_platform -- --ignored --nocapture` (ns per call, each function, `libm` against the platform); (b) MXCSR pinning: per call (a loop of 100,000,000 `pin()` in the same ignored test file) and per frame; (c) the frozen world `bench/voxels3d/main.kes --release`, `bench/kernels` (`py measure.py --rounds 9 --only race --threads 1` and the noise and step rows), the engine bench's p99 per phase (`kestrel/bench/engine/measure.py`, 3 runs), generation A/B (64 columns through `load_now`), and `kestrel run --release` of the kernels for the JIT, old against new; (d) `snapshot.kes`: save and restore of Pong's `Game` (1,000,000 pairs) and of a voxel-sized `Pool` (441 columns of 32 x 128 x 32 blocks, 100 pairs), with reuse (HEAD) and without (before), startup subtracted; (e) `checksum` of both states.
- [ ] **Step 3: Write** `results.md` (a section "Determinism round 1 (2026-10-27)"), say plainly where Kestrel got slower, and if the JIT lost more than 2% anywhere, say so and propose the host ISA for the JIT (spec decision 4) for the controller.
- [ ] **Step 4: Commit**: "Measure the determinism round: own math, the float mode, the baseline CPU, snapshot copies and checksums".

### Task 13: Final review, the full suite and docs

**Model: Opus reviewer.**

- [ ] **Step 1:** One Opus whole-round review (the commits of Tasks 1 to 12) against the spec and the Review Focus; one fix wave; one re-review.
- [ ] **Step 2 (ask the controller first):** `cargo test --workspace --no-fail-fast` and `cargo clippy --workspace --all-targets -- -D warnings` in `kestrel/` (light mode).
- [ ] **Step 3: Self-check against the proposal's Round 1:** (1) own math with pinned bits and a speed comparison: Tasks 1, 12; (2) MXCSR: Task 2; (3) baseline CPU named in the build output: Task 2; (4) `@prove(deterministic)` with messages and error tests: Tasks 4, 5; (5) `Rng`: Task 6; (6) `checksum`: Task 7; (7) storage-reusing assignment with `no_alloc` tests over a ring: Tasks 8, 9; (8) lowest-index `par for` failure: Task 3; (9) the Pong demo: Task 11; (10) JIT, checked and `--release` agree: Task 10.
- [ ] **Step 4: Docs:** a new "Start here" in `docs/superpowers/plans/2026-10-10-handoff.md` (status, commits, numbers, what is next: round 2's saves and replays, the user's Pong play test), the "Start here" line in `CLAUDE.md`, the follow-ups file (`2026-10-09-core-2a-followups.md`) with what stayed open (`no_alloc` over list-holding snapshots needs a capacity fact; interface values in `checksum`; heap map keys and values copied fresh; MXCSR in the Godot add-on; `u64`). Search the changed docs for control characters. Commit; the controller pushes.
