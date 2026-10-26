# Inlining bottom-up: the KIR inliner's budget (design, 2026-10-26)

## Why

The voxel engine build (2026-10-25/26) found that the KIR inliner's budget is charged per root function (followups entry 19, `docs/superpowers/plans/2026-10-09-core-2a-followups.md`). `kestrel/kir/src/inline.rs` works top-down from the original bodies: a call is inlined when the callee's original size is at most `MAX_NODES` (256) and the caller still has room in `GROWTH_BUDGET` (4,096 nodes), and everything nested inside an inlined callee is charged to the same root, down to `MAX_DEPTH` (8). So a big mid-level function such as `generate` gets inlined into `load_now` and the stream's `par for` body, and the budget runs out before the hot leaves inside it (`octave`, `lattice`, `mix`, `block_type`): 27 "the inlining budget of ... ran out" messages, generation 1.65 times slower than it could be, and in Task 4 a 26-70% loss whenever hot code was split into helpers under one root. A game author splitting a hot loop into readable helpers should not pay for it; that is the point of this round.

The user handed the decisions for this round to the controller ("U choose review ur self. Go", 2026-10-26). Every choice below is the controller's, backed by a measured spike (`.superpowers/sdd/inliner-spike-report.md`, patch `inliner-spike.patch` and harness `inliner-spike-harness/` beside it).

## Goals

- G1. Hot code split into helpers under one function is inlined as if it were written in one piece: the 27 budget messages in `voxel/main.kes`'s `check --report --release` go away, and generation (64 columns through `load_now`, the Task 10 A/B harness) runs within 3% of the spike's 296 ms (startup subtracted, medians of 9, load recorded).
- G2. Nothing measured gets slower: the frozen world (`bench/voxels3d/main.kes --release`, spike baseline 367 ms) and the `bench/kernels` rows within noise (~2%); compile time of `voxel/main.kes --release` no slower than today (spike: 346 → 272 ms).
- G3. Code size stays bounded on any program: a pathological chain (6 functions, each calling the next 20 times) compiles in well under a second with KIR size in the tens of thousands of nodes (spike: 117 ms, 15,923 nodes), pinned by a test.
- G4. The report stays honest: every call left in place has a plain-sentence reason, including the new one for a function too big after its own inlining.

## Non-goals

- `Builder.face` at 258 nodes (entry 20): raising `MAX_NODES` to 320 showed no gain in the race kernels in the spike. Left as is; entry 20 stays open with that evidence.
- Profile- or heat-driven inlining, or inlining across recursion beyond one level.
- A higher limit for calls with constant arguments (the spike's V2): not needed at the chosen limits; noted as the fallback if a later program shows a folding leaf losing.
- `GEN_BATCH` stays 2 (E65 holds: with all of generation inlined, 8 columns per frame still fail E32 on shared cores).

## Anti-goals (a failure even if it "works")

- A compile time or code-size blow-up on some program shape (the budget exists for this; G3 guards it).
- A contract (`@prove` / `@assume`, `Stmt::Prove` marking of inlined copies) that changes meaning: a probe that failed must still fail for the same reason, a contract that passed must still pass, unless the change is a strict improvement the report explains.
- A message that names a limit or a function wrongly, or local names printed as `(...)` in the report.

## The design (the controller's calls)

1. **Bottom-up order.** Build the call graph over `m.funcs`; find strongly connected components (Tarjan); process components callee-first (reverse topological order). Each function is inlined into once; its inlined body and its *post size* (KIR nodes after its own inlining) are what its callers see.
2. **When a call is inlined.** Callee `g` into caller `f` when all hold: `g` is inlinable as today (not native, not directly recursive, no early `return`); `g`'s **original** size ≤ `MAX_NODES` (256, unchanged: a function written big stays a call); `g`'s **post size** ≤ `POST_LIMIT` = 4,096; and `f`'s remaining budget ≥ `g`'s post size, from `BUDGET` = 8,192 per function, charged only for direct copies.
3. **Copies are not re-descended.** A copied body is already inlined; it is renamed into the caller and not walked again for further inlining. There is no depth limit (`MAX_DEPTH` and its message go away): depth is bounded by the bottom-up order. (Changed in the final review, 2026-10-26: without a limit a linear chain of small calls grew every function in it to about `POST_LIMIT`, and a 2,000-deep chain took 15.6 s to build. A nesting cap of 16 came back, measured on finished bodies: each function records how deep the copies in it nest, and a call whose copy would nest deeper stays a call with the reason "inlined calls nest more than 16 deep"; followups entry 35.)
4. **Recursion.** A callee in the caller's own component (mutual recursion) is inlined at most one level, from its original body, as long as the other rules allow; a directly recursive function is never inlined (as today).
5. **Which calls win when the budget can't fit them all:** calls inside loops before calls outside them (deeper loop nesting first), then program order. (The spike showed program order alone picked the cold one of two `solid_at` copies at a budget of 4,096. At 8,192 it doesn't matter for the engine; the loop rule keeps it from mattering elsewhere. Builder: the cheapest correct way to get this order is the builder's call — e.g. a first pass that collects call sites with their loop depth.)
6. **Contracts.** A contracted callee's copy is still wrapped in its sites' `Stmt::Prove` blocks, now around the already-inlined body. Its value temporary rule (`marked`) is unchanged.
7. **Report reasons** (`NotInlined::reason`, `kir/src/ir.rs` docs): keep "it is N nodes; the limit is 256" (original size), "it calls itself", "it has an early `return`", "it is implemented in Rust" (silent), "the inlining budget of `f` ran out"; add "it is N nodes after its own inlining; the limit is 4,096"; drop "inlined calls nest more than 8 deep"; and add **"it calls back into `g`"** for a call left inside a one-level copy of a cycle-mate (rule 4), naming the function actually called (not the one being inlined into: in a cycle of three that would name the wrong function). (Builder check, 2026-10-26: the controller's call.) A callee's left calls are re-logged under each caller that copies it, since `range/contracts.rs` and `range/items.rs` look reasons up by (caller, callee).
8. **The local-names bug the spike found.** Copying an already-inlined body chained local names (`a.b.c...`) and the report printed `(...)`; locals that came from an inlined body keep their existing name when copied again (one prefix, the immediate callee's).

## Constraints

- No new syntax. No change outside `kestrel/kir` except tests, docs and measured results.
- Files under about 400 lines: `inline.rs` is ~310 today; the call-graph and ordering part goes in its own module (e.g. `inline/order.rs`) if it grows past that.
- Error and report messages are plain sentences.
- Light mode while measuring (`CARGO_BUILD_JOBS=2`, low priority), CPU load recorded beside every number; never a visible window; never a bare `link.exe`.
- The frozen world's sources stay frozen. Its `check --report` and `--report --release` output may change only in inlining lines; any change is shown in the results.

## Tests that change, and new ones

- `kestrel/kir/tests/range_report.rs`: `the_budget_and_the_nesting_are_recorded` pins the budget message and the depth message; rewrite it to pin the budget message with the new numbers and the new "after its own inlining" message; `a_filter_that_isnt_inlined_still_says_what_it_builds` expects some of 150 filters to stay calls: make it exceed the new budget or the post limit.
- `kestrel/tests/errors/prove_inline_copy.kes` (plan `2026-10-20-round-b-annotations.md`, item I1: `@prove(inline)` holds in `f2`'s own body but fails in its copy in `f0`, with the "where it is inlined into `f0` (line N)" clause) **can't be triggered any more**: copies are exact (rule 3) and the own body is checked first (`kir/src/range/contracts/region.rs:58`), so an `inline` failure that shows only in a copy no longer exists, a strict improvement. Decision (builder check, the controller's call): turn the test into `f2`'s own body failing `@prove(inline)` with the plain message (no copy clause), say in its header why the copy-only case is gone, and keep the "where it is inlined into" clause covered by a `no_checks` contract whose copies differ through range facts (use an existing test if one already pins that clause; otherwise add one).
- New: the pathological chain (G3) as a KIR test with a node-count bound and a time bound loose enough not to flake; a bottom-up test (a mid-level function too big after inlining stays a call while its leaves are inlined inside it); a mutual-recursion test (one level, terminates); a loop-priority test (the call in the loop wins the last of the budget); the local-name test (no `(...)` in the report for a twice-inlined local).
- The whole workspace suite and clippy at the end; `voxel_engine` (contract probes) and `programs` along the way.

## Measurements to redo (results)

In `kestrel/bench/engine/results.md` (a new section "Inlining bottom-up (2026-10-26)") and `kestrel/kir` docs as fits: generation A/B, frozen world, the kernel rows, compile time of `voxel/main.kes` and `bench/voxels3d/main.kes`, exe size, the report's not-inlined counts by reason, `bench.kes` p99 per phase (E32; the spike didn't measure it, a scratch build with the budget raised showed 14.5-15.6 → 8.7-12.1 ms), and the pathological chain. Followups entry 19 marked done with the numbers; entry 20 updated with the spike's no-gain evidence; the engine's "As built" gets no entry (this is a compiler round), the handoff a new "Start here".

## Left to the builder

- Data structures for the call graph and SCCs; how post sizes are stored.
- How the loop-depth priority is computed (rule 5).
- Exact test names and fixtures; the pathological test's bounds (as long as they'd catch a 10x regression).
- Whether to keep the spike's code or rewrite it clean (the spike is evidence, not reviewed code).
