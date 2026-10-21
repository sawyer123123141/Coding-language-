# Round B Implementation Plan: `@prove` and `@assume`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Build on `main`, with the ledger under `.superpowers/sdd/2026-10-20-round-b-annotations/`.

**Revised 2026-10-20 after the Opus plan review** (`.superpowers/sdd/2026-10-20-round-b-annotations/plan-review.md`: 3 Critical, 6 Important, 22 Minor) and the controller's rulings (`progress.md`, "Plan review"). Every finding is folded in; the finding's tag (C1, I3, M12 ...) is written where it lands, so a reviewer can trace it.

**Goal:** Parse `@prove(no_alloc, no_checks, no_panic, inline)` and `@assume(cond)`; check assumptions at run time and let range analysis use them inside their block; fail the build when a contract on a function, method, interface method or loop doesn't hold in any compiled copy of it, with the call chain in the error.

**Architecture:** `@` is a new token. `@prove` is kept beside the syntax tree (`Program::proves`, keyed by the target's span); `@assume` is a statement. Lowering puts the rest of an assumption's block inside a new KIR block `Stmt::Assume`, and every statement an annotated loop lowers to inside a no-op block `Stmt::Prove`; the inliner wraps each expanded copy of a contracted function in a `Stmt::Prove` too; `Module::contracts` lists every contract with its target. Range analysis runs an assumption's block twice in a deciding pass (with it, and as if no assumption in the block were written; what leaves the block comes from the second run) and marks, per statement, which assumptions `--release` may trust. A driver step (`kestrel_driver::prove`) runs range analysis once, in place, when a program has annotations, gives the "removes no check" warning (`kestrel check` only) and calls `range::contracts::check`, which reads the analysis and the report's walk and summaries; codegen reuses that analysis.

**Tech Stack:** Rust, Cranelift 0.116. Tests are Rust unit tests plus `.kes` program, error, release and CLI tests.

**Spec:** `docs/superpowers/specs/2026-10-18-at-annotations-design.md`, with its "Round B design addendum (2026-10-20)" (revised with this plan), on top of Round A (`docs/superpowers/specs/2026-10-19-range-analysis-design.md`, "As built"). Every message, rule and number comes from them; the spec wins any conflict. **Two syntax picks wait for the user** (the addendum's "Syntax pick (1)" and "(2)"): build them as written, keep each in one place in the parser or checker so a change is cheap, and don't build on them elsewhere.

## Global Constraints

- **Soundness first.** Critical bugs: a removed check that could fire; an assumption used outside its block; an assumption statement trusted under `--release` while a bounds check rests on it; a contract that passes while one of its compiled copies has an item it counts. Where a rule is unclear, keep the check, keep the assumption evaluated, fail the contract.
- **Messages,** exactly (the addendum's "Lexing, parsing ...", "`@assume`", "The warning", "What a contract covers", "Error messages"):
  - parse: "unknown annotation `@fast`; Kestrel has `@prove` and `@assume`"; "unknown contract `fast`; `@prove` takes `no_alloc`, `no_checks`, `no_panic` and `inline`"; "expected an annotation name after `@`, like `@prove` or `@assume`" (also for `@ prove`: the name must start where `@` ends, M7); "`@prove` needs at least one contract, like `@prove(no_alloc)`"; "`@prove` goes before a function, a method or a loop (`for`, `par for` or `while`)"; "`@prove` goes on the line right before what it applies to, or on the same line"; "`@prove` can't go on a native function"; "`@assume` needs a condition, like `@assume(i < xs.len)`"; "`@assume` goes inside a function, as a statement of its own".
  - check: "`@assume` takes comparisons of integers joined with `and`, like `@assume(i < xs.len and n >= 0)`"; "`@assume` can't call a function or index a list (a `--release` build may not run it); compare names, fields, numbers and `.len`".
  - run time: "assumption `n <= xs.len` is false on line 9 of f.kes" (the condition's text as written; no " of f.kes" without a file).
  - warning: "`@assume(n <= xs.len)` removes no check; delete it".
  - report: a `can fail` item "the assumption `n <= xs.len` is checked here".
  - contracts: "{target} can't be proven `{contract}`: {reason}", target "`step`", "`Goblin.area`" (a method) or "the loop at line 12"; in an inlined copy "{target} can't be proven `{contract}` where it is inlined into `step` (line 40): {reason}" (a loop's copy names the function only: "... where it is inlined into `step`: ..."); an implementation of an interface method: "`Goblin.area` can't be proven `no_alloc`, which `Shape.area` requires: {reason}". Reasons: "it has a bounds check on line 14: {check text}" (`an overflow`, `a shift`, `a division`, `a narrowing`, `a bounds`); "it allocates on line 13: {text}"; "it can fail on line 9: {text}"; "it calls `big` (line 7), which isn't inlined: {inliner's reason}"; "it calls the function value `f` (line 8), which can't be inlined"; "it calls a method of the interface value `s` (line 8), which can't be inlined"; through calls "it " + the report's chain text ("it calls `spawn` (line 40), which adds to `ps` (line 12) and may grow the list"; for a check "..., which has a bounds check (line 9): `xs[i]` may be outside `xs` (`i` can be any `int`)"); an unknown call "it calls the function value `f` (line 8); its body isn't known here", "it calls a method of the interface value `s` (line 8); its body isn't known here", "it calls the native function `draw.clear` (line 4); its body isn't known here"; recursion "it calls `walk` (line 5), which can call itself (line 9), so the stack may run out" and its own "it can call itself (line 9), so the stack may run out"; give-up "range analysis stopped early in it (its code nests too deeply), so nothing can be proven about it; split it into smaller functions" (or "it took too long to analyse"), through a call "it calls `g` (line 5), where range analysis stopped early (its code nests too deeply)"; `main`'s constants "it builds the program's constants first, which ..."; a lost loop "`@prove` lost track of the loop at line 12; this is a compiler bug".
  - contract help lines: "`kestrel check --report` lists everything left in it" (default); "add `@prove(no_alloc)` to the interface method `Shape.area`; then every implementation must keep it" (an unpromised interface call); "contracts can't see into a function value yet; call a named function or an interface method instead" (a function value); "a `par for` makes an arena for each thread; put `@prove(no_alloc)` on the function its body calls" (a `par for`'s arenas, M18).
- **Numbers and names:** contracts `no_alloc`, `no_checks`, `no_panic`, `inline`; `ContractSet` bits `NO_ALLOC = 1`, `NO_CHECKS = 2`, `NO_PANIC = 4`, `INLINE = 8`; inserting `NO_PANIC` also inserts `NO_CHECKS`. Round A's limits are unchanged: `MAX_TERMS = 64`, nesting 256 in the analysis, `MAX_DEPTH = 4096` in the events walk; the statement budget stays `max(200_000, 100 * statements)`, with each statement counted once more for every `Assume` around it (I4).
- **File sizes:** under about 400 lines; split before passing it. Already over or near it: `kir/src/range/events.rs` (448: Task 1b moves its statement arms to `events/stmts.rs` first), `range/report.rs` (407: don't grow it), `range/mod.rs` (397: Task 2 moves `Marker` to `range/mark.rs` first), `range/stmt.rs` (359: new arms only delegate), `range/writes.rs` (346: new arms only delegate). New code goes in new files named in each task.
- **New statements, every walker:** `ast::Stmt::Assume` gets an arm in every exhaustive AST match (`syntax/src/link/walk.rs`, `sexpr.rs`, `check/src/body/{stmt,nested,remove,consts}.rs`, `kir/src/lower/stmt.rs`; the compiler lists any others). KIR's `Stmt::Assume { conds, check, body }` (Task 1b) and `Stmt::Prove { id, at, body }` (Task 3) get an arm in every exhaustive KIR `Stmt` match (`kir/src/visit.rs`, `visit_mut.rs`, `rename.rs`, `inline/descend.rs`, `lower/lines.rs`, `lower/sort.rs`, `promote.rs`, `split.rs`, `range/stmt.rs`, `range/writes.rs`, `range/events`, codegen `func/stmt.rs`; the compiler lists others). **Their bodies are treated exactly like an `If` arm: same loop depth** (in `promote.rs`, a `break`, `continue` or `return` inside one still gets its write-backs). `Assume`'s conditions are expressions every walker visits.
- **Runtime functions** need an `RtFuncs` field and `decl` in `codegen/src/rt.rs`, a `jit_symbols()` entry, and `#[no_mangle]`; one that dereferences a pointer argument is `pub unsafe extern "C"` like `kestrel_rt_fail` (M4); a failing one fails through `panic::fail`.
- **Never a bare `link.exe`:** Git's coreutils `link.exe` shadows the MSVC linker on PATH (`find-msvc-tools` finds the real one).
- **LIGHT MODE testing** (the user's machine lags under load):
  - Every cargo command runs with `CARGO_BUILD_JOBS=2` (PowerShell: `$env:CARGO_BUILD_JOBS=2`) and tests with `-- --test-threads=2`.
  - Run focused targets only: `-p <crate>`, a named `--test`, a test-name filter, and the programs target filtered with `KESTREL_TEST_ONLY` (comma-separated file-name prefixes).
  - Run long commands at idle CPU priority under a watcher that waits for them to end; don't poll in a loop.
  - No window, GPU or demo tests (`window_smoke*`, `voxels_demo`, `demos`, `window_print_flush`).
  - **The full `cargo test --workspace --no-fail-fast` runs only in Task 5, and only when the user says the machine is free.** Until then it is owed, and every task report says so.
  - Clippy per task on the crates touched: `cargo clippy -p <crate> --all-targets -- -D warnings`. `kestrel/godot` and `kestrel/bench/kernels` are separate workspaces: `cargo check` and clippy them when a task touches an API they use.
- **Test facts:**
  - Program tests: `kestrel/tests/programs/*.kes`. **Expectation lines go at the end of the file** (M6), so the line numbers in this plan count from the top as the program is written. The JIT pass runs with `KESTREL_CHECK_PROOFS=1`, the exe pass also with `KESTREL_TEST_VERIFY_EXE=1`, and the release pass (`programs_verify_release`, `kestrel run --release` with the proof check) fails on "range analysis removed" and, from Task 4, on "can't be proven" (M13: every `prove_*` program must hold in both builds).
  - Release tests: `kestrel/tests/release/*.kes`, run by `release_programs_run_as_executables` in `cli/tests/programs/release.rs`: built with `--release`, run, compared, then again under `KESTREL_CHECK_PROOFS=1`. The `expectations` parser is in `cli/tests/programs.rs` and panics on an unknown `// expect...` directive (M5).
  - Error tests: `kestrel/tests/errors/*.kes` with `// error <line>: <message>` lines (`<line>` is where the error points: for a contract, the `@prove` line; the item's own line is inside the message, M21; `<message>` is a substring), run through `kestrel check --diagnostics=short`, which must exit 1. That is the checked build; `--release` contract tests go in `cli/tests/contracts.rs`.
  - `kestrel check` needs a `main() {}` in every program it is given (M6).
  - No string literals inside `{...}` in program sources.
  - To keep a value unknown to the analysis, get it from a helper that isn't inlined: `id(x: int, d: int) -> int { if d == 0 { return x } \n id(x, d - 1) }`.
  - `kestrel_codegen::dump` prints a function's machine code; each check's failure path ends in `trap user2`.
  - KIR tests reuse `kir/tests/range_common/mod.rs` (`optimized`, `analysed`, `checks`, `counts`, `left_of`, `proven_of`, `line`).
- **Off-limits:** `kestrel/test.kes`, `voxels.exe`, `kestrel/b.exe`, the line-ending-only change in `kestrel/check/src/body/enums.rs`, and anything under `kestrel/bench/voxels3d/` or in `docs/superpowers/plans/2026-10-09-core-2a-followups.md` that another agent left uncommitted. Never use `git add -A`, `git stash` or `git clean`; stage files by name.
- **Commits:** plain-sentence messages ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`, on `main`; the controller pushes.

## Review Focus

1. **An assumption seen outside its block:** through its end, a `break`, a `continue` into the next iteration, an `if` arm joined with one that never ran it, a loop's back edge. (Task 2.)
2. **A trusted assumption under `--release` that a bounds check rests on:** directly, through a nested pair that each suffice, through two copies of one assumption in one function (C3), through an inlined copy, through a function the analysis gave up on. Each must stay evaluated. (Task 2.)
3. **An assumption proving more than `if cond` would:** a contradiction (unreachable block), a wrapped side under `--release`, a field in memory, a changed variable. (Task 2.)
4. **A contract that misses an item:** an inlined copy (I1), a header (`for x in xs.filter(...)`), a `par for` body, a loop inside a `par for` body (C2), a call left, a cycle (`no_panic`, I2), a closure made in the region, a native, a function value or interface call below a call (C1), an implementation of a promised interface method, `main`'s constants, a function the analysis gave up on, cut or left undecided (M2). (Tasks 3 and 4.)
5. **A program without annotations that changed:** its build time, its report, its code. (Tasks 3 and 5.)

---

### Task 1a: The `@` syntax and the checker — **Sonnet**

**Goal:** `@prove` and `@assume` parse with the addendum's placement rules and errors; the checker checks an assumption's condition; `@prove` is recorded (and decided from Task 4). Lowering skips `@assume` until Task 1b (the next commit): an assumption has no effect in this commit.

**Files:**
- `syntax/src/token.rs` (`Tok::At`, symbol `@`), `lexer.rs` (`'@'`), `ast.rs` (`pub enum Contract { NoAlloc, NoChecks, NoPanic, Inline }` with `name()`; `pub struct ProveAnnot { pub target: Span, pub contracts: Vec<(Contract, Span)>, pub span: Span }`; `Program::proves: Vec<ProveAnnot>`; `Stmt::Assume { cond: Expr, text: String, span: Span }` and its `span()`), new `parser/annot.rs` (reading `@name(...)`; the name must start where `@` ends; the errors; attaching with at most one line break before the target), `parser/mod.rs` (the parser keeps the file's source text for `text`, and collects `proves`), `parser/item.rs`, `parser/members.rs`, `parser/iface.rs` (annotations before items, methods and interface methods), `parser/stmt.rs` (`@assume` statements; `@prove` before `for`, `par for`, `while`), `lib.rs` (`parse_at` hands the source to the parser), `link.rs` (concatenates `proves`), `link/walk.rs`, `sexpr.rs` (`(assume <cond>)`).
- `check/src/body/assume.rs` (new: the condition's form, the two checker errors), `body/stmt.rs` (type-check `cond` as `bool`, then `assume.rs`), the other exhaustive AST matches; `kir/src/lower/stmt.rs` (the `Assume` arm lowers nothing yet, with a comment naming Task 1b).

- [ ] **Step 1: Write the failing tests.**
  - `syntax/tests/lexer.rs`: `at_is_a_token` (`@prove` lexes as `At`, `Ident("prove")`, with adjacent spans).
  - New `syntax/tests/parser_annot.rs`:
    - `prove_before_a_function_is_recorded`: `"@prove(no_alloc, no_checks)\npure step(x: int) -> int { x }\n"`: one `ProveAnnot`, `target` equal to the function's span, contracts `[NoAlloc, NoChecks]`.
    - `prove_goes_before_pub_methods_and_interface_methods`: `@prove(inline)` before `pub f()`, a struct method, an enum method and an interface method: four entries, each targeting that `FnDecl`'s span.
    - `prove_goes_before_loops`: `for`, `par for` and `while` in a body: three entries targeting the statements' spans.
    - `annotations_stack_and_merge`: `"@prove(no_alloc) @prove(inline)\nf() {}"` and the same one per line: the target's contracts are `[NoAlloc, Inline]`; a name listed twice counts once.
    - `the_target_may_follow_on_the_same_line` (syntax pick 1): `"@prove(inline) pure f() {}"` and `"@prove(no_checks) for i from 0 to n {}"` parse.
    - `a_blank_line_before_the_target_is_an_error`: `"@prove(inline)\n\nf() {}"` → "`@prove` goes on the line right before what it applies to, or on the same line".
    - `prove_on_a_native_is_an_error` (M8; only built-in modules reach it, so a parser unit test): `"@prove(no_alloc)\nnative f()"` → "`@prove` can't go on a native function".
    - `assume_is_a_statement`: the body of `f(n: int, xs: [int]) { @assume(n <= xs.len) }` prints `{(assume (<= n (. xs len)))}`, and `text == "n <= xs.len"`; whitespace runs inside are one space.
    - `two_assumes_share_a_line`: `@assume(a > 0) @assume(b > 0)` is two statements.
  - Error tests (one file per bullet; split further if parse recovery mixes them):
    - `tests/errors/annot_names.kes`: `@fast` before a function → "unknown annotation `@fast`; Kestrel has `@prove` and `@assume`"; `@prove(fast)` → "unknown contract `fast`; ..."; `@ prove(no_alloc)` → "expected an annotation name after `@`, like `@prove` or `@assume`".
    - `annot_prove_empty.kes`: `@prove` and `@prove()` → "`@prove` needs at least one contract, like `@prove(no_alloc)`".
    - `annot_prove_target.kes` (M15): before a `let`, an `if`, a `match`, a closure literal (`let f = @prove(no_alloc) (x: int) -> int { x }` and on the line before `let f = ...`), a struct, a constant, and an `@assume` (`@prove(no_alloc)` then `@assume(n > 0)` then a loop) → "`@prove` goes before a function, a method or a loop (`for`, `par for` or `while`)".
    - `annot_assume_place.kes`: a top-level `@assume(1 < 2)` and one in a struct body → "`@assume` goes inside a function, as a statement of its own"; `@assume` and `@assume()` in a body → "`@assume` needs a condition, like `@assume(i < xs.len)`".
    - `annot_assume_form.kes`: `@assume(a < b or b < c)`, `@assume(flag)`, `@assume(x < 1.5)` → "`@assume` takes comparisons of integers joined with `and` ..."; `@assume(id(n, 1) < 3)`, `@assume(xs[0] < 3)` and `@assume(int(big) < 3)` → "`@assume` can't call a function or index a list ..."; `@assume(nope < 3)` → the usual unknown-name error; `@assume(n < LIMIT)` with `let LIMIT = 100 / 3` is accepted (a constant, M11).
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-syntax --test parser_annot --test lexer -- --test-threads=2`. Expected: `Tok::At`, `Program::proves` and `Stmt::Assume` don't exist.
- [ ] **Step 3: Implement.** Token and AST first (the compiler finds the match arms), then the parser, then the checker.
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-syntax -p kestrel-check -- --test-threads=2`; `cargo test -p kestrel-cli --test errors -- --test-threads=2`; `cargo test -p kestrel-kir -- --test-threads=2` (lowering still compiles every program); clippy on `syntax`, `check`, `kir`.
- [ ] **Step 5: Commit:** "Parse @prove and @assume and check assumption conditions".

### Task 1b: `@assume` lowered and checked at run time — **Opus**

**Goal:** `@assume` lowers to `Stmt::Assume` holding the rest of its block and stops the program when false. Range analysis treats it as a plain block that teaches nothing (Task 2 changes that).

**Files:**
- KIR: `kir/src/ir_stmt.rs` (`Stmt::Assume { conds: Vec<(u32, Expr)>, check: AssumeCheck, body: Vec<Stmt> }`, `pub enum AssumeCheck { Check, Trust }`), `ir.rs` (`pub struct AssumeInfo { pub text: String, pub span: Span, pub line: u32 }`, `Module::assumptions: Vec<AssumeInfo>`, by each condition's `id`), new `lower/assume.rs` (the rest of a block inside the assumption; consecutive assumptions with no statement between them, M22, become one `Assume` with several conditions; a block's value and a final `return e`, below), `lower/scopes.rs` (`scoped_block` and `block_value`, the only two loops over a block's statements, go through it), `lower/stmt.rs` (Task 1a's placeholder arm goes), every exhaustive `Stmt` match (Global Constraints); `range/stmt.rs` (`Assume`: the conditions' checks are decided as an expression statement's, the body is a plain block, nothing is learned), `range/writes.rs` (delegate); `range/events.rs` split (move the statement arms to new `events/stmts.rs`), and there an `Assume` with `check: Check` is a `Fail` event "the assumption `{text}` is checked here" per condition.
- Codegen: new `codegen/src/func/assume.rs` (evaluate each condition in order; a false one calls `kestrel_rt_assume_failed` with its text, then `trap user2`; then the body), `func/stmt.rs`, `rt.rs`, `jit.rs`.
- Runtime: `runtime/src/panic.rs`: `pub fn assume_failed_message(text: &str, line: i32, file: &str) -> String` and `pub unsafe extern "C" fn kestrel_rt_assume_failed(text: *const u8, len: i64, line: i32, file: *const FileName) -> !` (M4).

**The value and `return` rules** (I3, M1): a value block with an assumption computes its tail value inside the innermost assumption's body exactly as `block_value`'s cleanup path does: `owned(v, false)`, `spill`, and `Move` of the temporary for a value with heap memory; the block's value is then that load or move. A function body whose rest ends in `return e` puts `owned(e, true)` in a temporary inside the assumption's body and the `Return` after the `Assume`. So a contracted or inlined function still ends in `Stmt::Return`, and a returned parameter list is still copied.

- [ ] **Step 1: Write the failing tests.**
  - New `kir/tests/assume_lower.rs`:
    - `the_rest_of_the_block_goes_inside`: `f(n: int) { let a = n + 1 \n @assume(n > 0) \n print(a) \n print(n) }` lowers to `[Let, Assume { body: [Print, Print] }]`.
    - `consecutive_assumptions_are_one`: `@assume(a > 0) @assume(b > 0)` and the same on two lines: one `Assume` with two conditions and two `AssumeInfo`s.
    - `a_tail_value_goes_through_a_temporary`: `pure g(n: int) -> int { @assume(n > 0) \n n * 2 }`: the body ends `[Assume { body: [.., Let(t, _)] }, Return(Some(Load(t) or Move(t)))]`.
    - `a_final_return_goes_after_the_assumption`: the same with `return n * 2`.
    - `a_function_with_an_assumption_is_still_inlined`: `h(n: int) -> int { g(n) + 1 }`, for both forms of `g`: after `optimize`, `h` calls nothing and `m.not_inlined` has no entry for `g`.
    - `assumptions_keep_their_text`: `m.assumptions[0].text == "n > 0"`, its line is 1.
  - `cli/tests/report.rs`: `an_assumption_is_a_way_to_fail`: the report of `pure g(n: int) -> int { @assume(n > 0) \n n * 2 }` plus `main() {}` has "line 1: can fail: the assumption `n > 0` is checked here".
  - Programs (expectations at the end):
    - `tests/programs/assume_holds.kes`:
      ```
      pure get(xs: [int], i: int) -> int {
          @assume(i >= 0 and i < xs.len)
          xs[i]
      }

      main() {
          let xs = [10, 20, 30]
          var s = 0
          for k from 0 to 3 {
              @assume(k < xs.len)
              s += get(xs, k)
          }
          print(s)
      }
      // expect: 60
      ```
    - `tests/programs/assume_false.kes`:
      ```
      id(x: int, d: int) -> int {
          if d == 0 { return x }
          id(x, d - 1)
      }

      main() {
          let xs = [1, 2, 3]
          let n = id(5, 1)
          @assume(n <= xs.len)
          print(n)
      }
      // expect-stderr: assumption `n <= xs.len` is false on line 9 of assume_false.kes
      // expect-exit: 101
      ```
    - `tests/programs/assume_scope.kes` (M6, written out):
      ```
      pure pick(xs: [int], i: int) -> int {
          if i >= 0 {
              @assume(i < xs.len)
              return xs[i]
          }
          0
      }

      pure tail(xs: [int], i: int) -> int {
          @assume(i >= 0 and i < xs.len)
          xs[i] * 2
      }

      main() {
          let xs = [4, 5, 6]
          var s = 0
          var k = 0
          while true {
              @assume(k >= 0)
              if k == 3 {
                  break
              }
              s += pick(xs, k) + tail(xs, k)
              k += 1
          }
          let v = if s > 0 {
              @assume(s > 0)
              s + 1
          } else {
              0
          }
          print(s)
          print(v)
      }
      // expect: 45
      // expect: 46
      ```
      (`pick` is 4 + 5 + 6 = 15, `tail` twice that, 30: `s` is 45.)
    - `tests/programs/assume_tail_copy.kes` (I3):
      ```
      pure keep(xs: list[int]) -> list[int] {
          @assume(xs.len > 0)
          xs
      }

      main() {
          let a: list[int] = [1, 2, 3]
          var b = keep(a)
          b[0] = 9
          print(a[0])
          print(b[0])
      }
      // expect: 1
      // expect: 9
      ```
    - `tests/programs/assume_inlined_false.kes`: a small helper with `@assume(i < xs.len)`, inlined into `main`, called with `id(7, 1)` on a 3-item array: the error names the helper's line.
    - `tests/programs/prove_parses.kes`: `@prove(no_alloc)` on a pure arithmetic function (bit operations only), `@prove(inline)` on a method that calls nothing and on an interface method whose implementations call nothing, `@prove(no_alloc, no_panic)` on `for i from 0 to xs.len { s = s ^ xs[i] }`; prints a result. (Contracts are decided from Task 4; these hold in both builds.)
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir --test assume_lower -- --test-threads=2`. Expected: `Stmt::Assume` doesn't exist in KIR.
- [ ] **Step 3: Implement.** KIR types first (the compiler finds the arms), then lowering, codegen and the runtime.
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-kir -p kestrel-codegen -p kestrel-runtime -- --test-threads=2`; `cargo test -p kestrel-cli --test errors --test report -- --test-threads=2`; `KESTREL_TEST_ONLY=assume_,prove_,range_ cargo test -p kestrel-cli --test programs -- --test-threads=2`; clippy on the touched crates; `cargo check` in `kestrel/godot` and `kestrel/bench/kernels`.
- [ ] **Step 5: Commit:** "Lower @assume around the rest of its block and check it at run time".

### Task 2: `@assume` in range analysis — **Opus**

**Goal:** inside its block an assumption teaches what `if cond` teaches; nothing outside its block sees it; under `--release` each assumption statement is trusted unless a bounds check rests on it; each statement says what it removed.

**Files:**
- New `kir/src/range/assume.rs` (the two runs; the comparison; `AssumeUse`; the note on M3), `range/stmt.rs` (the `Assume` arm calls it), new `range/mark.rs` (`Marker` moved out of `mod.rs`; it also sets `AssumeCheck` by statement address: `Check` under `Arith::Checked` always, actively, M12; under `Wrapping`, `Trust` exactly for the statements whose folded `bounds` is false), `range/mod.rs` (`Analysis::assumptions`; a scratch decision table; `budget` counts each statement once more for every `Assume` around it), `range/events/stmts.rs` (a `Trust` assumption is no way to fail).
- Codegen `func/assume.rs`: `Trust` emits nothing unless `opts.verify_proofs`.
- `cli/tests/programs.rs` (`expectations` accepts `// expect-stderr-proofs:` and `// expect-exit-proofs:`) and `cli/tests/programs/release.rs` (the run with the proof check uses them when given) (M5). A release program that uses them prints nothing before its failure, so stdout is the same in both runs.
- Tests: new `kir/tests/range_assume.rs`, new `codegen/tests/assume.rs`, programs and release programs below.

**Interfaces:**
- Consumes Task 1b's `Stmt::Assume` and Round A's `condition` (the true state of a condition), `decide`, `deciding`, `frames`, `learned`, `keep`.
- Produces: `pub struct AssumeUse { pub id: u32, pub func: FuncIdx, pub at: usize, pub removed: u32, pub bounds: bool }` (`id` the first condition's; `at` the `Assume` statement's address, as `Key` does for checks) and `Analysis::assumptions: Vec<AssumeUse>`, **one per analysed statement**: uses are recorded only in deciding passes with `keep == 0`, and several visits of one statement fold together (`removed` summed per distinct site, `bounds` OR) (C3). A function the analysis gave up on records nothing and changes nothing.

The rules, in the addendum's words ("`@assume`", "Soundness rules"):
- **Not deciding** (`!self.deciding`, an enclosing loop's fixed-point rounds): only the plain run (I4).
- **Deciding: the real run** starts from the true state of `condition(st, cond)` for each condition in order; the assumptions inside run as written; decisions go to the real table.
- **Then the plain run** from `st` itself, with every `Assume` inside analysed as a plain block; decisions go to a scratch table. Before the real run, take each enclosing frame's `breaks` and `continues` out with `std::mem::take` (not a clone, I4) and save `learned`'s length; after it, put them back and truncate `learned`; **only the plain run's `break` and `continue` states and its end reach the frames and the code after the block**. `return` goes nowhere in either.
- **Comparison:** a site decided proven in the real run and left in the plain run counts for this statement.
- A contradiction makes the real run unreachable (`keep`), so its checks stay, nothing is removed, and nothing is recorded.

- [ ] **Step 1: Write the failing tests.** In `range_assume.rs` (`analysed` from `range_common`; each with its near miss):
  - `an_assumption_proves_its_bounds_check`: `pure get(xs: [int], i: int) -> int { @assume(i >= 0 and i < xs.len) \n xs[i] }`: no bounds check left in `get`, in both builds; its `AssumeUse` has `removed == 1`, `bounds == true`.
  - `without_the_lower_end_it_does_not`: `@assume(i < xs.len)`: the check stays.
  - `another_list_proves_nothing`: `@assume(i >= 0 and i < ys.len)` then `xs[i]`: stays; `removed == 0`.
  - `a_change_ends_it`: `@assume(i >= 0 and i < xs.len) \n i = j \n s = xs[i]` stays; with a list, `xs.pop()` between: stays.
  - `it_ends_at_its_block`: `var s = 0 \n if c { @assume(i >= 0 and i < xs.len) \n s = xs[i] } \n s + xs[i]`: the first proven, the second left.
  - `a_break_leaves_it_behind`: `while true { @assume(i >= 0 and i < xs.len) \n if c { break } \n s = xs[i] \n break } \n xs[i]`: the one in the loop proven, the one after it left.
  - `the_next_iteration_starts_without_it`:
    ```
    pure f(xs: [int], n: int) -> int {
        var s = 0
        if xs.len > 0 {
            var i = 0
            while i < n {
                s = s ^ xs[i]
                @assume(i + 1 < xs.len)
                i += 1
            }
        }
        s
    }
    ```
    `xs[i]` keeps its check (it would be proven if the assumption reached the loop's top), and the assumption's `removed == 0`.
  - `facts_from_before_the_block_survive_it` (the reason for the plain run):
    ```
    pure f(xs: [int], n: int, c: bool) -> int {
        var s = 0
        for i from 0 to xs.len {
            if c {
                @assume(n >= 0 and n < xs.len)
                s = s ^ xs[n]
            }
            s = s ^ xs[i]
        }
        s
    }
    ```
    Both bounds checks proven.
  - `a_contradiction_proves_nothing`: `if i < 0 { @assume(i > 5) \n s = xs[i] }`: stays; no `AssumeUse`.
  - `an_assumption_removes_an_overflow_check`: `@assume(a >= 0 and a <= 1000 and b >= 0 and b <= 1000) \n a * b`: proven under `Checked` (`removed == 1`, `bounds == false`); under `Wrapping` there is no site and `removed == 0`.
  - `release_trusts_an_arithmetic_only_assumption`: `pure d(a: int, b: int) -> int { @assume(b > 0) \n a / b }`: under `Wrapping` the division is proven and the `Assume` is `Trust`; under `Checked` it is `Check`.
  - `checked_resets_trust` (M12): a module marked under `Wrapping`, then a fresh copy of the source analysed under `Checked`: every `Assume` is `Check`.
  - `release_keeps_a_bounds_assumption`: `get` under `Wrapping`: `Check`.
  - `release_keeps_the_outer_of_a_redundant_pair`: two identical assumptions with a statement between them (`let t = 0`, so M22 doesn't merge them) then `xs[i]`, `Wrapping`: the outer `Check`, the inner `Trust`, the index proven.
  - `release_keeps_both_of_a_needed_pair`: `@assume(i >= 0)`, `let t = 0`, `@assume(i < xs.len)`, then `xs[i]`: both `Check`.
  - `release_trusts_each_copy_on_its_own` (C3): the reviewer's program: `get` inlined twice into `main`, once in `for k from 0 to xs.len { s += get(xs, k) }` (copy A: the loop proves `xs[k]`, `bounds == false`) and once as `get(xs, j)` with `j = id(7, 1)` (copy B: `bounds == true`); under `Wrapping` copy A is `Trust` and copy B `Check`; two `AssumeUse`s in `main` with different `at`.
  - `an_inlined_assumption_counts_in_its_copy`: `h(xs: [int], k: int) -> int { get(xs, k) ^ 1 }`: no bounds check left in `h`; `AssumeUse`s for `get`'s statement and for `h`'s copy.
  - `an_assumption_around_a_nested_loop`: `for k from 0 to m { @assume(n >= 0 and n <= xs.len) \n for i from 0 to n { s = s ^ xs[i] } }`: proven.
  - `a_length_assumption_proves_a_neighbour`: `pure f(xs: [u8], i: int) -> u8 { @assume(xs.len == 100 and i >= 0 and i < 99) \n xs[i + 1] }`: proven.
  - `a_field_in_memory_teaches_nothing`: a struct `Grid { n: int, cells: [int] }` and `pure f(g: Grid, i: int) -> int { @assume(i >= 0 and i < g.n and g.n <= g.cells.len) \n g.cells[i] }`: stays (Round A: fields in memory aren't terms).
  - `the_plain_run_keeps_a_division_fact` (M3): `pure f(a: int, b: int) -> int { var r = 0 \n arena { @assume(b > 0) \n r = a / b } \n r + 100 / b }` under `Wrapping`: the first division is proven by the assumption, which is `Trust`; the second, after the block, is proven by what the plain run learned from the first division's zero check, which still holds because the plain divide traps on zero (`range/assume.rs` says so). Under `Checked` the same, with the assumption `Check`.
  - `an_assumption_does_not_spend_the_function_budget` (I4): a function at about 90% of its statement budget without an assumption still finishes (no `gave_up`) with one assumption around half its body.
  - `a_function_given_up_on_keeps_its_assumption`: 300 nested `if`s around an assumption (on a 2 MB-stack thread, as Round A's test): `gave_up`, the `Assume` stays `Check`, no `AssumeUse`.
  - `codegen/tests/assume.rs`: `a_checked_build_tests_the_assumption` (`d` has one `trap user2`: the assumption's; the division's check is gone); `a_trusted_assumption_emits_nothing` (`d` under `--release`: none); `a_kept_assumption_is_tested_under_release` (`get` under `--release`: one, the assumption's); `the_proof_check_tests_a_trusted_assumption` (a child process, as `verify_mode_still_checks_a_proof`: `--release` with `verify_proofs`, `d(1, 0)` exits 101 with "assumption `b > 0` is false on line").
  - `cli/tests/report.rs`: `a_trusted_assumption_is_no_way_to_fail_under_release` (`d`'s `--release` report has no assumption item; the checked one has it).
  - Programs that must still fail (`// expect-exit: 101`, values from `id`): `assume_keep_change.kes` (`@assume(i >= 0 and i < xs.len)` true for `i = 2`, then `i = id(3, 1)`, then `print(xs[i])`: "index 3 out of bounds for length 3 on line ..."); `assume_keep_pop.kes` (a list of 3, `i = 2`, the assumption, `xs.pop()`, then `xs[i]`: "index 2 out of bounds for length 2 ...").
  - Release programs (`tests/release/`), printing nothing before their failure:
    - `assume_release_bounds.kes`: `i = id(7, 1)`, `@assume(i >= 0 and i < xs.len)`, `print(xs[i])` with 3 items: "assumption `i >= 0 and i < xs.len` is false on line ..." (the same with the proof check), never a read outside the array.
    - `assume_release_pair.kes`: the redundant pair (with `let t = 0` between) and `i = 7`: the error names the outer assumption's line.
    - `assume_release_copies.kes` (C3): the reviewer's two-copy program: "assumption `i >= 0 and i < xs.len` is false on line ..." (copy B stays evaluated).
    - `assume_release_block.kes`: `b = id(0, 1)`; `if id(1, 1) == 1 { @assume(b > 0) \n if id(0, 1) == 1 { r = 10 / b } }` (the inner division never runs; the trusted assumption removes its check), then `print(100 / b)`: "division by zero on line ..." (the division after the block keeps its check), with `// expect-stderr-proofs: assumption `b > 0` is false on line ...`.
    - `assume_release_divide.kes` (M3): `b = id(5, 1)`, `arena { @assume(b > 0) \n r = 7 / b }`, then `print(r + 100 / b)`: prints 21 in both runs.
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir --test range_assume -- --test-threads=2` and `cargo test -p kestrel-codegen --test assume -- --test-threads=2`. Expected: the proofs fail (nothing learned yet) and `AssumeUse` doesn't exist; the near misses pass and must still pass after Step 3.
- [ ] **Step 3: Implement** in the files above. Show each Critical rule caught by mutation (the task report has the table: the rule broken, the tests that failed): the plain run's exits replaced by the real run's; the plain run keeping inner assumptions; `Trust` set while `bounds` is true; Trust keyed by `id` instead of the statement (C3); the real run started without `condition`'s forgetting.
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-kir -p kestrel-codegen -- --test-threads=2` (Round A's range tests unchanged); `KESTREL_TEST_ONLY=assume_,range_ cargo test -p kestrel-cli --test programs -- --test-threads=2`; `cargo test -p kestrel-cli --test programs release_programs_run_as_executables -- --test-threads=2` and `--test report`. **Then the required proof check,** as Round A's Tasks 2 and 3: every program in all three passes (JIT, exe with `KESTREL_TEST_VERIFY_EXE=1`, release), one batch of `KESTREL_TEST_ONLY` prefixes at a time, at idle priority: no "range analysis removed", no release-pass timeout. The task report gives the result; the task isn't done without it.
- [ ] **Step 5: Commit:** "Let range analysis use assumptions inside their block, and trust them under --release only away from bounds checks".

### Task 3: Contract plumbing: regions, copies, sites and the driver step — **Opus**

**Goal:** everything contracts need, with no contract decided yet: loop regions and inlined copies of contracted functions as `Prove` blocks, every contract listed in `Module::contracts` with its function right, checks and events tagged with their regions, range analysis run once in the driver for an annotated program and reused by codegen, and the "removes no check" warning from `kestrel check`.

**Files:**
- KIR: new `kir/src/contract.rs` (`ContractSet`, `ContractSite`, `ProveTarget`; re-exported from `ir`), `ir_stmt.rs` (`Stmt::Prove { id: u32, at: u32, body: Vec<Stmt> }` and its arms, Global Constraints), `ir.rs` (`Module::contracts: Vec<ContractSite>`, `Module::analysed: Option<Arith>`), `lower/stmt.rs` (an annotated `while`, `for` or `par for`: every statement its lowering pushes goes in one `Prove { id, at: 0 }`), `lower/mod.rs` (a site for each annotated function and method), `lower/iface.rs` (a site for each implementing method of an annotated interface method, from the checker's list of implementations, not the method tables made on demand, M10), `lower/par.rs` and `lower/closure.rs` (C2: where lowering moves statements into a new function, every site whose `Prove` id is in them is re-pointed to that function; the same anywhere else the compiler copies lowered statements into a new function), `inline.rs` (I1: expanding a callee that has a function site wraps the expansion in `Prove { id: site, at: call line }`, with the returned value computed into a temporary inside it as `block_value` does; `Seq([Prove { .. }], Load/Move(t))`).
- Range: `range/mod.rs` and `range/stmt.rs` (`CheckLeft::regions`: the `Prove` ids around the site, kept beside `loop_lines`, **recorded in both `remove_checks` and `mark_proven`**, M20; `Prove` is a plain block); `sites_of` keeps a `Prove` stack too, so a site listed for a gave-up function or as undecided has its regions (M2); `FuncChecks::undecided: bool` set when a site was never decided (the `debug_assert!` stays); `range/events.rs` and `events/stmts.rs` (`Event::regions`; a new `What::Cut` event where the walk stops at `MAX_DEPTH`); `remove_checks` and `mark_proven` set `Module::analysed`.
- Codegen `lib.rs` (`define_module`: with `m.analysed == Some(mode)` it compiles `m` as it is; with the other mode it returns "the module was analysed for the other build"; with `m.analysed` set and `remove_checks: false` it returns "the module was analysed, so it can't be compiled with every check kept" (M12); with `None`, as now).
- Driver: new `driver/src/prove.rs` (`prove`), `lib.rs` (`Compiled` keeps each file's base, path and text to render later diagnostics; the existing `report` function renders them).
- CLI `cli/src/main.rs`: `check`, `run` and `build` call `prove` after `compile` (only `check` asks for warnings); `check --report` reuses its analysis and prints the report even when `prove` fails; exit 1 on failure.
- Godot `godot/src/lib.rs`: calls `prove` with its checks setting; a failure is a `godot_error!` and nothing loads.
- Tests: new `kir/tests/contracts_sites.rs`, `codegen/tests/proven.rs` (three tests), new `cli/tests/contracts.rs`, new `driver/tests/prove.rs`.

**Interfaces:**
- `pub struct ContractSet(pub u8)` with `NO_ALLOC`, `NO_CHECKS`, `NO_PANIC`, `INLINE`, `contains`, `insert`.
- `pub struct ContractSite { pub target: ProveTarget, pub contracts: Vec<(Contract, Span)>, pub required_by: Option<String> }`; `pub enum ProveTarget { Func(FuncIdx), Loop { func: FuncIdx, line: u32 } }` (`func` the function that holds the loop's own code; `line` the loop keyword's packed line); a `Prove`'s `id` is the index of its site. `required_by` is "`Shape.area`" for a site made from an interface method, whose spans then point at the implementing method's name.
- `pub fn prove(c: &mut Compiled, checked: bool, warn: bool, style: DiagStyle) -> Proved`, `pub struct Proved { pub ok: bool, pub text: String, pub analysis: Option<Analysis> }`. With no `@prove` (and, when `warn`, no `@assume`) it returns at once, `ok`, no analysis, the module untouched. Otherwise it runs `remove_checks` in place in the mode given. With `warn` and assumptions, it also runs it on a clone in the other mode (cloned before the in-place run) and warns for each assumption whose `AssumeUse`s, in both modes, all have `removed == 0` (at least one recorded). Contracts are not decided yet (Task 4 adds the call).

- [ ] **Step 1: Write the failing tests.**
  - `contracts_sites.rs`:
    - `function_and_method_contracts_are_listed`: `@prove(no_alloc, inline)` on `f` and `@prove(no_panic)` on a method: two sites with those targets, contracts and spans.
    - `an_annotated_loop_is_one_prove_block`: `@prove(no_alloc)` on `for x in xs.filter((e) { e > 0 }) { s = s ^ x }`: the body holds one `Stmt::Prove`, and every statement the loop lowered to (the `filter` call included) is inside it; its `id` is the loop's site, `ProveTarget::Loop` with the `for`'s line.
    - `a_par_for_and_a_while_are_regions_too`.
    - `a_loop_in_a_par_for_body_belongs_to_the_body` (C2): the reviewer's program (`@prove(no_checks)` on `for i from 0 to 4 { acc = acc * 1000 }` inside `par for p in ps`): the site's `func` is the `par for` body, which holds the `Prove`.
    - `a_loop_in_a_closure_belongs_to_the_closure`: likewise for a closure literal's code.
    - `an_inlined_contracted_function_is_a_prove_block` (I1): `@prove(no_checks) pure get(...)` called from `step`: after `optimize`, `step` holds `Prove { id: get's site, at: the call's line }` around `get`'s code and the value's temporary; `get` itself holds none.
    - `an_interface_contract_binds_each_implementation`: `interface Shape { @prove(no_alloc) \n area(self) -> float }` with `Goblin` and `Box` implementing it, and only `Goblin` ever converted to `Shape`: two sites, `Func(Goblin.area)` and `Func(Box.area)` (M10), `required_by == Some("`Shape.area`")`, spans at each method's name.
    - `checks_and_events_know_their_region`: after `remove_checks` and after `mark_proven` (M20), and from `events`: a check in the annotated loop has `regions == [id]`, one in its header too, one after it `[]`; an allocation likewise; in a function given up on, the listed sites still carry their regions (M2).
    - `the_events_walk_says_where_it_stopped`: a function built by hand with 5,000 nested `If`s: its events end with `What::Cut`.
  - `codegen/tests/proven.rs`: `an_analysed_module_is_compiled_as_it_is` (analysed checked in place, compiled checked: no second analysis, the same `trap user2` count as today); `a_module_analysed_for_the_other_build_is_refused`; `an_analysed_module_with_every_check_kept_is_refused` (M12).
  - `driver/tests/prove.rs`: `a_program_without_annotations_is_left_alone` (`ok`, empty text, `module.analysed == None`); `an_annotated_program_is_analysed_once_in_place` (`analysed == Some(mode)`).
  - `cli/tests/contracts.rs`:
    - `an_assumption_that_removes_nothing_warns`: `@assume(n > 0)` before `print(n)`: `kestrel check` exits 0 and stderr has "warning: `@assume(n > 0)` removes no check; delete it"; `kestrel run` prints no warning.
    - `an_assumption_for_overflow_checks_does_not_warn_under_release`: `@assume(a >= 0 and a <= 1000) \n print(a * 3)`: `kestrel check --release` gives no warning (it removes a check in a checked build).
    - `annotations_do_not_change_what_runs`: `prove_parses.kes`'s output through `run` and `build`, checked and `--release`.
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir --test contracts_sites -- --test-threads=2`. Expected: `Stmt::Prove`, `Module::contracts` don't exist.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-kir -p kestrel-codegen -p kestrel-driver -- --test-threads=2` (`kir/tests/inline.rs` unchanged: a callee with no contract inlines exactly as before); the cli `contracts`, `report`, `errors` and `modules` targets; `KESTREL_TEST_ONLY=assume_,prove_ cargo test -p kestrel-cli --test programs -- --test-threads=2`, then the same with `KESTREL_TEST_VERIFY_EXE=1` (the in-place analysis must give the same proofs); `cli/tests/report.rs`'s exact fixture unchanged; clippy; `cargo check` and clippy in `kestrel/godot` and `kestrel/bench/kernels`.
- [ ] **Step 5: Commit:** "Record contracts, loop regions and inlined copies, and analyse annotated programs once in the driver".

### Task 4: Deciding contracts, through calls — **Opus**

**Goal:** `range::contracts::check` decides every contract in `Module::contracts`, in every compiled copy of its target, as the addendum's "What a contract covers", "Calls through interfaces, function values and natives" and "Error messages" say, and the driver step reports what fails.

**Files:**
- New `kir/src/range/contracts.rs` (the entry and the per-contract rules), `contracts/region.rs` (a site's copies and their items: for a function, its own function whole and every `Prove { id }` elsewhere; for a loop, every `Prove { id }`; items filtered by the copy's function and region; `par for` bodies and closures made there whole; `m.init` for `main`; fail closed when a loop's `Prove` is nowhere, C2), `contracts/words.rs` (the messages and help lines).
- `range/events.rs`, `events/stmts.rs`, `events/reasons.rs` (C1): a `CallIndirect`, a `CallVirtual` and a native call are one `What::Unknown { kind: Indirect | Virtual(ContractSet) | Native, reason }` event instead of the `Alloc` and `Fail` pair; the report renders them with the same words (its exact fixture unchanged). The item-limit failure of growth (`add`, `insert`, a map key set, the built-ins that build a list by growing it) is a `Fail` whose reason is marked as running out of resources (I5); `array[T](n)` isn't.
- `range/summary.rs`: per-contract chains: `allocates`, `fails` (resource failures left out for contracts, kept for the report), `checks` (the first check left; `via` "has a bounds check", `tail` ": {check text}"), `unknown_checks`, `unknown_alloc`, `unknown_panic` (an `Unknown` event for that contract, a give-up, an undecided site or a `What::Cut`, in a function or below it), each by `chains`'s fixed point (C1); and `recursion: Option<Chain>` (a function on a call-graph cycle, by `Call`, `Body` and `Closure` edges, or one that reaches one: the reason "can call itself (line L)" with tail ", so the stack may run out", I2).
- `ir_expr.rs`, `ir_stmt.rs`: `CallVirtual { ..., keeps: ContractSet }` (9 patterns without `..`), set in `lower/iface.rs` from the interface method's `@prove`.
- `driver/src/prove.rs`: calls `contracts::check` and renders its errors (with help lines).
- `cli/tests/programs.rs`: the release pass also fails on "can't be proven" (M13).
- Tests: new `kir/tests/contracts_calls.rs`, error tests, programs, `cli/tests/contracts.rs`, `cli/tests/report.rs`.

**Interfaces:**
- `pub fn check(m: &Module, a: &Analysis) -> Vec<ContractError>`, `pub struct ContractError { pub span: Span, pub message: String, pub help: Option<String> }`; one per failed (site, contract), at the contract's span, naming the first item in report order (own checks before other items on a line, as `items::arrange`), the own copy before inlined copies; never a count. When an implementing method's own `@prove` and its interface's ask for the same contract, only the interface's error is given (M9).
- Per contract, the addendum's table: each copy's own items; each call left (`What::Call`), closure made (`What::Closure`) and `par for` (`What::Body`) followed through `Summary`; `Unknown` events decided per contract (a function value for all three, a native for `no_alloc` and `no_panic`, an interface call for each contract its `keeps` lacks), in the copy and below a call; a `CallIndirect` or `CallVirtual` always breaks `inline`; `recursion` breaks `no_panic`; a give-up, an undecided site or a `What::Cut` in a copy's function breaks all four; a site on `main` also follows `m.init`. For `inline` (M14): calls left in the copy, in a closure's code made there, and from a compiler-made function there into program code (a `sort_by` comparison calling the key's closure) count; the compiler-made function itself doesn't. The report's totals are never read.

- [ ] **Step 1: Write the failing tests.**
  - Error tests (`kestrel check`, checked build). The `// error` line is the `@prove` line; the message has the item's own line (M21):
    - `prove_own_checks.kes`: `@prove(no_checks)` on the report fixture's `total` → "`total` can't be proven `no_checks`: it has an overflow check on line N: `s + xs[i]` may exceed `int` ..." (N the line of `s += xs[i]`); the same body with the overflow made impossible (`s = s ^ xs[i]`, and `s ^ xs[k]` at the end) → "... it has a bounds check on line N: `xs[k]` may be outside `xs` (`k` can be any `int`)".
    - `prove_loop.kes`: `@prove(no_checks)` on `for i from 0 to n { acc = acc * 3 }` → "the loop at line N can't be proven `no_checks`: it has an overflow check on line N+1: ..."; a sibling loop in the same function that holds raises nothing.
    - `prove_header.kes`: `@prove(no_alloc)` on `for x in xs.filter((e) { e > 0 })` → "the loop at line N can't be proven `no_alloc`: it allocates on line N: `filter` builds a new list"; on `for i from 0 to xs.len` with `xs.any((e) { e > 0 })` in the body: no error.
    - `prove_no_alloc.kes`: the report fixture's `grow` → "`grow` can't be proven `no_alloc`: it allocates on line N: `add` may grow `ys`".
    - `prove_no_panic.kes`: `xs.remove_at(0)` → "... it can fail on line N: ..."; a bounds check alone also fails `no_panic`; an `@assume` inside → "it can fail on line N: the assumption `n <= xs.len` is checked here"; `array[int](n)` with `n` a parameter → a way to fail (I5); `ys.add(1)` with `@prove(no_panic)` alone → **no error** (I5).
    - `prove_recursion.kes` (I2): `@prove(no_panic)` on a function that calls itself → "... it can call itself (line N), so the stack may run out"; on one calling a mutually recursive pair → "... it calls `a` (line N), which can call itself (line M), so the stack may run out".
    - `prove_par_for.kes`: `@prove(no_alloc)` on a `par for` → "the loop at line N can't be proven `no_alloc`: it allocates on line N: a `par for` makes an arena for each thread" (and the M18 help in full diagnostics); `@prove(no_checks)` on a `par for` whose body indexes another list by its index → the body's bounds check.
    - `prove_par_body_loop.kes` (C2): the reviewer's program → "the loop at line 5 can't be proven `no_checks`: it has an overflow check on line 6: ...".
    - `prove_chain_alloc.kes`: the spec's example: `step` calls `spawn` (kept a call: it calls itself) which adds to `ps` → "`step` can't be proven `no_alloc`: it calls `spawn` (line 40), which adds to `ps` (line 12) and may grow the list" (lay the file out so the lines are 40 and 12, or use the real lines).
    - `prove_chain_checks.kes`: a call to a recursive `g` with `xs[i]` → "... it calls `g` (line 5), which has a bounds check (line 9): `xs[i]` may be outside `xs` (`i` can be any `int`)".
    - `prove_chain_unknown.kes` (C1): the reviewer's program (`relay` calling itself and `s.area()`, `Sq.area` with a bounds check on fields in memory, `@prove(no_checks) total(s: Shape) -> int { relay(s, 1) }`) → "`total` can't be proven `no_checks`: it calls `relay` (line N), which calls a method of the interface value `s` (line M); its body isn't known here"; the same with `relay(f: (int) -> int, d: int)` and `f(d)`; and a virtual call whose method promises `no_alloc` only still fails `no_checks`.
    - `prove_inline.kes`: a call to a function with an early `return` → "`f` can't be proven `inline`: it calls `g` (line N), which isn't inlined: it has an early `return`"; a recursive call → "... it calls itself"; a closure literal made in `f` whose code calls a function that isn't inlined, and a `sort_by` whose key closure calls one (M14) → each fails.
    - `prove_inline_copy.kes` (I1): a chain `f0 → f1 → f2 → ... → f9` of small functions; `@prove(inline)` on `f2` holds in its own copy (seven levels below it) but not in its copy inside `f0`, where calls nest more than 8 deep → "`f2` can't be proven `inline` where it is inlined into `f0` (line N): it calls `f9` (line M), which isn't inlined: inlined calls nest more than 8 deep" (the copy is in `f0`'s code, expanded inside `f1`'s; N is the call's line as the `Prove` records it; the inliner wraps a contracted callee at every depth).
    - `prove_fn_value.kes`: `apply(f: (int) -> int, x: int) -> int { f(x) }` with `@prove(no_alloc, no_checks, no_panic, inline)`: four errors, "it calls the function value `f` (line N); its body isn't known here" (three) and "it calls the function value `f` (line N), which can't be inlined".
    - `prove_iface_unpromised.kes`: `@prove(no_alloc)` on a function calling `s.area()` where `Shape.area` promises nothing → "it calls a method of the interface value `s` (line N); its body isn't known here".
    - `prove_iface_impl.kes`: `interface Shape { @prove(no_alloc) \n area(self) -> float }`, `Goblin.area` builds a list → "`Goblin.area` can't be proven `no_alloc`, which `Shape.area` requires: it allocates on line N: ...", on the line of `Goblin.area`'s name; with `@prove(no_alloc)` on `Goblin.area` too, still one error (M9).
    - `prove_native.kes` (M16): `use draw` and `@prove(no_alloc)` on a function that calls the native `draw.clear(...)` directly (it is `pub native` in `driver/std/draw.kes`, so no wrapper is involved; `check` builds nothing) → "... it calls the native function `draw.clear` (line N); its body isn't known here" (match the name as KIR has it).
    - `prove_main_constants.kes`: `@prove(no_alloc) main()` using a heap constant (`let TABLE = [1, 2, 3]`) → "`main` can't be proven `no_alloc`: it builds the program's constants first, which ...".
    - `prove_closure.kes`: `@prove(no_alloc)` on a function that stores a capturing closure → its environment allocation.
    - `prove_other_file.kes` with a helper in `prove_geom.kes`: a chain through the helper writes its line as `prove_geom.kes:N` (if the error harness can't hold a two-file program, put it in `cli/tests/contracts.rs`).
  - Programs that pass (and run, both builds, M13): `prove_holds.kes` (the four contracts on an xor loop over `for i from 0 to xs.len`; `@prove(no_checks)` on `get` with its `@assume`; `@prove(no_alloc, no_checks)` on a recursive function that allocates nothing and has no checks, which `no_panic` would fail (I2); `@prove(no_alloc)` on a loop using `xs.any(...)`; `@prove(no_panic)` on a function that adds to a list (I5)); `prove_iface_promised.kes` (`@prove(no_alloc, no_panic)` on `Shape.area`, two implementations that keep it **using float arithmetic only** (an `int` product of fields read from memory keeps its overflow check, M17), and `@prove(no_alloc, no_panic)` on a function looping over `list[Shape]` calling `s.area()`).
  - `contracts_calls.rs`: `the_checks_chain_follows_calls_and_cycles` (a cycle where one function has a check: every function in it has `checks`; a clean cycle has none); `recursion_is_found_through_a_cycle` (I2); `an_unknown_call_below_a_call_is_unknown_to_the_caller` (C1, per contract); `a_give_up_is_unknown_to_every_caller`; `a_cut_walk_is_unknown_to_every_caller` (KIR built by hand); `growth_is_not_a_failure_for_contracts` (I5); `a_promised_interface_call_is_not_an_allocation` (no allocation for it).
  - `cli/tests/contracts.rs`: `release_contracts_use_the_release_build` (`@prove(no_checks)` on `pure m(a: int, b: int) -> int { a * b }`: `kestrel check` fails exit 1, `kestrel check --release` passes, `kestrel build --release` builds, `kestrel run` exits 1 and runs nothing); `the_report_prints_when_contracts_fail` (exit 1, the report on stdout, the error on stderr); `a_function_the_analysis_gave_up_on_fails_every_contract` (300 nested `if`s with `@prove(no_alloc)`: "range analysis stopped early in it (its code nests too deeply), so nothing can be proven about it; split it into smaller functions"); `a_callee_given_up_on_fails_its_callers`; `an_inlined_copy_in_a_caller_given_up_on_fails` (I1: `@prove(no_checks)` on `get` with its `@assume`, inlined into a generated function with 300 nested `if`s: "`get` can't be proven `no_checks` where it is inlined into ..."); `full_diagnostics_show_the_help` (the help lines for an unpromised interface call, a function value and a `par for`).
  - `cli/tests/report.rs`: `a_promised_interface_call_lists_no_allocation`; the exact fixture unchanged.
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-cli --test errors --test contracts -- --test-threads=2`. Expected: no contract errors yet.
- [ ] **Step 3: Implement.** Show by mutation that each rule is held by a test (the task report has the table): the header left out of a loop region; a `par for` body not followed; inlined copies ignored (I1); a site whose `Prove` is nowhere passing (C2); a callee's `checks` chain ignored; the unknown chain dropped for `no_checks` (C1); recursion passing `no_panic` (I2); `keeps` trusted without the implementations being checked; give-ups ignored; `m.init` not followed.
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-kir -- --test-threads=2`; the cli `errors`, `contracts`, `report` and `modules` targets; `KESTREL_TEST_ONLY=assume_,prove_,iface_ cargo test -p kestrel-cli --test programs -- --test-threads=2` (with the release pass now failing on "can't be proven"); clippy; `cargo check` and clippy in `kestrel/godot` and `kestrel/bench/kernels` (the `CallVirtual` change).
- [ ] **Step 5: Commit:** "Fail the build when a contract doesn't hold in any copy, following calls".

### Task 5: Measure and document — **Sonnet**

**Files:** `docs/superpowers/specs/2026-10-18-at-annotations-design.md` (an "As built" after the addendum), `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md` (an `@` section: `@prove`, `@assume`, the contracts, inlined copies, the warning, the run-time error, and what `no_panic` doesn't count: running out of memory, the item limit on growth, and under `--release` a false trusted assumption's divide fault, M19; that recursion fails `no_panic`; that a function-level `@prove(no_panic)` and an `@assume` in it can't both hold in a checked build), `README.md` (`@prove` and `@assume` next to the report), `docs/superpowers/plans/2026-10-09-core-2a-followups.md` (close the Round B items of "Found while building range analysis"; add "Found while building `@prove` and `@assume`"), `docs/superpowers/plans/2026-10-10-handoff.md` ("Start here"). Touch `kestrel/bench/voxels3d/results.md` only if nobody else has uncommitted changes there (`git status`); otherwise record the numbers in the spec's "As built" only.

- [ ] **Step 1: Check first.** `git status`: if the follow-ups file or `kestrel/bench/voxels3d/` has changes that aren't this round's, stop and ask the controller (another agent's work); never stage them.
- [ ] **Step 2: Compile time, no annotations (target: no measurable change, under 2%, I4).** Build the compiler at the commit before Task 1a in a worktree at `C:\kt\rb-old` (the long-path limit) and the current one; `kestrel build` of `kestrel/bench/voxels3d/main.kes` (the frozen benchmark, no annotations), old and new taking turns, checked and `--release`, 9 runs each, medians and ranges, as Round A's table (`kestrel/bench/voxels3d/results.md`, "Compile time": 181 ms (172-195) checked, 160 ms (153-211) `--release`). Also `kestrel check --report` of `voxels3d.kes`, checked and `--release`, identical between old and new.
- [ ] **Step 3: Compile time, with annotations (targets: within 10% of the unannotated build; `kestrel check` with an assumption at most 2.2 times without, I4).** A scratch copy of the voxel world outside the repo with `@prove(no_alloc)` on `hash`, `lattice` and `octave` and one `@assume` (adjust until the contracts hold): its `kestrel build` time against the unannotated file, same method; and `kestrel check` time with and without the assumption. Record both; a miss goes to the follow-ups with what it cost.
- [ ] **Step 4: Docs,** each in its file's style, honest numbers included. The spec's "As built" lists what differs from the addendum, the rulings made during the build, the holes reviews found, and what was owed. The follow-ups file gets at least: contracts on function types; natives marked safe; `@prove` on a native in a built-in module; warnings in `kestrel run` and the Godot add-on; facts across calls (an assumption at a call site doesn't reach the callee); indexes that are a constant or a local in an assumption (`grid[y].len`); and the two syntax picks if the user hasn't ruled.
- [ ] **Step 5: The full suite,** only if the user says the machine is free: `cargo test --workspace --no-fail-fast` (including the exe pass without the proof check), `cargo clippy --workspace --all-targets -- -D warnings`, and clippy in `kestrel/godot` and `kestrel/bench/kernels`; otherwise record it as owed in the handoff.
- [ ] **Step 6: Commit:** "Measure and document @prove and @assume".
