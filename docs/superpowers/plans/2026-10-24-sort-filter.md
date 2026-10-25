# `sort_by` key caching and the `filter` cleanup (2026-10-24)

The user asked for these two fixes from the benchmark gaps (`kestrel/bench/kernels/results.md`, "The list methods"): `sort_by` was 6.1 times Rust's `sort_by` and 1.43 times a hand-written merge sort; `filter` was 23-24% slower than its hand loop. No language change. Light mode (the user is gaming): focused tests only, one job, idle priority; no timings until the user says the machine is quiet.

## `sort_by` with a number or bool key

**Before:** the runtime sorted positions with Rust's stable sort, calling a generated compare function `(env, a, b) -> u8` per comparison, which ran the key on both items (about `2 n log n` key runs).

**After:** the runtime calls a generated key function `(env, item) -> i64` (or `-> f64`) once per item, in item order, sorts `(key, position)` pairs with Rust's stable sort (the comparison inlined), then moves the items into that order as before.

Rulings:
- **R1. Which keys:** every key but a string (`int`, `i64`, `u8`, `bool` as `i64`; `float`, `f64` as `f64`). Widening keeps the order exactly (an `f32` is exact as an `f64`; `false` is 0 and `true` 1). String keys keep the compare function: caching them would mean keeping every key string alive for the sort, a different design. *If wrong:* string keys stay at today's speed.
- **R2. The float order is unchanged:** numbers by value, `-0.0` equal to `0.0` (stable, so they keep their order), a NaN after every number and equal to another NaN. Not `total_cmp`, which orders `-0.0` before `0.0` and NaNs by sign.
- **R3. A new `ListOp::SortBy(SortKey)`** (`SortKey::Int` or `Float`), with the same `[key_fn, key]` arguments as `Sort`'s `[compare, key]`; two runtime functions, `kestrel_rt_sort_by_int` and `kestrel_rt_sort_by_float`. A variant, not a flag in the arguments, so every match site has to decide.
- **R4. The key function keeps the scratch arena per call** (`PushArena` around the key's call, freed before returning the widened key), so a key that builds strings frees them per item as before; `strip_idle_scratch` takes it out when the key builds nothing, unchanged.
- **R5. The keys are computed once each, in item order.** Keys are `pure`, so the only visible difference is which item's failure is reported first when a key fails (both versions stop the program).
- **R6. Memory:** a buffer of 16 bytes an item (`Vec<(i64, u32)>`), next to the existing 4-byte position list and the item-sized temporary; freed before the sort returns.
- `sort()` with no key still uses a compare function per item type. A typed runtime sort for number lists would be the next step (follow-up, not this round).

## `filter`

From the dump in `results.md`: the method's loop loads the item twice (once for the test, once to add it) and loads the closure's environment word for an inlined body that never reads it.

- **R7. A scalar item is read once:** the loop binds it to a local and passes that to the test and to `add`. Aggregates and items with strings or lists keep today's path (an aggregate copy would cost more than the second read).
- **R8. The inliner skips an unused argument with no effect:** when the callee never mentions a by-value parameter and the argument is a constant, a closure's environment word (`FnEnv`) or a read of a place with no index, it isn't evaluated. General (any inlined call), not just `filter`.
- The literal's environment is still built on the stack once per call (two stores per call, not per item): left.

## Tests

- `kir/tests/list_sort.rs` updated: number keys make `__kestrel_sort_key_*` functions and `ListOp::SortBy`; string keys keep `__kestrel_compare_*`.
- New program tests: `sort_by` with every key type, NaN and `-0.0` float keys, equal keys keeping their order, a key that builds strings, an empty and a one-item list, arrays.
- Runtime unit tests for both new functions (stable, float order, empty).
- A KIR test for R7 and one for R8.

## As built (2026-10-24)

Built as written above, in one session on `main`, while the user was gaming: every build ran with one job at idle priority.
- **Files:** `runtime/src/sort.rs` (`kestrel_rt_sort_by_int`, `kestrel_rt_sort_by_float`, shared `count`/`at`/`permute`); `kir/src/ops.rs` (`ListOp::SortBy`, `SortKey`); `kir/src/lower/sort.rs` (`sort_key_fn`, `key_call`, `wide_key`); `kir/src/lower/scratch.rs` (the scratch-arena code, moved out of `sort.rs` to keep it under 400 lines); `kir/src/lower/list_fns.rs` (R7); `kir/src/inline.rs` (R8, `no_effect`); `codegen/src/func/lists.rs` and `codegen/src/rt.rs`; `kir/src/range/{writes,report,summary}.rs` and `events/reasons.rs` (a key function reads as "the comparison for `sort`" in reports, so messages are unchanged).
- **Tests:** `kir/tests/list_sort.rs` updated plus one new test; two new tests in `kir/tests/list_fns.rs`; three runtime unit tests; new programs `list_sort_by_keys.kes` and `list_filter_scalars.kes`. Results: `kestrel-kir`, `kestrel-codegen`, `kestrel-runtime` and `kestrel-check` 857 passed; the CLI tests 128 passed (every program under the JIT, as an exe and in the proof sweep; the 30 `window_smoke*` tests not run); 0 failed; clippy clean in `kestrel/`, `kestrel/godot` and `kestrel/bench/kernels`.
- **Review (Sonnet, read-only):** no Critical or Important findings. Its minors were fixed: a program check for a bool key (the `If` widening) and stale doc comments.
- **Not measured yet:** the user asked for no timings until the machine is quiet. Then run `py measure.py --rounds 9 --threads 1` with `--only filter`, `--only map` and `--only sort` (one label substring each) in `kestrel/bench/kernels` (old: `filter` 4.82 ns, `sort_by` 139.5 ns, against Rust 2.67 and 22.8) and update `results.md`.
