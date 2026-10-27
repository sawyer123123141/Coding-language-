# Cheaper lists in hot loops (design, DRAFT 2026-10-26)

**Status: draft.** Written from the IR while the machine was busy; the cost of each part is still to be measured. The stopped spike (`.superpowers/sdd/list-spike-partial.patch`) has switches for parts 1, 2, 3 (upper bound) and 4; its numbers fill the table below before this is planned. The user asked for this round ("sure", 2026-10-26, after the inliner round) and handed the decisions to the controller ("U choose review ur self"); every choice below is the controller's.

## Why

Kestrel loses the race against Rust's `block-mesh` by 1.36x (faces) and 1.78x (faces and arrays), and by 1.61x and 1.85x against a plain Rust port of the same kernels, so the gap is the compiler and runtime, not the crate's tuning (`kestrel/bench/engine/results.md`, "Race, round 1"). Reading the IR of the hot loop (followups entries 37-41, "Read from the IR") shows where: bounds checks that an earlier check implies, a list's header loaded again after every `add` to another list, every `add` going through memory with the list pointers spilled to the stack, and (from the race write-up) `clear()` zeroing the items and lists held in a struct running 14% slower than the same lists as locals (entries 29-30). A game's hot loops look like this: read a grid, append to a few lists. They should cost what the same loop costs in Rust.

## Goals

- G1. The race's kernels close most of the gap to the plain Rust port. Target, to be set from the spike's numbers: kernel 1 within 1.2x and kernel 2 within 1.4x of the plain port.
- G2. Nothing measured gets slower: the frozen world, the other `bench/kernels` rows, the engine's `bench.kes` p99, compile time.
- G3. No change to what a program does, including which line a failing program names (part 3 keeps that), and every `@prove` / `@assume` result the same or strictly better.

## Non-goals

- `Builder.face` over the inliner's limit (entry 20), the packed `p` computed in each branch (entry 40) and jump-only blocks (entry 41): small; picked up only if a part touches the same code anyway.
- A grid's length carried from its constructor (entry 37's second fix): a range-analysis round of its own.
- New syntax or library names.

## Anti-goals

- A wrong length or pointer after a hoist: a list grown, cleared, replaced or reached through another name inside the loop must never be read from a stale copy. This is memory safety; every part needs a test that tries to break it (an `add` that grows past capacity mid-loop, a `clear` in the loop, the list passed to a call, a list in a struct whose other field is written, a `par for`, a closure capturing it).
- A failing bounds check that names a different line or index than today.

## The parts (the controller's calls)

1. **Hoist the header of a list the loop can't change** (entry 38). For a loop, find the lists it only reads (a read-only parameter, or a list no write in the loop reaches except through an index), load their `{data, len}` once before the loop, and use those values inside it. Done in codegen where loops are lowered (the spike's `hdr_cache.rs` is one working shape, written as an experiment). The scan bails on any indirect, virtual or `par for` call, closure or interface value made in the loop, and on any write or rebinding of the list's root.
2. **Keep the header of a list the loop only reads and `add`s to in SSA values** (entry 39). `{data, len, cap}` live in variables across the loop; `add` updates them, calling the runtime's grow on the slow path and reloading after it; the length is stored back to memory at every `add` (write-through, the safe first step). Storing back only at the loop's exits and before anything that may read the header is the later step, taken only if the numbers say write-through leaves a real cost. A loop with more than a few such lists may lose more to register pressure than it gains (the spike caps it, `KSPIKE_ADDMAX`); the cap is set from measurement.
3. **Widen a run of bounds checks on one list** (entry 37). In a block that reads the same list at `i + c1`, `i + c2`, ... with no write to the list between, check `i + cmin >= 0` and `i + cmax < len` once before the first read; if that check fails, fall back to the original checks one by one, in order, so the failing program names the same line and index as today (G3; the slow path runs only on failure). Done in KIR after range analysis (it knows which checks remain), or in range analysis itself. The checks the analysis already proves stay proven.
4. **`clear()` of a list of plain items only sets its length to 0** (entry 29), where nothing can read past the length (the runtime zeroes on growth or `set_len` already, to be checked; if a later `add` relies on zeroed memory, keep the zeroing there instead).
5. **Lists in a struct** (entry 30): measure again after parts 1-2, which also cache a list reached by fields (`faces.neg_x`); if a gap is left, find its cause in the IR then.

## Measurements (to fill from the spike, then the round)

| Experiment | Kernel 1 ns/chunk | Kernel 2 ns/chunk | vs plain Rust | Load |
|---|---|---|---|---|
| Today (results.md) | 66,012 | 103,177 | 1.61x / 1.85x | idle |
| 1 hoist | TBD | TBD | | |
| 1 + 2 | TBD | TBD | | |
| 3 (upper bound: no checks) | TBD | TBD | | |
| 4 | TBD | TBD | | |
| all | TBD | TBD | | |

Then for the round: the race (9 rounds), the frozen world, the other kernel rows, `bench.kes` p99, compile time, and the IR of `race.faces_run` before and after (the redundant operations gone, by count).

## Constraints

- Light mode; never a build or benchmark while the user is gaming unless they say the machine is free (2026-10-26: a spike's builds froze Roblox).
- Files under about 400 lines; plain-sentence messages.
- Never a visible window; never a bare `link.exe`.

## Left to the builder

- Where exactly each part lives (codegen loop lowering, a KIR pass, range analysis), as long as the anti-goals' tests hold.
- The scan's data structures; the add-cap value from measurement.
