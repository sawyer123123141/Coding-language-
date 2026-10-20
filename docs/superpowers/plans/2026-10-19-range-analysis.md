# Range Analysis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove overflow, shift, division, narrowing and bounds checks that can't fail, leave them out of the code, and list what is left (with allocations, other ways to fail and calls not inlined) in `kestrel check --report`.

**Architecture:** A new KIR module, `kir/src/range/`, interprets each function abstractly after `optimize`: ranges for tracked integer locals, `a - b <= c` facts between at most 64 terms (locals, list and array lengths, loop ends), one statement at a time with writes forgotten before and after, loops repeated to a fixed point with widening. It marks proven checks (`IndexCheck::Proven`, `ExprKind::Proven`) and returns what is left. Codegen runs it inside `define_module` on a copy of the module, with the build's arithmetic mode, and emits proven operations without their checks. The report reads the same result plus a walk for allocations, failures and calls, summarised through the call graph.

**Tech Stack:** Rust, Cranelift 0.116. Tests are Rust unit tests plus `.kes` program, release and report tests.

**Spec:** `docs/superpowers/specs/2026-10-19-range-analysis-design.md` (revised after the Opus review). Every message, rule and number comes from it; the spec wins any conflict.

## Global Constraints

- **Soundness first.** A removed check that could fire is a Critical bug. Where a rule is unclear, keep the check. Every proof rule gets a test with a near miss that must keep its check; each of the spec's five Critical rules (statement order, sibling operands, release facts, `continue` through the step, conditions that write) gets a program that must still fail, in the task that builds the rule.
- **Messages and report lines** are plain sentences, worded exactly as in the spec ("What changes for a program", "The report", "Checking the proofs", "Loops") and in Task 4's fixture.
- **File sizes:** under about 400 lines; split before passing it. New code goes in `kir/src/range/`: `mod.rs` (entry, per-function driver, limits), `interval.rs`, `sites.rs` (which checks codegen emits), `show.rs` (expressions and ranges as text), `state.rs` (ranges and facts), `terms.rs` (terms, stable places, homes, may-overlap), `writes.rs` (what a statement or expression may write), `eval.rs` (expressions), `refine.rs` (conditions), `stmt.rs` (the four steps, statements), `loops.rs` (fixed points), `report.rs` (items and summaries), `render.rs` (text).
- **Numbers from the spec:** `MAX_ITEMS = 2_147_483_647` (lists and arrays); `MAX_TERMS = 64`; a budget of `max(200_000, 100 * statements)` statement visits and a nesting limit of 256 per function; fact constants dropped past 2^66 in size; shift amounts clipped to 0..127 inside the analysis; report expressions cut at 40 characters with `...`.
- **No integer fields in memory as terms** (controller's ruling): terms are tracked locals (`Value` or `Addressed`, never `RefParam`), `Len` of stable places, and loop ends.
- **Every exhaustive `ExprKind` match** gets a `Proven` arm that walks into the inner expression: `kir/src/visit.rs`, `visit_mut.rs`, `rename.rs`, `inline/descend.rs`, `lower/lines.rs`, `lower/sort.rs`, `scalarize.rs`, `promote.rs`, `split.rs`, `stack_envs.rs`, codegen `func/expr.rs` (the compiler lists any others). Grep the `if let ExprKind::Binary`/`ExprKind::Int` shape tests in codegen and KIR (codegen's `const_divisor` in `func/expr.rs:77`, for one) and make each see through `Proven` or note why it needn't.
- **`Options` literals** become `Options { checked_arith: x, ..Options::default() }` everywhere: `cli/src/main.rs`, `codegen/tests/{convert,dump,jit,natives}.rs`, `godot/src/lib.rs` and `bench/kernels/src/main.rs` (the last two are separate workspaces: build and clippy them too).
- **Runtime functions** need an `RtFuncs` field and `decl` in `codegen/src/rt.rs`, a `jit_symbols()` entry, and `#[no_mangle] pub extern "C"`; a failing one fails through `panic::fail`.
- **LIGHT MODE testing** (the user asked that tests not lag the machine):
  - Every cargo command runs with `CARGO_BUILD_JOBS=2` (PowerShell: `$env:CARGO_BUILD_JOBS=2`) and tests with `-- --test-threads=2`.
  - Run focused targets: the task's own test files, `-p kestrel-kir`, `-p kestrel-codegen`, `-p kestrel-runtime`, the cli `errors`, `report` and `release` targets, and the program target filtered with `KESTREL_TEST_ONLY` (Task 1 adds it: a comma-separated list of file-name prefixes).
  - Run long commands at idle CPU priority under a watcher that waits for them to end; don't poll in a loop.
  - No window, GPU or demo tests (`window_smoke*`, `voxels_demo`, `demos`, `window_print_flush`).
  - **Required at the end of Tasks 2 and 3:** the proof check over every program, in all three passes (JIT, exe with `KESTREL_TEST_VERIFY_EXE=1`, and the release pass), in light mode (one batch of `KESTREL_TEST_ONLY` prefixes at a time if the machine is busy). Task reports give the result; a task isn't done without it.
  - The full `cargo test --workspace --no-fail-fast` runs only at the end of Task 5, and only when the user allows it; until then it is owed, and task reports say so.
  - Clippy per task on the crates touched: `cargo clippy -p <crate> --all-targets -- -D warnings`.
- **Never call a bare `link.exe`:** Git's coreutils `link.exe` shadows the MSVC linker on PATH (`find-msvc-tools` finds the real one).
- **Test facts:**
  - Program tests: `kestrel/tests/programs/*.kes` with `// expect:`, `// expect-stderr:` and `// expect-exit:` lines; JIT and exe, poison on.
  - No string literals inside `{...}` in program sources.
  - To keep a value unknown to the analysis in a program test, get it from a helper that isn't inlined: `id(x: int, d: int) -> int { if d == 0 { return x } \n id(x, d - 1) }` (recursive with an early `return`), and `id64` likewise.
  - `kestrel_codegen::dump` prints a function's machine code; each check's failure path ends in `trap user2`, so counting `trap user2` counts the checks left.
- **Off-limits:** `kestrel/test.kes`, `voxels.exe`, `kestrel/b.exe`, and the line-ending-only change in `kestrel/check/src/body/enums.rs`. Never use `git add -A`, `git stash` or `git clean`.
- **Commits:** plain-sentence messages ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Work on `main`; the controller pushes.

## Review Focus

1. **A write the forgetting rules miss:** a loop element's list replaced (`grid[0] = []` in `for row in grid`), a `var` call, a lent interface value, a `par for`, `Compact`, a map write, a `Move`, a length moved for a list that only may be the written one. The check must stay and fire. (Task 3.)
2. **A wrapped value or an unemitted check proving a bounds check under `--release`:** `let j = i + 1` with `i` at the maximum, `if i < n - 1` with `n` near the minimum, `1 << n` then `xs[n]`. Each must fail as an index error, not read memory. (Task 3, `tests/release/`; Task 2 for the rule.)
3. **A fact outliving its value or crossing operands:** `i = xs[i]`, `xs[i] + shrink(var xs)`, `get(xs, i) + pop_one(var xs)`, `x = i64(int(x)) + x`, a condition whose later part pops (`i < xs.len and pop_one(var xs) > 0`), a check in an `if` arm or on the right of `and`. The later check must stay. (Tasks 2 and 3.)
4. **Loop edges:** `continue` in a `for` (through the step), an empty loop, a counter counting down, a `break` path's facts, a nested loop decided in a non-final outer pass, an inner loop entered again (a fresh end). (Tasks 2 and 3.)
5. **Every route to a list or array past `MAX_ITEMS`:** `add`, `insert`, list literals, `array[T](n)`, `split`, `filter`/`map`, and a Godot array through the add-on's bridge. Each fails with the spec's message. (Task 1.)

---

### Task 1: Check markers, the proof check and the item limit

**Files:**
- KIR: `kir/src/ir.rs` (`IndexCheck`, `Proj::Index { at, check }`), `kir/src/ir_expr.rs` (`ExprKind::Proven`), the construction sites of `Proj::Index` (`lower/call.rs:31`, `lower/collections.rs:46,110,119`, `lower/helpers.rs:255`, `lower/lists.rs:38,65,131`, `lower/list_fns.rs:62`, `lower/list_helpers.rs:23`, `lower/remove.rs:66,111`: `checked: true` becomes `Checked`, `false` becomes `Known`), and every exhaustive `ExprKind` match (Global Constraints).
- Codegen: `lib.rs` (`Options`), `func/arith.rs`, `func/divide.rs`, `func/expr.rs`, `func/place.rs`, `rt.rs`, `jit.rs`.
- Runtime: `runtime/src/panic.rs` (`PANIC_BOUNDS`, `kestrel_rt_proof_failed`, `proof_failed_message`), `runtime/src/arena.rs` (`MAX_ITEMS`, `grown_capacity`, `items_fit`, the limit in `kestrel_rt_list_grow` and `kestrel_rt_alloc`), `runtime/src/string_make.rs:100` (`split`), and any other place that sets a list's length (grep `len =` and `List {` in `runtime/src` and codegen's `func/lists.rs`, `func/collections.rs`).
- Godot: `godot/src/bridge.rs` (`put_seq`): a host array longer than `MAX_ITEMS` is refused with "a list can't hold more than 2,147,483,647 items" when it is passed as a list, and "an array can't hold more than 2,147,483,647 items" as a `[T]` (strings are not limited).
- CLI: `cli/src/main.rs` (reads `KESTREL_CHECK_PROOFS` for `run` and `build`). `cli/tests/programs.rs`: `KESTREL_TEST_ONLY` (prefixes) filters `programs()`; the JIT pass sets `KESTREL_CHECK_PROOFS=1` after `hermetic` scrubs the environment; `KESTREL_TEST_VERIFY_EXE=1` makes the exe pass build with it too; a new `programs_verify_release` runs `kestrel run --release` on every program with it and fails only when stderr has "range analysis removed" (each program with a 30-second timeout, since a program whose checked run stops on an overflow may loop when it wraps; a timeout is reported, not a hang); `compare` fails any run whose stderr has that sentence.
- Every `Options` literal (Global Constraints).
- Tests: new `codegen/tests/proven.rs`; runtime unit tests; a `godot` unit test; `tests/programs/array_too_long.kes`.

**Interfaces:**
- Produces:
  - `pub enum IndexCheck { Checked, Known, Proven }` and `Proj::Index { at: Expr, check: IndexCheck }`.
  - `ExprKind::Proven(Box<Expr>)`: the inner expression is an integer `Binary(Add | Sub | Mul | Div | Rem | Shl | Shr, ..)`, `Unary(Neg, ..)` or a narrowing integer `Cast`, all of whose checks were proven. For `/` and `%` that means the divisor is never 0 and never -1 with the dividend at its type's minimum, so codegen emits a plain divide (no select).
  - `pub struct Options { pub checked_arith: bool, pub remove_checks: bool, pub verify_proofs: bool }`, default `(true, true, false)`. `remove_checks` is read from Task 2 on.
  - Codegen's `pub(crate) enum Check { Emit, Skip, Verify }`, passed to `binary`, `int_arith`, `shift`, `int_div`, `cast` and the bounds check. `Emit` is today's behaviour; `Skip` emits no check; `Verify` emits the check but its failure calls `kestrel_rt_proof_failed`. A `Proven` node lowers its operands with `Emit` (they carry their own wrappers) and itself with `Skip`, or `Verify` when `opts.verify_proofs`; an `IndexCheck::Proven` index likewise.
  - Runtime: `pub const MAX_ITEMS: i64 = i32::MAX as i64`; `pub fn grown_capacity(cap: i64) -> Option<i64>` (`max(4, 2 * cap)` capped at `MAX_ITEMS`, `None` when `cap` is already `MAX_ITEMS`); `pub fn items_fit(n: i64) -> bool`; `pub const PANIC_BOUNDS: i32 = 8`; `kestrel_rt_proof_failed(kind: i32, line: i32, file: *const FileName) -> !`; `pub fn proof_failed_message(kind: i32, line: i32, file: &str) -> String` giving "range analysis removed the {overflow|shift|division|narrowing|bounds} check on line {line} of {file}, but it fails; this is a compiler bug" (no " of {file}" when there is no file).
  - List growth past the limit fails with "a list can't hold more than 2,147,483,647 items"; `kestrel_rt_alloc` with a count past it fails with "an array can't hold more than 2,147,483,647 items; this one would have {count} on line {line} of {file}" (plain digits; the existing "too large" message stays for sizes that overflow).

- [ ] **Step 1: Write the failing tests.**
  - `codegen/tests/proven.rs`, with a helper `prove(m: &mut Module, func: &str)` (a `VisitMut` that wraps every integer `Add`, `Mul` and `Div` in `Proven` and sets every `IndexCheck::Checked` to `Proven` in that function), compiled with `remove_checks: false` so only the helper's marks count:
    - `a_proven_add_has_no_check`: `pure f(x: int) -> int { x + 1 }`, proven, compiled checked: `call1(f, 2147483647) == -2147483648`, and `dump` of `f` has no `trap user2` (unproven: one).
    - `a_proven_index_has_no_check`: `pure g(xs: [int], i: int) -> int { xs[i] }` proven: `dump` of `g` has no `trap user2`.
    - `a_proven_division_divides_plainly`: `pure d(a: int, b: int) -> int { a / b }` proven: no `trap user2`, and `d(7, 2) == 3`.
    - `verify_mode_still_checks_a_proof`: a child-process test (the `compiler_panic_exits_3` pattern in `cli/src/main.rs`): with `verify_proofs: true`, the proven `f(2147483647)` exits 101 and its stderr contains "range analysis removed the overflow check on line 1".
  - Runtime unit tests: `grown_capacity(0) == Some(4)`, `grown_capacity(MAX_ITEMS / 2 + 1) == Some(MAX_ITEMS)`, `grown_capacity(MAX_ITEMS) == None`; `items_fit(MAX_ITEMS)`, `!items_fit(MAX_ITEMS + 1)`, `!items_fit(-1)`; `proof_failed_message(PANIC_BOUNDS, 12, "world.kes")` is the spec's sentence.
  - `godot` unit test: the bridge's length test refuses `MAX_ITEMS + 1` with the list message for a list and the array message for a `[T]` (test the check as a function; don't allocate the array).
  - `tests/programs/array_too_long.kes`: `let n: i64 = 3000000000` then `array[u8](n)`; `// expect-stderr: an array can't hold more than 2,147,483,647 items; this one would have 3000000000 on line 3 of array_too_long.kes`, `// expect-exit: 101`.
  - `cli/tests/programs.rs`: `compare` reports a failure whenever stderr contains "range analysis removed" (unit-test it on a fake `Output`); `KESTREL_TEST_ONLY=a,b` keeps only programs whose names start with `a` or `b` (unit-test the filter function).
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-codegen --test proven -- --test-threads=2`. Expected: compile errors (`Proven`, `IndexCheck`, `Options` fields don't exist).
- [ ] **Step 3: Implement.** Markers and match arms first (the compiler finds the sites), then codegen's `Check`, then the runtime and the bridge. Grep the runtime and codegen for every write of a list's length (`add`, `insert`, list literals, `array[T](n)`, `split`, the lists `filter`/`map` build) and make each go through `grown_capacity`, `kestrel_rt_alloc`'s limit or `items_fit`.
- [ ] **Step 4: Verify.** The new tests pass; `cargo test -p kestrel-kir -p kestrel-codegen -p kestrel-runtime -- --test-threads=2`; `KESTREL_TEST_ONLY=array_too_long cargo test -p kestrel-cli --test programs -- --test-threads=2`; clippy on the touched crates; `godot` and `bench/kernels` build and pass clippy.
- [ ] **Step 5: Commit:** "Mark proven checks in KIR, check proofs on request, and cap lists at 2,147,483,647 items".

### Task 2: Ranges and the interpreter (overflow, shift, division and narrowing checks)

**Files:**
- New `kir/src/range/`: `mod.rs`, `interval.rs`, `sites.rs`, `show.rs`, `state.rs` (ranges only in this task), `writes.rs`, `eval.rs`, `refine.rs`, `stmt.rs`, `loops.rs`; `kir/src/lib.rs` (`pub mod range`).
- `codegen/src/lib.rs`: `define_module` runs the analysis when `opts.remove_checks`.
- Tests: new `kir/tests/range_arith.rs`, `kir/tests/range_loops.rs`, `kir/tests/range_order.rs`; `codegen/tests/proven.rs` (one more test); new programs `tests/programs/range_keep_{multiply,shift,narrow,divide,renarrow,sibling,continue}.kes`.

**Interfaces:**
- Consumes Task 1's markers, `Options.remove_checks` and the harness's proof passes.
- Produces:
  - `pub enum Arith { Checked, Wrapping }` (`Wrapping` is `--release`; `Arith::of(checked_arith: bool)`).
  - `pub fn remove_checks(m: &mut Module, arith: Arith) -> Analysis`: analyses every function with a body, marks what it proves, and returns what is left.
  - `pub struct Analysis { pub funcs: Vec<FuncChecks> }`, one per `m.funcs` (empty for natives).
  - `pub struct FuncChecks { pub left: Vec<CheckLeft>, pub removed: u32, pub gave_up: bool }`.
  - `pub struct CheckLeft { pub kind: CheckKind, pub line: u32, pub loops: Vec<u32>, pub text: String }`: `line` packed (`pack_line`); `loops` the packed lines of the enclosing loops' headers, outermost first (a `ForRange`'s `start.line`, a `While`'s `cond.line`); `text` the report wording after the kind, like "`a * b` may exceed `int` (`a` can be any `int`; `b` can be any `int`)" (evidence for each operand that isn't a constant, joined with "; "). `left` is in the order the code runs the checks.
  - `pub enum CheckKind { Overflow, Shift, Division, Narrowing, Bounds }`.
  - `sites::checks_of(e: &Expr, arith: Arith) -> Vec<CheckKind>`: the checks codegen emits for this node, mirroring `func/arith.rs` and `func/divide.rs` exactly. A literal divisor is first wrapped to its type as `divide.rs`'s `wrap_literal` does; one that wraps to 0 or (signed) -1 is a site, any other has none. In `Wrapping`, only `Division` (zero) sites remain among the arithmetic ones; `Checked` adds overflow (`u8` arithmetic included), shift and narrowing. Bounds sites are `Proj::Index` with `IndexCheck::Checked`.
  - `interval::Interval { lo: i128, hi: i128 }` with `of_type(KTy)`, `exact(i128)`, `fits(KTy)`, `clip(KTy)`, `join`, `widen(KTy)` (a growing end jumps to the type's limit), and one function per operator following the spec's table (a shift's amount clipped to 0..127 first; a `bool` cast is 0..1; `rand_int` falls back to the type's range when its joined range doesn't fit the result).
  - `show::expr_text(m: &Module, f: &Func, e: &Expr) -> String` (Kestrel syntax from local names; a nameless local is `(...)`; cut at 40 characters) and `show::range_text(i: Interval, ty: KTy) -> String`, the phrase after the value's name ("is exactly 5", "is 0 to 99", "is 0 or more", "is at most 10", "can be any `int`"; commas from five digits).
  - `writes::Write`: `Local(LocalIdx)` and `Place(Place, WriteKind)` with `WriteKind::{Whole, Items, Length(LenChange)}` and `LenChange::{Add(i64), Set(Expr), Unknown}`; `writes::of_stmt` / `writes::of_expr` take every place `visit::places` marks written as `Whole`, then refine the known list operations (`ListAdd` and `Insert` `Add(1)`, `RemoveAt` `Add(-1)`, `Clear` `Set(0)`, `SetLen` `Set`, `Compact` `Unknown`, `Sort` and `Dup` `Items`). This task acts on `Local` writes (and on `Place` writes rooted at a tracked `Addressed` local); Task 3 acts on the rest.
  - In `refine.rs`: `pub(crate) fn refine(st: &mut State, cond: &Expr, truth: bool)`, the parts of a condition in their order. This is the hook Round B's `@assume` will call. Callers wrap it in `stmt.rs`'s `condition(st, cond) -> (State, State)` (the true and false states), which applies the spec's condition rule: forget the condition's writes, refine, add the facts from its passed checks, then forget its writes again in both states before any arm, loop body or loop exit sees them. `If`, `While` and a value `If` all go through `condition`.
  - `State` lives on the heap (its facts in a `Vec` or `Box`), so recursing 256 levels never puts a 64-term matrix on the stack per level.
  - `fn budget(f: &Func) -> u32` (`max(200_000, 100 * statements)`) and a nesting limit of 256 (counted as the interpreter recurses); a function past either is left exactly as it was (`gave_up: true`, every check in `left`).

The interpreter, in the spec's words ("How the analysis works"):
- **Tracked locals** are `Value` and `Addressed` integer locals; a load through a `RefParam` local, or of any place with a path, is "any other integer load".
- **Simple statements** follow "One statement at a time": forget the expressions' writes; analyse each operand from that state, passing facts forward only along KIR's own orders; decide the checks; then add the facts from checks that passed on every path (with checks on: an overflow check's exact result, a narrowing check's operand fits, a shift check's amount is 0 to the width minus 1; with `Wrapping`: only a division's zero check), forget the writes again, and apply the statement's own effect.
- **Compound statements:** `If` refines each arm and joins; a value `If` and `Seq` likewise; `Break` carries its state to the loop's exit; `Continue` to the loop's top, through the step for a `ForRange`; `Return` nowhere; `Trap` and `Fail` end the path.
- **Loops** follow "Loops": `ForRange` as `var = start; end = <hidden snapshot>; while var < end { body; var = var + 1 (wrapping) }`; repeat to a fixed point with `widen`, then one deciding pass; a check is decided (marked, or pushed to `left`) only when every enclosing loop is in its deciding pass, and an inner loop skips its deciding pass otherwise.
- **Results** follow "After a check that isn't proven": exact (clipped) under `Checked`, the whole type under `Wrapping`. Unreachable code (a contradiction found by `refine`) decides nothing and keeps its checks.

- [ ] **Step 1: Write the failing tests.** Helper: `analysed(src, arith) -> (Module, Analysis)` (parse, check, lower, optimize, `remove_checks`), and `counts(m, a, "f") -> (proven, left)`.
  - `range_arith.rs`:
    - `the_voxel_hash_has_no_checks_left`: the voxel world's `hash` (copied from `bench/voxels3d/voxels3d.kes`): `left` is empty.
    - `a_mask_times_a_31_bit_constant_fits` / `times_a_33_bit_constant_does_not`: `(h & 0x7FFFFFFF) * 0x5BD1E995` with `h: i64` proven; `* 0x1_5BD1_E995` keeps its check.
    - `a_constant_shift_is_proven` (`x >> 3`, `1 << 5`) / `a_computed_shift_is_not` (`1 << n`) / `a_shift_by_minus_one_to_three_is_not` (`x >> n` under `if n >= -1 and n <= 3`).
    - `narrowing_a_masked_value_is_proven` (`int(h & 0x7FFFFFFF)`) / `narrowing_any_i64_is_not` / `u8_of_minus_one_to_ten_is_not` (`u8(x)` under `if x >= -1 and x <= 10`).
    - `a_small_u8_add_is_proven` (`a + b` with both `u8` under `<= 127`) / `u8_200_plus_100_is_not`.
    - `a_guarded_divisor_is_proven` (`if b > 0 { a / b }`) / `an_unguarded_one_is_not` / `a_divisor_of_minus_one_with_a_nonnegative_dividend_is_proven` (`if a >= 0 { a / -1 }`) / `a_u8_literal_divisor_that_wraps_to_zero_is_a_site` (`x / 256` on a `u8`: one `Division` left).
    - `an_inlined_constant_argument_proves_its_shift`: `octave`'s `1 << shift` inlined with 5.
    - `release_forgets_a_wrapped_value`: `if x >= 0 { let y = x + 1 \n z / y }`: `Checked` proves the division (`y` is 1 or more), `Wrapping` keeps its zero check.
    - `release_shifts_teach_nothing`: `let y = 1 << n \n z / (32 - n)` under `if n >= 0`: under `Checked` the division is proven (the shift's check taught `n <= 31`, so `32 - n` is 1 to 32); under `Wrapping` it keeps its zero check (`32 - n` can be 0).
    - `a_reference_local_sees_writes_through_its_list`: `for x in xs { if x >= 0 and x < 100 { xs[0] = 1000000000 \n s = x * 100 } }`: the multiply keeps its check (`x` is a reference into `xs`, so it has its type's range).
    - `the_left_text_names_the_ranges`: `pure f(a: int, b: int) -> int { a * b }` gives one `CheckLeft` with text "`a * b` may exceed `int` (`a` can be any `int`; `b` can be any `int`)".
    - `literal_divisors_are_not_sites`: `x / 4` gives no `CheckLeft` and counts as nothing removed.
  - `range_loops.rs`:
    - `a_counter_below_a_guard_is_proven`: `var i = 0 \n while i < n { i += 1 }`: the add is proven.
    - `an_unguarded_counter_keeps_its_check`: `var i = 0 \n while true { i += 1 \n if i > 9 { break } }`; the add stays (and `left[0].loops` is the loop's line).
    - `a_counted_loop_bounds_its_variable`: `for i from 0 to 100 { acc = i * 1000 }`: the multiply is proven; `acc += i` (accumulating) keeps its check.
    - `an_empty_loop_proves_nothing_wrong`: `for i from 5 to 0 { x = i * 1000000000 }`: the body never runs; the check stays (unreachable keeps checks).
    - `a_count_down_stays_in_range`: `var i = n \n while i > 0 { i -= 1 }` with `n: int`: the subtract is proven.
    - `continue_before_the_step_is_joined`: a `continue` path that skips `i += 1` still yields a proven add under `while i < n`.
    - `a_for_continue_takes_the_step`: `range_keep_continue.kes`'s `pick`: the division keeps its check.
    - `nested_loops_decide_in_the_final_pass`: `var k = 0 \n while k < n { for i from 0 to 10 { x = k * 100000000 } \n k += 1 }`: the multiply keeps its check, although `k` is 0 in the outer loop's first round.
    - `the_budget_gives_up_cleanly`: 40 nested `while` loops each changing a counter: `gave_up` is true and every check stays.
    - `deep_nesting_gives_up_without_overflowing_the_stack`: 300 nested `if`s, analysed on a thread with a 2 MB stack: `gave_up` is true.
  - `range_order.rs` (the spec's statement rule):
    - `a_narrowing_fact_is_about_the_old_value`: `x = i64(int(x)) + x` (with `x: i64`) then `int(x)`: the second narrowing stays.
    - `a_condition_forgets_its_own_writes`: `if x >= 0 and x < 100 and bump(var x, 1) > 0 { y = x * 10000000 }` (`bump` not inlined, returns 1 and triples `x`): the multiply keeps its check; the same with `while` and with a value `if`.
    - `a_sibling_write_wins`: `let t = half(x) + bump(var x, 1)` (`half` inlined and narrowing `x`, `bump` not inlined) then `int(x)`: stays.
    - `an_if_arm_teaches_nothing_after_it`: `if c { y = int(x) } \n z = int(x)`: the second stays.
  - `codegen/tests/proven.rs`: `define_module_removes_proven_checks`: `pure h(x: int) -> int { (x & 255) * 1000 }` has no `trap user2` with default options and one with `remove_checks: false`.
  - Programs that must still fail (one failure each, `// expect-exit: 101`, `// expect-stderr:` with the existing message and line, values from `id`/`id64`): `range_keep_multiply.kes` (`acc *= 3` thirty times in a loop), `range_keep_shift.kes` (`1 << n` with `n` 40), `range_keep_narrow.kes` (`int(x)` of an `i64` at 3,000,000,000), `range_keep_divide.kes` (`a / (n - n)`), `range_keep_renarrow.kes` (`x = i64(int(x)) + x` with `x` 2,000,000,000, then `int(x)`: "value doesn't fit in int"), `range_keep_sibling.kes` (the `half(x) + bump(var x, 1)` form, then `int(x)`), and `range_keep_continue.kes`:
    ```
    pick(n: int, k: int) -> int {
        var j = 0
        if n > 0 {
            for i from 0 to n {
                if i == k {
                    j = 1
                    break
                }
                continue
            }
            return 100 / j
        }
        0
    }
    ```
    with `main` printing `pick(id(3, 1), id(99, 1))`: "division by zero on line N" for the line of `100 / j`. (If `continue` skipped the step, the loop would seem never to end normally, `j` would seem always 1, and the division would lose its check.)
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir --test range_arith --test range_loops --test range_order -- --test-threads=2`. Expected: `kestrel_kir::range` doesn't exist.
- [ ] **Step 3: Implement** in the files above, then wire `define_module`: when `opts.remove_checks`, clone the module, run `remove_checks(&mut copy, Arith::of(opts.checked_arith))` and compile the copy. Grep the shape tests (Global Constraints).
- [ ] **Step 4: Verify.** The new tests pass; `-p kestrel-kir -p kestrel-codegen`; `KESTREL_TEST_ONLY=range_,array_too_long` programs. Update a codegen test that counts `trap user2` only where the analysis provably removed that check, saying why in the test. **Then the required proof check** (Global Constraints): every program, JIT, exe with `KESTREL_TEST_VERIFY_EXE=1` and release, all clean.
- [ ] **Step 5: Commit:** "Prove arithmetic checks with ranges and leave them out".

### Task 3: Lengths and facts between values (bounds checks)

**Files:**
- New `kir/src/range/terms.rs`; `state.rs` gains the facts; `writes.rs`'s `Place` writes take effect; `eval.rs` linear forms; `refine.rs` facts; `stmt.rs` length changes, lengths of new lists, bounds facts from passed checks, `ParFor`; `mod.rs` the `par for` body's entry facts and the pre-scan.
- Tests: new `kir/tests/range_bounds.rs`, `kir/tests/range_writes.rs`; new programs `tests/programs/range_keep_{off_by_one,pop,row,var_call,iface,remove_at,compact,reindex,get_pop}.kes`; a new folder `tests/release/` with `range_wrap_index.kes`, `range_wrap_bound.kes`, `range_wrap_shift_index.kes` and `range_wrap_divide.kes`, and a test `release_programs_run_as_executables` in `cli/tests/programs.rs` (builds each with `--release`, runs it, compares as `programs_run_as_executables` does; once more under `KESTREL_CHECK_PROOFS=1`).

**Interfaces:**
- Consumes Task 2's interpreter, `State`, `refine`, `writes`, `CheckLeft`.
- Produces:
  - `terms::Term`: `Local(LocalIdx)`, `Len(PlaceKey)`, `Hidden(u32)` (a loop's end, a fresh number each time the loop is entered); `PlaceKey` is a stable place (fields, variants, payloads, and indexes that are an integer constant or a tracked local, from any root) of a list or array, in a hashable form. No field-value terms (Global Constraints).
  - `terms::home(f: &Func, root: LocalIdx) -> LocalIdx` (following `Bind`s to the variable a loop element belongs to; a local bound by two different `Bind`s, which lowering doesn't make today, overlaps every place, with a debug assertion that it doesn't happen), `terms::may_overlap(f, written: &Place, term: &PlaceKey) -> bool` (same home; paths matching step by step, any index matching any index, two `Variant`s of one enum overlapping; the written place equal to or a prefix of the term's) and `terms::must_equal(f, written: &Place, term: &PlaceKey) -> bool` (same root, same path, the same index locals). A `Length` write moves only the terms that `must_equal`; every other term that `may_overlap` is forgotten.
  - `state::Facts`: closed `a - b <= c` over at most `MAX_TERMS = 64` terms chosen by `terms::pick(f: &Func) -> Vec<Term>` (the spec's pre-scan), with `add(a, b, c)`, `forget(term)`, `shift(term, by)`, `join`, `widen`, `implies(a, b, c) -> bool`; ranges and facts tighten each other after every change; a fact whose constant passes 2^66 in size is dropped.
  - Bounds facts from a passed check (`0 <= i`, `i < P.len`) join Task 2's step-4 list, in both builds.
  - The `par for` body (`m.par_bodies`) starts with `0 <= start <= end <= xs.len`, where `xs` is the body's list parameter: the `ParFor`'s `count` is `Len(P)`, and `P` with its root renamed to the matching parameter is the body's list (`P` may be a temporary the loop made). `start` and `end` are the body's last two parameters.

- [ ] **Step 1: Write the failing tests.**
  - `range_bounds.rs` (each proof with its near miss):
    - `a_counted_loop_over_len_proves_its_index` (`for i from 0 to xs.len { s += xs[i] }`) / `the_next_index_is_not` (`xs[i + 1]` stays) / `off_by_one_is_not` (`0 to xs.len + 1`).
    - `a_guard_that_breaks_proves_the_rest_of_the_body` (`while true { if i >= xs.len or i < 0 { break } \n s += xs[i] \n i += 1 }`: proven) / `the_facts_after_a_break_are_the_breaks` (after that loop, `xs[i]` stays).
    - `a_while_guard_proves_its_index` (`while i < xs.len { s += xs[i] \n i += 1 }`) / `a_guard_with_and_proves_it` (`if i >= 0 and i < xs.len { xs[i] }`) / `a_guard_without_the_lower_end_does_not` (`if i < xs.len { xs[i] }` with `i: int`).
    - `pop_proves_its_own_check`: `xs.pop()`'s internal `xs[n - 1]` is proven.
    - `the_draw_copy_is_proven`: `kernels/copy.kes`'s `bench`: `positions[i]` is proven.
    - `a_later_statement_reuses_a_passed_check` (`parts[i].x = 1 \n parts[i].y = 2`: the second is proven) / `the_same_statement_does_not` (`s = xs[i] + xs[i]`: both stay) / `the_right_of_and_does_not` (`if c and xs[i] > 0 { } \n xs[i]`: both stay).
    - `a_new_array_knows_its_length` (`let a = array[int](10) \n a[9]` proven, `a[10]` stays).
    - `rand_int_inside_the_list_is_proven` (`xs[rand_int(0, xs.len - 1)]` under `if xs.len > 0`).
    - `a_par_for_body_knows_its_chunk`: `par for i, p in ps { qs[i] }` keeps `qs[i]`; the body's conversions of `start` and `end` to `int` are proven; the same with `par for p in make()` (a temporary list).
    - `an_inner_loop_entered_again_has_a_fresh_end`: `for k from 0 to m { for i from 0 to xs.len { s += xs[i] } \n xs.pop() }`: `xs[i]` proven (each entry's end is that entry's `xs.len`).
    - `a_promoted_field_forgets_after_its_change`: in a `var self` method, `if self.n > 0 and self.n <= xs.len { self.bump() \n s = xs[self.n - 1] }` with `bump` raising `n` (not inlined): stays.
    - `len_conversions_are_proven`: `xs.len` read as an `int` has no narrowing check left; a string's `s.len` keeps its own.
  - `range_writes.rs` (each must keep the check that a fact would have removed):
    - `pop_in_the_loop_forgets` (`for i from 0 to xs.len { xs.pop() \n xs[i] }`).
    - `a_var_call_forgets` (`if i < xs.len and i >= 0 { shrink(var xs, 1) \n xs[i] }`, where `shrink(var xs: list[int], n: int)` pops and calls itself, so it isn't inlined) / `an_unrelated_var_call_keeps` (`shrink(var ys, 1)`: proven).
    - `a_loop_elements_list_forgets` (`for row in grid { grid[0] = [] \n row[0] }` after `if row.len > 0`).
    - `a_lent_interface_forgets` (`heal(var g)` through `var d: Damageable` between a guard on `g.items.len` and `g.items[0]`).
    - `remove_at_forgets` / `compact_forgets` (`let n = xs.len`, a `remove b` loop, then `xs[n - 1]`: stays) / `sort_keeps_the_length` / `add_shifts_the_length` (`let n = xs.len \n xs.add(1) \n xs[n]` proven).
    - `a_length_moves_only_for_the_same_list`: `for a in grid { for b in grid { let n = b.len \n a.add(0) \n s = b[n] } }`: stays (`b.len` is forgotten, not moved).
    - `a_map_write_forgets_its_place` (a list inside a map value's struct, guarded, then `m[k] = v`, then indexed: stays).
    - `assigning_an_index_variable_forgets_its_term` (`grid[y].len` after `y += 1`).
    - `a_par_for_forgets_inner_lists` (`ps[0].trail.len` before and after a `par for` that adds to `p.trail`).
    - `a_check_on_the_old_index_says_nothing_about_the_new` (`i = xs[i]` then `xs[i]`: stays).
    - `a_condition_that_pops_after_its_guard_forgets` (`if i >= 0 and i < xs.len and pop_one(var xs) > 0 { s = xs[i] }`: stays; the same as a `while` condition).
    - `a_check_before_a_shrinking_call_in_the_same_statement` (`s = xs[i] + shrink_get(var xs)` then `xs[i]`: stays) / `an_inlined_get_and_a_pop_in_one_expression` (`t = get(xs, i) + pop_one(var xs)` with `get` inlined, then `xs[i]`: stays; and `get(xs, i) + pop_one(var xs) + xs[i]`: the last stays).
  - Programs that must still fail, with the existing index message ("index N out of bounds for length M on line L of f.kes") and `// expect-exit: 101`: `range_keep_off_by_one.kes`, `range_keep_pop.kes`, `range_keep_row.kes`, `range_keep_var_call.kes`, `range_keep_iface.kes`, `range_keep_remove_at.kes` (a `while` loop that removes and indexes), `range_keep_compact.kes` (`let n = xs.len`, a `remove b` loop that removes items, then `xs[n - 1]`), `range_keep_reindex.kes` (`xs = [5]`, `i = 0`, `i = xs[i]`, then `xs[i]`), `range_keep_get_pop.kes` (the inlined `get(xs, i) + pop_one(var xs)` form, then `xs[i]`), `range_keep_cond_pop.kes` (`xs = [1, 2, 3]`, `i = 2` from `id`, then `if i >= 0 and i < xs.len and pop_one(var xs) > 0 { print(xs[i]) }`: "index 2 out of bounds for length 2 ...").
  - `tests/release/` (each also under the proof check, and each guarded so that a rule keeping a wrapped value's linear form would prove the index): `range_wrap_index.kes` (`i` is 2,147,483,647 from `id`, `let j = i + 1`, then `if i >= 0 and j < xs.len { xs[j] }`: "index -2147483648 out of bounds for length 3 on line ..."); `range_wrap_bound.kes` (`n` is -2,147,483,648 and `i` 5, both from `id`, then `if n <= xs.len and i >= 0 and i < n - 1 { xs[i] }` with `xs.len` 3: fails as an index error); `range_wrap_shift_index.kes` (`let y = 1 << n` with `n` -5 from `id`, then `if n < xs.len { xs[n] }`: "index -5 out of bounds ..."); `range_wrap_divide.kes` (a divisor that wraps to 0: "division by zero on line ...").
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir --test range_bounds --test range_writes -- --test-threads=2`. Expected: the bounds proofs fail (no facts yet); the forgetting tests pass trivially and must still pass after Step 3.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify.** The new tests and Task 2's pass; `-p kestrel-kir -p kestrel-codegen`; `KESTREL_TEST_ONLY=range_` programs; the release target. Check `codegen/tests/dump.rs`'s `a_struct_store_into_a_list_checks_the_index_once` still counts one (nothing proves `dst.len`). **Then the required proof check** over every program in all three passes, all clean.
- [ ] **Step 5: Commit:** "Prove bounds checks from lengths and facts between values".

### Task 4: `kestrel check --report`

**Files:**
- `kir/src/ir.rs`: `Func::lines: (u32, u32)` (the declaration's first and last packed lines; `(0, 0)` for generated functions), set in `kir/src/lower/mod.rs` (closure code functions get their literal's lines); every other `Func { .. }` construction gets `(0, 0)`. `Module::not_inlined: Vec<NotInlined>`.
- `kir/src/inline.rs`: `inline_small` records each call it leaves in place, with its reason.
- New `kir/src/range/report.rs` and `render.rs`.
- `cli/src/main.rs`: `--report`, its usage error, and printing.
- Tests: new `kir/tests/range_report.rs`, new `cli/tests/report.rs`.

**Interfaces:**
- Consumes Task 2's `Analysis` (with Task 3's bounds items).
- Produces:
  - `pub struct NotInlined { pub caller: FuncIdx, pub line: u32, pub callee: FuncIdx, pub reason: String }`, the reason one of the spec's ("it calls itself", "it is N nodes; the limit is 256", "it has an early `return`", "the inlining budget of `caller` ran out", "inlined calls nest more than 8 deep"), recorded where the inliner decides.
  - `report::build(m: &Module, a: &Analysis, arith: Arith) -> Report`.
  - `pub struct Report { pub funcs: Vec<FuncReport>, pub checks: u32, pub removed: u32, pub left: u32, pub allocations: u32, pub fails: u32 }`.
  - `pub struct FuncReport { pub func: FuncIdx, pub name: String, pub lines: (u32, u32), pub never_called: bool, pub gave_up: bool, pub items: Vec<Item>, pub par_bodies: Vec<(u32, FuncReport)> }` (a `par for` body under its function, keyed by the `ParFor`'s `count.line`).
  - `pub struct Item { pub line: u32, pub loops: Vec<u32>, pub kind: ItemKind, pub text: String }`, `pub enum ItemKind { Check(CheckKind), Allocation, CanFail, Call }`.
  - `report::Summary { pub allocates: Option<Chain>, pub fails: Option<Chain>, pub checks_left: u32 }` per function, computed to a fixed point over the call graph (a cycle allocates if any function in it does); `Chain` holds the first reason and its line, and the callee names and lines leading to it, for "calls `spawn` (line 40), which adds to `ps` (line 12) and may grow the list".
  - `render::render(r: &Report, m: &Module, user_file: &dyn Fn(u32) -> bool) -> String`: the spec's format. "(in `f`)" comes from `Func::lines` (the innermost function whose lines hold the item's line; a closure's code is inside its enclosing function's lines). Functions in files where `user_file` is false (built-in modules) are left out. A generated function is named by the built-in it implements.
- The allocation and failure items come from one walk over each function. Check each candidate against its codegen arm and the runtime before listing it. Allocations: `ArrayNew`; a non-empty `ArrayLit`; `Format`; the `Str` operations that build a string; `ListAdd` and `List(Insert)` ("`add` may grow `ys`"); `MapSet`; `List(Sort)`; `Dup`; `MakeFn` with a heap environment, `EnvDup`, `CopyFnEnv`; `MakeIface` boxed, `BlockDup`, `CopyIface`; `NewArena`, `PushArena`; `ParFor` (an arena per thread: `runtime/src/par.rs`, `participate`); calls (by summary), natives, `CallIndirect`, `CallVirtual`. Failures: `List(RemoveAt | Insert)`, `Str(At | Slice)`, `Str(Split)` (an empty separator), `ListAdd` and `List(Insert)` past the item limit ("`add` fails if `ys` would hold more than 2,147,483,647 items"), the built-ins that build lists (through their generated functions or runtime calls), `MapValue` checked, `ArrayNew`, natives, `CallIndirect`, `CallVirtual`, `Trap`, `Fail`.

- [ ] **Step 1: Write the failing tests.**
  - `range_report.rs`: `a_filter_is_an_allocation` ("`filter` builds a new list"); `add_is_an_allocation_and_a_way_to_fail`; `sort_allocates`; `a_par_for_allocates_per_thread`; `a_chain_follows_calls` (`main` calls `spawn` (not inlined) which adds: the item's text is the spec's chain sentence); `a_recursive_function_that_allocates_nothing_is_clean`; `a_function_value_call_is_unknown` ("calls the function value `f` (line 8); its body isn't known here"); `a_big_callee_says_why_it_isnt_inlined` (the recorded reason); `a_function_inlined_everywhere_is_never_called`; `remove_at_and_slice_can_fail`; `an_item_in_a_closure_names_the_closures_function`.
  - `cli/tests/report.rs` (runs `kestrel check --report` on files written to `CARGO_TARGET_TMPDIR`):
    - `the_report_matches_exactly`: this program as `report_fixture.kes` (line numbers matter; lines 8 and 14 are empty):
      ```
      pub total(xs: list[int], k: int) -> int {
          var s = 0
          for i from 0 to xs.len {
              s += xs[i]
          }
          s + xs[k]
      }

      pub grow(var ys: list[int], n: int) {
          for i from 0 to n {
              ys.add(i * 2)
          }
      }

      main() {}
      ```
      and the whole output compared as one string:
      ```
      report_fixture.kes
        function `total` (line 1): 3 checks left (2 removed), no allocations
          loop at line 3
            line 4: overflow check: `s + xs[i]` may exceed `int` (`s` can be any `int`; `xs[i]` can be any `int`)
          line 6: bounds check: `xs[k]` may be outside `xs` (`k` can be any `int`)
          line 6: overflow check: `s + xs[k]` may exceed `int` (`s` can be any `int`; `xs[k]` can be any `int`)
        function `grow` (line 9): 1 check left (0 removed), 1 allocation, 1 other way to fail
          loop at line 10
            line 11: overflow check: `i * 2` may exceed `int` (`i` is 0 to 2,147,483,646)
            line 11: allocation: `add` may grow `ys`
            line 11: can fail: `add` fails if `ys` would hold more than 2,147,483,647 items
        function `main` (line 15): no checks, no allocations
      total: 6 checks, 2 removed, 4 left; 1 allocation; 1 other way to fail
      ```
      (The two removed checks are `int(xs.len)` and `xs[i]`.) If the code is right and this differs, the rule behind the difference decides: fix the code, or fix the spec, this plan and the test together.
    - `release_reports_only_release_checks`: the same file with `--release`: no overflow items; the bounds item on line 6 stays.
    - `report_needs_check`: `kestrel run --report f.kes` exits 2 and prints "`--report` goes with `check`: kestrel check --report file.kes".
    - `a_program_with_errors_gets_no_report`: exit 1, empty stdout.
    - `the_limits_are_reported`: the spec's "range analysis stopped early in `f` (its code nests too deeply), so every check in it stays".
    - `an_item_in_another_file_names_it`: an inlined helper from `geom.kes` prints as `geom.kes:N (in `helper`)`.
    - `an_inlined_item_names_its_function`: "line N (in `density`)".
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir --test range_report -- --test-threads=2` and `cargo test -p kestrel-cli --test report -- --test-threads=2`. Expected: missing items and an unknown flag.
- [ ] **Step 3: Implement.** The CLI's `check --report` runs `kestrel_driver::compile`, then `remove_checks` on the module with `Arith::of(!release)`, then `report::build` and `render::render`, with `user_file` true for the files whose path isn't a built-in module's.
- [ ] **Step 4: Verify.** The new tests; `-p kestrel-kir` (`kir/tests/inline.rs` still passes with the recording); the cli `report`, `errors` and `modules` targets; clippy.
- [ ] **Step 5: Commit:** "Add kestrel check --report".

### Task 5: Measure and document

**Files:**
- `kestrel/bench/kernels`: a new `step_index.kes` (`step.kes`'s step in `for i from 0 to parts.len { parts[i].step(dt, target, keep) }`), its line in `measure.py`, a `--keep-checks` flag in `src/main.rs` (sets `remove_checks: false`), and `results.md`.
- `kestrel/bench/voxels3d/results.md`.
- `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md`, `README.md`, `docs/superpowers/specs/2026-10-18-at-annotations-design.md` (status line, and the block-end question for Round B), `docs/superpowers/plans/2026-10-09-core-2a-followups.md`, `docs/superpowers/plans/2026-10-10-handoff.md` ("Start here"), and the spec's "As built".

- [ ] **Step 1: The voxel world.** Build the compiler at the commit before Task 1 in a worktree at `C:\kt\ra-old` (the long-path limit), and run `bench/voxels3d/run.ps1`'s builds with old and new taking turns, checked and `--release`, 7 runs each, medians and ranges. Record the checked/`--release` ratio against the spec's target (at most 1.2x), and `kestrel check --report` totals (checked and `--release`) for `voxels3d.kes`. Confirm the report shows no check left in the inlined `hash`, `lattice` and `octave` code; if it does, say which and why.
- [ ] **Step 2: The kernels.** `py measure.py --rounds 7 --threads 1` with and without `--keep-checks` (taking turns): `copy.kes` has no bounds check in its loop (`--dump bench`: no `trap user2` there) and is faster; `step_index.kes`'s loop has no bounds check, with its gain against `--keep-checks` and its distance from `step.kes`; no other kernel slower beyond noise (re-check any difference under 10% with an unused function added). Also `bench/2a` and `bench/heightmap`, old against new.
- [ ] **Step 3: Compile time.** `kestrel build` of the voxel world and the particles demo, old against new, 7 runs: the analysis adds under 10%. Record the report's totals (checks, removed, left, allocations) for the voxel world, the particles demo and the kernels.
- [ ] **Step 4: Docs,** each in its file's style, with the honest numbers, including misses. The follow-ups file closes "Range analysis for overflow checks" and the bounds half of "Bounds checks and list-header reloads", and adds what this round found (at least: hoisting loop-invariant checks, facts across calls, products in facts, fields in memory as terms, the header reload).
- [ ] **Step 5: The full suite,** if the user allows it: `cargo test --workspace --no-fail-fast` (including the exe pass without the proof check) and `cargo clippy --workspace --all-targets -- -D warnings`, plus clippy in `kestrel/godot`; otherwise record it as owed in the handoff.
- [ ] **Step 6: Commit:** "Measure range analysis and document it".
