# Inlining Bottom-Up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the KIR inliner work bottom-up so hot code split into helpers is inlined as if written in one piece, without code-size blow-ups.

**Architecture:** `kestrel/kir/src/inline.rs` keeps its entry point `inline_small(m: &mut Module)`. A new `inline/order.rs` gives the call graph's strongly connected components callee-first (Tarjan). Each function is inlined into once, in that order; callers copy callees' already-inlined bodies, judged by post size against a per-function budget charged only for direct copies.

**Tech Stack:** Rust (Cargo workspace `kestrel/`), crate `kestrel-kir`; tests in `kestrel/kir/tests/`, `kestrel/tests/errors/`, `kestrel/cli/tests/`.

**Spec:** `docs/superpowers/specs/2026-10-26-inliner-budget-design.md` (read it whole; rules 1-8 are numbered there). Spike evidence: `.superpowers/sdd/inliner-spike-report.md`, `inliner-spike.patch` (unreviewed; reference only) and the harness `inliner-spike-harness/` (`spk.py` generation A/B, `kern.py`, `patho.kes`, `tests.sh`, `totals.py`).

## Global Constraints

- Limits: `MAX_NODES = 256` (original size), `POST_LIMIT = 4096` (size after the callee's own inlining), `BUDGET = 8192` (per function, charged for direct copies only). `MAX_DEPTH` is removed.
- Report reasons, exact text: "it is {n} nodes; the limit is 256"; "it is {n} nodes after its own inlining; the limit is 4,096"; "the inlining budget of `{caller}` ran out"; "it calls itself"; "it has an early `return`"; "it calls back into `{callee}`" (the function actually called); natives silent. "inlined calls nest more than 8 deep" is gone.
- No new syntax; no change outside `kestrel/kir` except tests, docs and results.
- Files under about 400 lines; plain-sentence messages and comments.
- Light mode: `CARGO_BUILD_JOBS=2`, `--test-threads=2`, low priority; CPU load recorded beside every timing; never a visible window (`KESTREL_HEADLESS` / `KESTREL_HIDDEN` only); never a bare `link.exe` (use a short `CARGO_TARGET_DIR` such as `C:/kt/...` on LNK1104).
- Never edit `kestrel/bench/voxels3d/` sources, `kestrel/test.kes`, `voxels.exe`; never stage `kestrel/check/src/body/enums.rs`. Stage by explicit path. Commits on `main`, plain-sentence messages ending `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; the controller pushes.

## Review Focus

1. **A contract on an inlined function** (`@prove(no_alloc|no_checks|no_panic|inline)`, `@assume`): copies are wrapped in `Stmt::Prove` around the already-inlined body; every `kestrel/voxel/tests/contract_*.kes` probe and `kestrel/tests/errors/prove_*.kes` must give the same result as before (Task 1 runs them).
2. **Mutual recursion and recursion through closures/function values** (`list_fns` lowers closures to `Call(code, [FnEnv(f), item])`): must terminate and inline at most one level (Task 1 test `mutual_recursion_inlines_one_level`; also run `kir/tests/closures.rs`, `fn_values.rs`, `captures.rs`).
3. **Many call sites of one mid-size callee** (the budget exhausted part-way): the calls in loops win (Task 2 test).
4. **A callee whose left calls appear in each caller's report**: reasons re-logged per (caller, callee), as `range/contracts.rs` and `range/items.rs` look them up (Task 1 test `left_calls_are_reported_under_each_caller`).
5. **Huge fan-out chains**: bounded nodes and time (Task 1 test `a_fan_out_chain_stays_small`).

---

### Task 1: Bottom-up inlining

**Model: Opus; review: yes.**

**Files:**
- Create: `kestrel/kir/src/inline/order.rs`
- Modify: `kestrel/kir/src/inline.rs`, `kestrel/kir/src/inline/descend.rs`, `kestrel/kir/src/ir.rs` (the `NotInlined::reason` docs near line 367 and line 197)
- Test: `kestrel/kir/tests/inline.rs` (new tests), `kestrel/kir/tests/range_report.rs` (rewrite two), `kestrel/tests/errors/prove_inline_copy.kes` (convert)

**Interfaces:**
- Produces: `pub(super) fn callee_first(funcs: &[Func]) -> Vec<Vec<FuncIdx>>` in `order.rs` (components in callee-first order; each component's functions in index order); `inline_small(m: &mut Module)` unchanged; constants `MAX_NODES`, `POST_LIMIT`, `BUDGET` in `inline.rs`; the reason texts above.

- [ ] **Step 1: Write the failing tests** in `kestrel/kir/tests/inline.rs` (follow the file's existing pattern of building a module from Kestrel source and calling `inline::inline_small`):
  - `a_big_middle_function_stays_a_call_and_keeps_its_leaves_inlined`: `leaf` (small arithmetic), `mid` (a loop calling `leaf` enough times that `mid`'s post size > 4,096 while its original size ≤ 256), `root` calling `mid`. Assert: `root` still has a call to `mid`; `mid` has no call to `leaf`; `not_inlined` has an entry (caller `root`, callee `mid`) with reason "it is N nodes after its own inlining; the limit is 4,096".
  - `mutual_recursion_inlines_one_level`: `a` calls `b`, `b` calls `a` (both small), `main` calls `a`. Assert: terminates; `a`'s body contains `b`'s body once and a call to `a` with reason "it calls back into `a`"; no "nest more than" reason anywhere.
  - `left_calls_are_reported_under_each_caller`: `g` calls a native or a big function `h`; `f1` and `f2` both call `g` (inlined). Assert `not_inlined` has (f1, h) and (f2, h) entries with the same reason.
  - `a_twice_inlined_local_keeps_its_name`: `inner` has a local `n` used in a check; `outer` inlines `inner`; `top` inlines `outer`. Assert the report text for `top` (via the same helper `range_report.rs` uses) names `n` (one prefix, e.g. `inner.n`) and contains no `(...)`.
  - `a_fan_out_chain_stays_small`: six functions `f0..f5`, each `f(i)` calling `f(i+1)` 20 times (use `inliner-spike-harness/patho.kes` as the source). Assert total KIR nodes after `inline_small` < 100,000 and the call takes < 10 s in a debug build.
- [ ] **Step 2: Run** `cd kestrel; CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir --test inline -- --test-threads=2`. Expected: the five new tests FAIL (the fan-out test may hang or exceed its bound — give it the bound check, not a hang: assert on node count after a run with today's code only if it finishes; if today's code runs past ~60 s, record that and mark RED by its node count on a smaller fan-out of 12).
- [ ] **Step 3: Implement** in `order.rs`: `callee_first` with Tarjan's SCC over `Call` edges (statement and expression calls; closure calls through `Call(code, [FnEnv(f), ...])` count as edges to `code`). In `inline.rs`: process `callee_first(&m.funcs)`; for each function, inline from the current (already-processed) bodies of callees outside its component, and from original bodies (one level, rule 4) for callees inside it; record each function's post size; apply rule 2 with `MAX_NODES`, `POST_LIMIT`, `BUDGET`; do not re-descend copied bodies (rule 3); re-log each copied callee's left calls under the new caller with the same reason (Review Focus 4); keep `marked` and `Stmt::Prove` wrapping around the already-inlined copy (rule 6); fix local naming so a local from an already-inlined body keeps one prefix (rule 8). Remove `MAX_DEPTH`. Update `ir.rs` docs to the reason list in Global Constraints. Keep `inline.rs` under ~400 lines (move pieces to `order.rs`/`descend.rs` as needed). The spike patch shows one working shape; rewrite it cleanly.
- [ ] **Step 4: Update pinned tests** (they break by design):
  - `range_report.rs::the_budget_and_the_nesting_are_recorded`: pin the budget message with the new numbers (make the test's callee count exceed 8,192 nodes of direct copies) and the "after its own inlining" message; drop the depth assertion.
  - `range_report.rs::a_filter_that_isnt_inlined_still_says_what_it_builds`: raise the filter count (or size) so some still stay calls under the new limits; keep what it asserts about the message.
  - `kestrel/tests/errors/prove_inline_copy.kes`: per the spec's "Tests that change": `f2`'s own body fails `@prove(inline)` with the plain message; header explains the copy-only case is gone. Then find a test that pins the "where it is inlined into `f0` (line N)" clause for `no_checks` (grep `kestrel/tests/errors/` and `kir/tests/` for "where it is inlined into"); if none, add `kestrel/tests/errors/prove_no_checks_copy.kes` where a callee's copy fails `no_checks` only in one caller because range facts differ there.
- [ ] **Step 5: Run** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir -- --test-threads=2`, `cargo test -p kestrel-cli --test programs -- --test-threads=2`, `cargo test -p kestrel-cli --test errors -- --test-threads=2` (or the crate's error-test target; check `kestrel/cli/tests/`), `cargo test -p kestrel-cli --test voxel_engine -- --test-threads=2`. Expected: all PASS; every `contract_*.kes` probe unchanged. Then `kestrel check --report --release voxel/main.kes` (from `kestrel/`): zero "inlining budget ... ran out"; `generate` left with the "after its own inlining" reason.
- [ ] **Step 6: Commit** (stage the files above by path).

### Task 2: Loop calls win the last of the budget

**Model: Sonnet; review: no (final review covers it).**

**Files:**
- Modify: `kestrel/kir/src/inline.rs` (and/or `inline/descend.rs`)
- Test: `kestrel/kir/tests/inline.rs`

**Interfaces:**
- Consumes: Task 1's `inline_small`, `BUDGET`, the post sizes.

- [ ] **Step 1: Write the failing test** `calls_in_loops_win_the_last_of_the_budget`: a caller with two calls to the same callee whose post size is just over half of `BUDGET` (so only one copy fits): the first call written outside any loop, the second inside a `for` loop. Assert the loop call is inlined and the outside call is left with "the inlining budget of `caller` ran out". Add a nested-loop variant: of a call in a single loop and one in a double loop (budget fits one), the double-loop call wins.
- [ ] **Step 2: Run** `cargo test -p kestrel-kir --test inline calls_in_loops -- --test-threads=2`: FAIL (program order picks the first).
- [ ] **Step 3: Implement** rule 5: choose which calls of a function get the budget by loop depth (deepest first), then program order. A first pass collecting call sites with their loop depth is the expected shape; the decision must be deterministic.
- [ ] **Step 4: Run** the kir suite and `--test voxel_engine`: PASS.
- [ ] **Step 5: Commit.**

### Task 3: Measure, and write it down

**Model: Opus; review: yes (honesty of the numbers).**

**Files:**
- Modify: `kestrel/bench/engine/results.md` (new section "Inlining bottom-up (2026-10-26)"), `docs/superpowers/plans/2026-10-09-core-2a-followups.md` (entries 19 and 20), `kestrel/bench/voxels3d/results.md` only if its reported numbers change materially (add a dated note; don't rewrite history)

- [ ] **Step 1: Measure** with the repo compiler before (build from the commit before Task 1 in a temp worktree at a short path) and after (HEAD), in turn, idle machine, load recorded:
  - generation A/B (64 columns through `load_now`, `inliner-spike-harness/spk.py`'s method; spike: 489 → 296 ms; G1 target: within 3% of 296);
  - frozen world `bench/voxels3d/main.kes --release` (G2: within ~2%);
  - `bench/kernels`: `py measure.py --rounds 9 --only race --threads 1` and the noise/step rows the spike ran (`kern.py`);
  - compile time `kestrel build --release` of `voxel/main.kes` and `bench/voxels3d/main.kes`, 9 runs, median and range; exe sizes;
  - `check --report --release` not-inlined counts by reason, before and after, for both programs (the frozen world's report lines may change only in inlining; show the diff);
  - `bench.kes` p99 per phase (`kestrel/bench/engine/measure.py`, 3 runs) at `GEN_BATCH` 2;
  - the fan-out chain (`patho.kes`) compile time and KIR nodes.
- [ ] **Step 2: Write** the results section (every number with median, range, load; the spike's numbers beside for comparison), mark followups entry 19 done with the numbers, update entry 20 with the spike's no-gain evidence for `MAX_NODES` 320 (left open).
- [ ] **Step 3: Commit.**

### Task 4: Final review and the full suite

**Model: Opus reviewer.**

- [ ] **Step 1:** One Opus whole-branch review over the round's commits (correctness of the order and budget, contracts, reports, tests, honesty of Task 3). One fix wave, one re-review.
- [ ] **Step 2:** `cargo test --workspace --no-fail-fast` and clippy in `kestrel/` and `bench/kernels`.
- [ ] **Step 3:** Docs: the handoff's new "Start here" (status, numbers, what's next), `CLAUDE.md`'s "Start here" line. Commit; the controller pushes.
