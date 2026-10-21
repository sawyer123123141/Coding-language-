# Round B Implementation Plan: `@prove` and `@assume`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Build on `main`, with the ledger under `.superpowers/sdd/2026-10-20-round-b/`.

**Goal:** Parse `@prove(no_alloc, no_checks, no_panic, inline)` and `@assume(cond)`; check assumptions at run time and let range analysis use them inside their block; fail the build when a contract on a function, method, interface method or loop doesn't hold, with the call chain in the error.

**Architecture:** `@` is a new token. `@prove` is kept beside the syntax tree (`Program::proves`, keyed by the target's span); `@assume` is a statement. Lowering puts the rest of an assumption's block inside a new KIR block `Stmt::Assume`, and every statement an annotated loop lowers to inside a no-op block `Stmt::Prove`; `Module::contracts` lists every contract with its target. Range analysis runs an assumption's block twice (with it, and as if no assumption in the block were written; what leaves the block comes from the second run) and marks under `--release` which assumptions it may trust. A new driver step (`kestrel_driver::prove`) runs range analysis once, in place, when a program has annotations, gives the "removes no check" warning (`kestrel check` only) and calls `range::contracts::check`, which reads the analysis and the report's walk and summaries; codegen reuses that analysis.

**Tech Stack:** Rust, Cranelift 0.116. Tests are Rust unit tests plus `.kes` program, error, release and CLI tests.

**Spec:** `docs/superpowers/specs/2026-10-18-at-annotations-design.md`, with its "Round B design addendum (2026-10-20)", on top of Round A (`docs/superpowers/specs/2026-10-19-range-analysis-design.md`, "As built"). Every message, rule and number comes from them; the spec wins any conflict. **Two syntax picks wait for the user** (the addendum's "Syntax pick (1)" and "(2)"): build them as written, keep each in one place in the parser or checker so a change is cheap, and don't build on them elsewhere.

## Global Constraints

- **Soundness first.** Critical bugs: a removed check that could fire; an assumption used outside its block; an assumption trusted under `--release` while a bounds check rests on it; a contract that passes while what it covers has an item it counts. Where a rule is unclear, keep the check, keep the assumption evaluated, fail the contract.
- **Messages,** exactly (the addendum's "Lexing, parsing ...", "`@assume`", "The warning", "Error messages"):
  - parse: "unknown annotation `@fast`; Kestrel has `@prove` and `@assume`"; "unknown contract `fast`; `@prove` takes `no_alloc`, `no_checks`, `no_panic` and `inline`"; "expected an annotation name after `@`, like `@prove` or `@assume`"; "`@prove` needs at least one contract, like `@prove(no_alloc)`"; "`@prove` goes before a function, a method or a loop (`for`, `par for` or `while`)"; "`@prove` can't go on a native function"; "`@assume` needs a condition, like `@assume(i < xs.len)`"; "`@assume` goes inside a function, as a statement of its own".
  - check: "`@assume` takes comparisons of integers joined with `and`, like `@assume(i < xs.len and n >= 0)`"; "`@assume` can't call a function or index a list (a `--release` build may not run it); compare names, fields, numbers and `.len`".
  - run time: "assumption `n <= xs.len` is false on line 9 of f.kes" (the text as written; no " of f.kes" without a file).
  - warning: "`@assume(n <= xs.len)` removes no check; delete it".
  - report: a `can fail` item "the assumption `n <= xs.len` is checked here".
  - contracts: "{target} can't be proven `{contract}`: {reason}", target "`step`", "`Goblin.area`" (a method) or "the loop at line 12"; an implementation of an interface method: "`Goblin.area` can't be proven `no_alloc`, which `Shape.area` requires: {reason}". Reasons: "it has a bounds check on line 14: {check text}" (`an overflow`, `a shift`, `a division`, `a narrowing`, `a bounds`); "it allocates on line 13: {text}"; "it can fail on line 9: {text}"; "it calls `big` (line 7), which isn't inlined: {inliner's reason}"; "it calls the function value `f` (line 8), which can't be inlined"; "it calls a method of the interface value `s` (line 8), which can't be inlined"; through calls "it " + the report's chain text ("it calls `spawn` (line 40), which adds to `ps` (line 12) and may grow the list"; for a check "..., which has a bounds check (line 9): `xs[i]` may be outside `xs` (`i` can be any `int`)"); a report item without its own sentence "it " + "{via} ({line}){tail}" ("it calls the function value `f` (line 8); its body isn't known here"); give-up "range analysis stopped early in it (its code nests too deeply), so nothing can be proven about it; split it into smaller functions" (or "it took too long to analyse"), through a call "it calls `g` (line 5), where range analysis stopped early (its code nests too deeply)"; `main`'s constants "it builds the program's constants first, which ...".
  - contract help lines: "`kestrel check --report` lists everything left in it" (default); "add `@prove(no_alloc)` to the interface method `Shape.area`; then every implementation must keep it" (an unpromised interface call); "contracts can't see into a function value yet; call a named function or an interface method instead" (a function value).
- **Numbers and names:** contracts `no_alloc`, `no_checks`, `no_panic`, `inline`; `ContractSet` bits `NO_ALLOC = 1`, `NO_CHECKS = 2`, `NO_PANIC = 4`, `INLINE = 8`, and `no_panic` always brings `no_checks`. Round A's limits are unchanged: `MAX_TERMS = 64`, a budget of `max(200_000, 100 * statements)` visits, nesting 256 in the analysis, `MAX_DEPTH = 4096` in the events walk.
- **File sizes:** under about 400 lines; split before passing it. Already over or at it: `kir/src/range/events.rs` (448: Task 1 moves its statement arms to `events/stmts.rs` first), `range/report.rs` (407: don't grow it), `range/mod.rs` (397: Task 2 moves `Marker` to `range/mark.rs` first), `range/stmt.rs` (359: new arms only delegate). New code goes in new files named in each task.
- **New statements, every walker:** `ast::Stmt::Assume` gets an arm in every exhaustive AST match (`syntax/src/link/walk.rs`, `sexpr.rs`, `check/src/body/{stmt,nested,remove,consts}.rs`, `kir/src/lower/stmt.rs`; the compiler lists any others). KIR's `Stmt::Assume { id, cond, check, body }` (Task 1) and `Stmt::Prove { id, body }` (Task 3) get an arm in every exhaustive KIR `Stmt` match (`kir/src/visit.rs`, `visit_mut.rs`, `rename.rs`, `inline/descend.rs`, `lower/lines.rs`, `lower/sort.rs`, `promote.rs`, `split.rs`, `range/stmt.rs`, `range/writes.rs`, `range/events`, codegen `func/stmt.rs`; the compiler lists others). **Their bodies are treated exactly like an `If` arm: same loop depth** (in `promote.rs`, a `break`, `continue` or `return` inside one still gets its write-backs). `Assume`'s `cond` is an expression every walker visits.
- **Runtime functions** need an `RtFuncs` field and `decl` in `codegen/src/rt.rs`, a `jit_symbols()` entry, and `#[no_mangle] pub extern "C"`; a failing one fails through `panic::fail`.
- **Never a bare `link.exe`:** Git's coreutils `link.exe` shadows the MSVC linker on PATH (`find-msvc-tools` finds the real one).
- **LIGHT MODE testing** (the user's machine lags under load):
  - Every cargo command runs with `CARGO_BUILD_JOBS=2` (PowerShell: `$env:CARGO_BUILD_JOBS=2`) and tests with `-- --test-threads=2`.
  - Run focused targets only: `-p <crate>`, a named `--test`, a test-name filter, and the programs target filtered with `KESTREL_TEST_ONLY` (comma-separated file-name prefixes).
  - Run long commands at idle CPU priority under a watcher that waits for them to end; don't poll in a loop.
  - No window, GPU or demo tests (`window_smoke*`, `voxels_demo`, `demos`, `window_print_flush`).
  - **The full `cargo test --workspace --no-fail-fast` runs only in Task 5, and only when the user says the machine is free.** Until then it is owed, and every task report says so.
  - Clippy per task on the crates touched: `cargo clippy -p <crate> --all-targets -- -D warnings`. `kestrel/godot` and `kestrel/bench/kernels` are separate workspaces: `cargo check` and clippy them when a task touches an API they use.
- **Test facts:**
  - Program tests: `kestrel/tests/programs/*.kes` with `// expect:`, `// expect-stderr:` and `// expect-exit:` lines; the JIT pass runs with `KESTREL_CHECK_PROOFS=1`, the exe pass also with `KESTREL_TEST_VERIFY_EXE=1`, and the release pass (`kestrel run --release` with the proof check) fails only on "range analysis removed".
  - Release tests: `kestrel/tests/release/*.kes`, built with `--release`, run, compared; then again under `KESTREL_CHECK_PROOFS=1`.
  - Error tests: `kestrel/tests/errors/*.kes` with `// error <line>: <message>` lines (a substring of the message), run through `kestrel check --diagnostics=short`, which must exit 1. That is the checked build; `--release` contract tests go in `cli/tests/contracts.rs`.
  - No string literals inside `{...}` in program sources.
  - To keep a value unknown to the analysis, get it from a helper that isn't inlined: `id(x: int, d: int) -> int { if d == 0 { return x } \n id(x, d - 1) }`.
  - `kestrel_codegen::dump` prints a function's machine code; each check's failure path ends in `trap user2`.
  - KIR tests reuse `kir/tests/range_common/mod.rs` (`optimized`, `analysed`, `checks`, `counts`, `left_of`, `proven_of`, `line`).
- **Off-limits:** `kestrel/test.kes`, `voxels.exe`, `kestrel/b.exe`, the line-ending-only change in `kestrel/check/src/body/enums.rs`, and anything under `kestrel/bench/voxels3d/` or in `docs/superpowers/plans/2026-10-09-core-2a-followups.md` that another agent left uncommitted (the tuned voxel copy runs in parallel). Never use `git add -A`, `git stash` or `git clean`; stage files by name.
- **Commits:** plain-sentence messages ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`, on `main`; the controller pushes.

## Review Focus

1. **An assumption seen outside its block:** through its end, a `break`, a `continue` into the next iteration, an `if` arm joined with one that never ran it, a loop's back edge. (Task 2.)
2. **A trusted assumption under `--release` that a bounds check rests on:** directly, through a nested pair that each suffice, through an inlined copy, through a function the analysis gave up on. Each must stay evaluated. (Task 2.)
3. **An assumption proving more than `if cond` would:** a contradiction (unreachable block), a wrapped side under `--release`, a field in memory, a changed variable. (Task 2.)
4. **A contract that misses an item:** a header (`for x in xs.filter(...)`), a `par for` body, a call left, a cycle, a closure made in the region, a native, a function value, an unpromised interface call, an implementation of a promised one, `main`'s constants, a function the analysis gave up on or the events walk cut. (Tasks 3 and 4.)
5. **A program without annotations that changed:** its build time, its report, its code. (Tasks 3 and 5.)

---

### Task 1: The `@` syntax, and `@assume` as a run-time check — **Sonnet**

**Goal:** `@prove` and `@assume` parse with the addendum's placement rules and errors; `@prove` is recorded (and checked from Task 4); `@assume` lowers to `Stmt::Assume` holding the rest of its block and stops the program when false. Range analysis treats it as a plain block that teaches nothing (Task 2 changes that).

**Files:**
- Syntax: `syntax/src/token.rs` (`Tok::At`, symbol `@`), `lexer.rs` (`'@'`), `ast.rs` (`pub enum Contract { NoAlloc, NoChecks, NoPanic, Inline }` with `name()`; `pub struct ProveAnnot { pub target: Span, pub contracts: Vec<(Contract, Span)>, pub span: Span }`; `Program::proves: Vec<ProveAnnot>`; `Stmt::Assume { cond: Expr, text: String, span: Span }` and its `span()`), new `parser/annot.rs` (reading `@name(...)`, the errors, attaching), `parser/mod.rs` (the parser keeps the file's source text for `text`, and collects `proves`), `parser/item.rs`, `parser/members.rs`, `parser/iface.rs` (annotations before items, methods and interface methods), `parser/stmt.rs` (`@assume` statements; `@prove` before `for`, `par for`, `while`), `lib.rs` (`parse_at` hands the source to the parser), `link.rs` (concatenates `proves`), `link/walk.rs`, `sexpr.rs` (`(assume <cond>)`).
- Check: new `check/src/body/assume.rs` (the condition's form, the two checker errors), `body/stmt.rs` (type-check `cond` as `bool`, then `assume.rs`), and the other exhaustive matches.
- KIR: `kir/src/ir_stmt.rs` (`Stmt::Assume { id: u32, cond: Expr, check: AssumeCheck, body: Vec<Stmt> }`, `pub enum AssumeCheck { Check, Trust }`), `ir.rs` (`pub struct AssumeInfo { pub text: String, pub span: Span, pub line: u32 }`, `Module::assumptions: Vec<AssumeInfo>`, by `id`), new `lower/assume.rs` (the rest of a block inside the assumption; a block's value through a temporary), `lower/scopes.rs` (`scoped_block` and `block_value` lower their statements through it), every exhaustive `Stmt` match (Global Constraints); `range/stmt.rs` (`Assume`: the condition's checks are decided as an expression statement's, the body is a plain block, nothing is learned), `range/writes.rs`; `range/events.rs` split (move the statement arms to new `events/stmts.rs`), and there an `Assume` with `check: Check` is a `Fail` event "the assumption `{text}` is checked here".
- Codegen: new `codegen/src/func/assume.rs` (evaluate `cond`; false calls `kestrel_rt_assume_failed`, then `trap user2`; then the body), `func/stmt.rs`, `rt.rs`, `jit.rs`.
- Runtime: `runtime/src/panic.rs`: `pub fn assume_failed_message(text: &str, line: i32, file: &str) -> String` and `kestrel_rt_assume_failed(text: *const u8, len: i64, line: i32, file: *const FileName) -> !`.

**Interfaces produced:** the AST and KIR types above. Lowering gives each `@assume` an `id` (its index in `Module::assumptions`) and `check: Check`. Inlining copies an assumption with its `id`. A function whose body ends in a value inside an assumption still ends in `Stmt::Return(Some(Load(tmp)))`, so it stays inlinable.

- [ ] **Step 1: Write the failing tests.**
  - `syntax/tests/lexer.rs`: `at_is_a_token` (`@prove` lexes as `At`, `Ident("prove")`).
  - New `syntax/tests/parser_annot.rs`:
    - `prove_before_a_function_is_recorded`: `"@prove(no_alloc, no_checks)\npure step(x: int) -> int { x }\n"`: one `ProveAnnot`, `target` equal to the function's span, contracts `[NoAlloc, NoChecks]`.
    - `prove_goes_before_pub_methods_and_interface_methods`: `@prove(inline)` before `pub f()`, a struct method, an enum method and an interface method: four entries, each targeting that `FnDecl`'s span.
    - `prove_goes_before_loops`: `for`, `par for` and `while` in a body: three entries targeting the statements' spans.
    - `annotations_stack_and_merge`: `"@prove(no_alloc) @prove(inline)\nf() {}"` and the same one per line: the contracts of one target are `[NoAlloc, Inline]` (merged; a name listed twice counts once).
    - `the_target_may_follow_on_the_same_line` (syntax pick 1): `"@prove(inline) pure f() {}"` and `"@prove(no_checks) for i from 0 to n {}"` parse.
    - `assume_is_a_statement`: the body of `f(n: int, xs: [int]) { @assume(n <= xs.len) }` prints `{(assume (<= n (. xs len)))}`, and `text == "n <= xs.len"`; whitespace runs inside are one space.
    - `two_assumes_share_a_line`: `@assume(a > 0) @assume(b > 0)` is two statements.
  - Error tests (one file per bullet if parse recovery mixes them):
    - `tests/errors/annot_names.kes`: `@fast` before a function → "unknown annotation `@fast`; Kestrel has `@prove` and `@assume`"; `@prove(fast)` → "unknown contract `fast`; ..."; `@ prove` → "expected an annotation name after `@`, like `@prove` or `@assume`".
    - `annot_prove_empty.kes`: `@prove` and `@prove()` → "`@prove` needs at least one contract, like `@prove(no_alloc)`".
    - `annot_prove_target.kes`: before a `let`, an `if`, a struct and a constant → "`@prove` goes before a function, a method or a loop (`for`, `par for` or `while`)".
    - `annot_assume_place.kes`: a top-level `@assume(1 < 2)` and one in a struct body → "`@assume` goes inside a function, as a statement of its own"; `@assume` and `@assume()` in a body → "`@assume` needs a condition, like `@assume(i < xs.len)`".
    - `annot_assume_form.kes`: `@assume(a < b or b < c)`, `@assume(flag)`, `@assume(x < 1.5)` → "`@assume` takes comparisons of integers joined with `and` ..."; `@assume(id(n, 1) < 3)`, `@assume(xs[0] < 3)` and `@assume(int(big) < 3)` → "`@assume` can't call a function or index a list ..."; `@assume(nope < 3)` → the usual unknown-name error.
  - New `kir/tests/assume_lower.rs`:
    - `the_rest_of_the_block_goes_inside`: `f(n: int) { let a = n + 1 \n @assume(n > 0) \n print(a) \n print(n) }` lowers to `[Let, Assume { body: [Print, Print] }]`.
    - `a_tail_value_goes_through_a_temporary`: `pure g(n: int) -> int { @assume(n > 0) \n n * 2 }`: the body ends `[Assume { body: [.., Let(t, _)] }, Return(Some(Load(t)))]`.
    - `a_function_with_an_assumption_is_still_inlined`: `h(n: int) -> int { g(n) + 1 }`: after `optimize`, `h` calls nothing and `m.not_inlined` has no entry for `g`.
    - `assumptions_keep_their_text`: `m.assumptions[0].text == "n > 0"`, its line is 1.
  - `cli/tests/report.rs`: `an_assumption_is_a_way_to_fail`: the report of `pure g(n: int) -> int { @assume(n > 0) \n n * 2 }` has "line 1: can fail: the assumption `n > 0` is checked here".
  - Programs:
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
      ```
      `// expect: 60`.
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
      ```
      `// expect-stderr: assumption `n <= xs.len` is false on line 9 of assume_false.kes`, `// expect-exit: 101`.
    - `tests/programs/assume_scope.kes`: an assumption before a `return` inside an `if`, in a value function's tail (`xs[i] * 2`), in a `while true` body with a `break` after it, and in an `if` used as a value (`let v = if s > 0 { @assume(s > 0) \n s + 1 } else { 0 }`); prints the sums (`// expect: 45 46` for `xs = [4, 5, 6]` summing `pick + tail`).
    - `tests/programs/assume_inlined_false.kes`: a small helper with `@assume(i < xs.len)`, inlined into `main`, called with 7 on a 3-item array: the error names the helper's line.
    - `tests/programs/prove_parses.kes`: `@prove(no_alloc)` on a pure arithmetic function, `@prove(inline)` on a method and an interface method, `@prove(no_alloc, no_panic)` on a `for` loop that xors an array's items; prints a result. (Contracts are checked from Task 4; pick ones that hold in both builds: no `+` that could overflow, every index from `for i from 0 to xs.len`.)
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-syntax --test parser_annot --test lexer -- --test-threads=2`. Expected: `Tok::At`, `Program::proves` and `Stmt::Assume` don't exist.
- [ ] **Step 3: Implement.** Token and AST first (the compiler finds the match arms), then the parser, the checker, KIR and lowering, then codegen and the runtime. The text comes from the source between the parentheses, trimmed, runs of whitespace made one space. In `block_value`, when an assumption is among the block's statements, spill the tail value (`spill`) at the end of the innermost assumption's body.
- [ ] **Step 4: Verify.** The new tests pass; `cargo test -p kestrel-syntax -p kestrel-check -p kestrel-kir -p kestrel-codegen -p kestrel-runtime -- --test-threads=2`; `cargo test -p kestrel-cli --test errors --test report -- --test-threads=2`; `KESTREL_TEST_ONLY=assume_,prove_,range_ cargo test -p kestrel-cli --test programs -- --test-threads=2`; clippy on the touched crates; `cargo check` in `kestrel/godot` and `kestrel/bench/kernels`.
- [ ] **Step 5: Commit:** "Parse @prove and @assume, and check assumptions at run time".

### Task 2: `@assume` in range analysis — **Opus**

**Goal:** inside its block an assumption teaches what `if cond` teaches; nothing outside its block sees it; under `--release` it is trusted unless a bounds check rests on it; each copy says what it removed.

**Files:**
- New `kir/src/range/assume.rs` (the two runs; the comparison; `AssumeUse`), `range/stmt.rs` (the `Assume` arm calls it), new `range/mark.rs` (`Marker` moved out of `mod.rs`, plus `AssumeCheck::Trust`), `range/mod.rs` (`Analysis::assumptions`; a scratch decision table), `range/events/stmts.rs` (a `Trust` assumption is no way to fail).
- Codegen `func/assume.rs`: `Trust` emits nothing unless `opts.verify_proofs`.
- `cli/tests/programs.rs`: a release program may give `// expect-stderr-proofs:` and `// expect-exit-proofs:` for its run under `KESTREL_CHECK_PROOFS=1` (a trusted assumption is evaluated there).
- Tests: new `kir/tests/range_assume.rs`, new `codegen/tests/assume.rs`, programs and release programs below.

**Interfaces:**
- Consumes Task 1's `Stmt::Assume` and Round A's `condition` (the true state of a condition), `decide`, `frames`, `learned`, `keep`.
- Produces: `pub struct AssumeUse { pub id: u32, pub removed: u32, pub bounds: bool }` and `Analysis::assumptions: Vec<AssumeUse>`, one per analysed copy (own function and every function it was inlined into); `removed` counts the checks the real run proved and the plain run left, `bounds` whether one of them is a bounds check. Under `Arith::Wrapping` the marker sets `check: Trust` on every analysed copy whose `bounds` is false; under `Arith::Checked` every assumption stays `Check`; a function the analysis gave up on records nothing and changes nothing.

The rules, in the addendum's words ("`@assume`", "Soundness rules"):
- **Real run:** from the true state of `condition(st, cond)`; the assumptions inside run as written; decisions go to the real table (only in deciding passes, as now).
- **Plain run:** from `st` itself, with every `Assume` inside analysed as a plain block; decisions go to a scratch table (in deciding passes); `learned` and the loop frames are saved before the real run and restored before the plain one, so **only the plain run's `break` and `continue` states and its end reach the frames and the code after the block**. `return` goes nowhere in either.
- **Comparison:** a site decided proven in the real run and left in the plain run counts for this copy.
- A contradiction makes the real run unreachable (`keep`), so its checks stay and nothing is removed.

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
  - `a_contradiction_proves_nothing`: `if i < 0 { @assume(i > 5) \n s = xs[i] }`: stays; `removed == 0`.
  - `an_assumption_removes_an_overflow_check`: `@assume(a >= 0 and a <= 1000 and b >= 0 and b <= 1000) \n a * b`: proven under `Checked` (`removed == 1`, `bounds == false`); under `Wrapping` there is no site and `removed == 0`.
  - `release_trusts_an_arithmetic_only_assumption`: `pure d(a: int, b: int) -> int { @assume(b > 0) \n a / b }`: under `Wrapping` the division is proven and the `Assume` is `Trust`; under `Checked` it is `Check`.
  - `release_keeps_a_bounds_assumption`: `get` under `Wrapping`: `Check`.
  - `release_keeps_the_outer_of_a_redundant_pair`: two identical `@assume(i >= 0 and i < xs.len)` lines then `xs[i]`, `Wrapping`: the outer `Check`, the inner `Trust`, the index proven.
  - `release_keeps_both_of_a_needed_pair`: `@assume(i >= 0)` then `@assume(i < xs.len)` then `xs[i]`: both `Check`.
  - `an_inlined_assumption_counts_in_its_copy`: `h(xs: [int], k: int) -> int { get(xs, k) ^ 1 }`: no bounds check left in `h`; two `AssumeUse`s with `get`'s assumption's `id`.
  - `an_assumption_around_a_nested_loop`: `for k from 0 to m { @assume(n >= 0 and n <= xs.len) \n for i from 0 to n { s = s ^ xs[i] } }`: proven.
  - `a_length_assumption_proves_a_neighbour`: `pure f(xs: [u8], i: int) -> u8 { @assume(xs.len == 100 and i >= 0 and i < 99) \n xs[i + 1] }`: proven.
  - `a_field_in_memory_teaches_nothing`: a struct `Grid { n: int, cells: [int] }` and `pure f(g: Grid, i: int) -> int { @assume(i >= 0 and i < g.n and g.n <= g.cells.len) \n g.cells[i] }`: stays (Round A: fields in memory aren't terms).
  - `a_function_given_up_on_keeps_its_assumption`: 300 nested `if`s around an assumption (on a 2 MB-stack thread, as Round A's test): `gave_up`, the `Assume` stays `Check`, no `AssumeUse`.
  - `codegen/tests/assume.rs`: `a_checked_build_tests_the_assumption` (`d` has one `trap user2`: the assumption's; the division's check is gone); `a_trusted_assumption_emits_nothing` (`d` under `--release`: none); `a_kept_assumption_is_tested_under_release` (`get` under `--release`: one, the assumption's); `the_proof_check_tests_a_trusted_assumption` (a child process, as `verify_mode_still_checks_a_proof`: `--release` with `verify_proofs`, `d(1, 0)` exits 101 with "assumption `b > 0` is false on line").
  - `cli/tests/report.rs`: `a_trusted_assumption_is_no_way_to_fail_under_release` (`d`'s `--release` report has no assumption item; the checked one has it).
  - Programs that must still fail (`// expect-exit: 101`, values from `id`): `assume_keep_change.kes` (`@assume(i >= 0 and i < xs.len)` true for `i = 2`, then `i = id(3, 1)`, then `print(xs[i])`: "index 3 out of bounds for length 3 on line ..."); `assume_keep_pop.kes` (a list of 3, `i = 2`, the assumption, `xs.pop()`, then `xs[i]`: "index 2 out of bounds for length 2 ...").
  - Release programs (`tests/release/`):
    - `assume_release_bounds.kes`: `i = id(7, 1)`, `@assume(i >= 0 and i < xs.len)`, `print(xs[i])` with 3 items: "assumption `i >= 0 and i < xs.len` is false on line ..." (the same with the proof check), never a read outside the array.
    - `assume_release_pair.kes`: the redundant pair with `i = 7`: the error names the outer assumption's line.
    - `assume_release_block.kes`: `b = id(0, 1)`; `if id(1, 1) == 1 { @assume(b > 0) \n if id(0, 1) == 1 { r = 10 / b } }` (the inner division never runs; the trusted assumption removes its check), then `print(100 / b)`: "division by zero on line ..." (the division after the block keeps its check), and `// expect-stderr-proofs: assumption `b > 0` is false on line ...` for the run with the proof check.
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir --test range_assume -- --test-threads=2` and `cargo test -p kestrel-codegen --test assume -- --test-threads=2`. Expected: the proofs fail (nothing learned yet) and `AssumeUse` doesn't exist; the near misses pass and must still pass after Step 3.
- [ ] **Step 3: Implement** in the files above. Show each Critical rule caught by mutation (the task report has the table: the rule broken, the tests that failed): the plain run's exits replaced by the real run's; the plain run keeping inner assumptions; `Trust` set while `bounds` is true; the real run started without `condition`'s forgetting.
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-kir -p kestrel-codegen -- --test-threads=2` (Round A's range tests unchanged); `KESTREL_TEST_ONLY=assume_,range_ cargo test -p kestrel-cli --test programs -- --test-threads=2`; `cargo test -p kestrel-cli --test programs release_programs_run_as_executables -- --test-threads=2` (the release folder) and `--test report`. **Then the required proof check,** as Round A's Tasks 2 and 3: every program in all three passes (JIT, exe with `KESTREL_TEST_VERIFY_EXE=1`, release), one batch of `KESTREL_TEST_ONLY` prefixes at a time, at idle priority: no "range analysis removed", no release-pass timeout. The task report gives the result; the task isn't done without it.
- [ ] **Step 5: Commit:** "Let range analysis use assumptions inside their block, and trust them under --release only away from bounds checks".

### Task 3: Contract plumbing: regions, sites and the driver step — **Sonnet**

**Goal:** everything contracts need, with no contract decided yet: loop regions in KIR, every contract listed in `Module::contracts`, checks and events tagged with their regions, range analysis run once in the driver for an annotated program and reused by codegen, and the "removes no check" warning from `kestrel check`.

**Files:**
- KIR: new `kir/src/contract.rs` (`ContractSet`, `ContractSite`, `ProveTarget`; re-exported from `ir`), `ir_stmt.rs` (`Stmt::Prove { id: u32, body: Vec<Stmt> }` and its arms, Global Constraints), `ir.rs` (`Module::contracts: Vec<ContractSite>`, `Module::analysed: Option<Arith>`), `lower/stmt.rs` (an annotated `while`, `for` or `par for`: every statement its lowering pushes goes in one `Prove` block), `lower/mod.rs` (a site for each annotated function and method), `lower/iface.rs` (a site for each implementing method of an annotated interface method).
- Range: `range/mod.rs` and `range/stmt.rs` (`CheckLeft::regions`: the `Prove` ids around the site, kept beside `loop_lines`; `Prove` is a plain block), `range/events.rs` and `events/stmts.rs` (`Event::regions`; a new `What::Cut` event where the walk stops at `MAX_DEPTH`), `remove_checks` and `mark_proven` set `Module::analysed`.
- Codegen `lib.rs` (`define_module`: with `m.analysed == Some(mode)` it compiles `m` as it is; with another mode it returns the error "the module was analysed for the other build"; with `None`, as now).
- Driver: new `driver/src/prove.rs` (`prove`), `lib.rs` (`Compiled` keeps each file's base, path and text to render later diagnostics; the existing `report` function renders them).
- CLI `cli/src/main.rs`: `check`, `run` and `build` call `prove` after `compile` (only `check` asks for warnings); `check --report` reuses its analysis and prints the report even when `prove` fails; exit 1 on failure.
- Godot `godot/src/lib.rs`: calls `prove` with its checks setting; a failure is a `godot_error!` and nothing loads.
- Tests: new `kir/tests/contracts_sites.rs`, `codegen/tests/proven.rs` (two tests), new `cli/tests/contracts.rs`, new `driver/tests/prove.rs`.

**Interfaces:**
- `pub struct ContractSet(pub u8)` with `NO_ALLOC`, `NO_CHECKS`, `NO_PANIC`, `INLINE`, `contains`, `insert` (inserting `NO_PANIC` inserts `NO_CHECKS`).
- `pub struct ContractSite { pub target: ProveTarget, pub contracts: Vec<(Contract, Span)>, pub required_by: Option<String> }`; `pub enum ProveTarget { Func(FuncIdx), Loop { func: FuncIdx, line: u32 } }` (`line` is the loop keyword's line, packed); a loop's `Prove { id }` is the index of its site. `required_by` is "`Shape.area`" for a site made from an interface method, whose spans then point at the implementing method's name.
- `pub fn prove(c: &mut Compiled, checked: bool, warn: bool, style: DiagStyle) -> Proved`, `pub struct Proved { pub ok: bool, pub text: String, pub analysis: Option<Analysis> }`. With no `@prove` (and, when `warn`, no `@assume`) it returns at once, `ok`, no analysis, the module untouched. Otherwise it runs `remove_checks` in place in the mode given. With `warn` and assumptions, it also runs it on a clone in the other mode (cloned before the in-place run) and warns for each assumption id whose `AssumeUse`s, in both modes, all have `removed == 0` (at least one recorded). Contracts are not checked yet (Task 4 adds the call).

- [ ] **Step 1: Write the failing tests.**
  - `contracts_sites.rs`:
    - `function_and_method_contracts_are_listed`: `@prove(no_alloc, inline)` on `f` and `@prove(no_panic)` on a method: two sites with those targets, contracts and spans.
    - `an_annotated_loop_is_one_prove_block`: `@prove(no_alloc)` on `for x in xs.filter((e) { e > 0 }) { s = s ^ x }`: the body holds one `Stmt::Prove`, and every statement the loop lowered to (the `filter` call included) is inside it; its `id` is the loop's site, `ProveTarget::Loop` with the `for`'s line.
    - `a_par_for_and_a_while_are_regions_too`.
    - `an_interface_contract_binds_each_implementation`: `interface Shape { @prove(no_alloc) \n area(self) -> float }` with `Goblin` and `Box` implementing it: two sites, `Func(Goblin.area)` and `Func(Box.area)`, `required_by == Some("`Shape.area`")`, spans at each method's name.
    - `checks_and_events_know_their_region`: after `remove_checks` and `events`: a check in the annotated loop has `regions == [id]`, one in its header too, one after it `[]`; an allocation likewise.
    - `the_events_walk_says_where_it_stopped`: a function built by hand with 5,000 nested `If`s: its events end with `What::Cut`.
  - `codegen/tests/proven.rs`: `an_analysed_module_is_compiled_as_it_is` (analysed checked in place, compiled checked: no second analysis, the same `trap user2` count as today) and `a_module_analysed_for_the_other_build_is_refused`.
  - `driver/tests/prove.rs`: `a_program_without_annotations_is_left_alone` (`ok`, empty text, `module.analysed == None`); `an_annotated_program_is_analysed_once_in_place` (`analysed == Some(mode)`).
  - `cli/tests/contracts.rs`:
    - `an_assumption_that_removes_nothing_warns`: `@assume(n > 0)` before `print(n)`: `kestrel check` exits 0 and stderr has "warning: `@assume(n > 0)` removes no check; delete it"; `kestrel run` prints no warning.
    - `an_assumption_for_overflow_checks_does_not_warn_under_release`: `@assume(a >= 0 and a <= 1000) \n print(a * 3)`: `kestrel check --release` gives no warning (it removes a check in a checked build).
    - `annotations_do_not_change_what_runs`: `prove_parses.kes`'s output through `run` and `build`, checked and `--release`.
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir --test contracts_sites -- --test-threads=2`. Expected: `Stmt::Prove`, `Module::contracts` don't exist.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-kir -p kestrel-codegen -p kestrel-driver -- --test-threads=2`; the cli `contracts`, `report`, `errors` and `modules` targets; `KESTREL_TEST_ONLY=assume_,prove_ cargo test -p kestrel-cli --test programs -- --test-threads=2`, then the same with `KESTREL_TEST_VERIFY_EXE=1` (the in-place analysis must give the same proofs); `cli/tests/report.rs`'s exact fixture unchanged; clippy; `cargo check` and clippy in `kestrel/godot`.
- [ ] **Step 5: Commit:** "Record contracts and loop regions, and analyse annotated programs once in the driver".

### Task 4: Deciding contracts, through calls — **Opus**

**Goal:** `range::contracts::check` decides every contract in `Module::contracts` as the addendum's "What a contract covers", "Calls through interfaces and function values" and "Error messages" say, and the driver step reports what fails.

**Files:**
- New `kir/src/range/contracts.rs` (the entry and the per-contract rules), `contracts/region.rs` (a site's items: its function's checks left and events, filtered by region for a loop; `par for` bodies whole; `m.init` for `main`), `contracts/words.rs` (the messages and help lines).
- `range/summary.rs`: `Summary::checks: Option<Chain>` (the first check left in a function or below it; the reason's `via` is "has a bounds check" and its `tail` ": {check text}") and `Summary::unknown: Option<Chain>` (a give-up or a `What::Cut` in a function or below it); both by `chains`'s fixed point.
- `ir_expr.rs`, `ir_stmt.rs`: `CallVirtual { ..., keeps: ContractSet }` (9 patterns without `..`), set in `lower/iface.rs` from the interface method's `@prove`; `range/events/reasons.rs` (`virtual_call` lists no allocation when `keeps` has `NO_ALLOC` and no way to fail when it has `NO_PANIC`).
- `driver/src/prove.rs`: calls `contracts::check` and renders its errors (with help lines).
- Tests: new `kir/tests/contracts_calls.rs`, error tests, programs, `cli/tests/contracts.rs`, `cli/tests/report.rs`.

**Interfaces:**
- `pub fn check(m: &Module, a: &Analysis) -> Vec<ContractError>`, `pub struct ContractError { pub span: Span, pub message: String, pub help: Option<String> }`; one per failed (site, contract), at the contract's span, naming the first item in report order (own checks before other items on a line, as `items::arrange`), never a count.
- Per contract, the addendum's table: own items of the region; each call left (`What::Call`), closure made (`What::Closure`) and `par for` (`What::Body`, whole) followed through `Summary`; `CallIndirect` breaks all four; `CallVirtual` breaks `inline`, and the others unless `keeps` has them; a native breaks `no_alloc` and `no_panic`; `Summary::unknown`, a give-up in the site's own function or a `What::Cut` there breaks all four; a site on `main` also follows `m.init`. The report's totals are never read.

- [ ] **Step 1: Write the failing tests.**
  - Error tests (`kestrel check`, checked build), each with the exact message:
    - `prove_own_checks.kes`: `@prove(no_checks) pub total(xs: list[int], k: int) -> int` (the report fixture's `total`) → "`total` can't be proven `no_checks`: it has an overflow check on line N: `s + xs[i]` may exceed `int` ..." (the loop's line); the same body with the overflow made impossible (`s = s ^ xs[i]`, and `s ^ xs[k]` at the end) → "... it has a bounds check on line N: `xs[k]` may be outside `xs` (`k` can be any `int`)".
    - `prove_loop.kes`: `@prove(no_checks)` on `for i from 0 to n { acc = acc * 3 }` → "the loop at line N can't be proven `no_checks`: it has an overflow check on line N+1: ..."; a sibling loop on the same function that holds raises nothing.
    - `prove_header.kes`: `@prove(no_alloc)` on `for x in xs.filter((e) { e > 0 })` → "the loop at line N can't be proven `no_alloc`: it allocates on line N: `filter` builds a new list"; on `for i from 0 to xs.len` with `xs.any((e) { e > 0 })` in the body: no error.
    - `prove_no_alloc.kes`: the report fixture's `grow` → "`grow` can't be proven `no_alloc`: it allocates on line N: `add` may grow `ys`".
    - `prove_no_panic.kes`: `xs.remove_at(0)` → "... it can fail on line N: ..."; a bounds check alone also fails `no_panic` (it includes `no_checks`); an `@assume` inside → "it can fail on line N: the assumption `n <= xs.len` is checked here".
    - `prove_par_for.kes`: `@prove(no_alloc)` on a `par for` → "the loop at line N can't be proven `no_alloc`: it allocates on line N: a `par for` makes an arena for each thread"; `@prove(no_checks)` on a `par for` whose body indexes another list by its index → the body's bounds check.
    - `prove_chain_alloc.kes`: the spec's example: `step` calls `spawn` (kept a call: it calls itself) which adds to `ps` → "`step` can't be proven `no_alloc`: it calls `spawn` (line 40), which adds to `ps` (line 12) and may grow the list" (lay the file out so the lines are 40 and 12, or use the real lines).
    - `prove_chain_checks.kes`: a call to a recursive `g` with `xs[i]` → "... it calls `g` (line 5), which has a bounds check (line 9): `xs[i]` may be outside `xs` (`i` can be any `int`)".
    - `prove_inline.kes`: a call to a function with an early `return` → "`f` can't be proven `inline`: it calls `g` (line N), which isn't inlined: it has an early `return`"; a recursive call → "... it calls itself".
    - `prove_fn_value.kes`: `apply(f: (int) -> int, x: int) -> int { f(x) }` with `@prove(no_alloc, no_checks, no_panic, inline)`: four errors, "it calls the function value `f` (line N); its body isn't known here" (three) and "it calls the function value `f` (line N), which can't be inlined".
    - `prove_iface_unpromised.kes`: `@prove(no_alloc)` on a function calling `s.area()` where `Shape.area` promises nothing → "it calls a method of the interface value `s` (line N); its body isn't known here".
    - `prove_iface_impl.kes`: `interface Shape { @prove(no_alloc) \n area(self) -> float }`, `Goblin.area` builds a list → "`Goblin.area` can't be proven `no_alloc`, which `Shape.area` requires: it allocates on line N: ...", on the line of `Goblin.area`'s name.
    - `prove_native.kes`: a call to a native from a built-in module (`use draw`; `check` builds nothing) → "... it calls the native function `...` (line N); its body isn't known here".
    - `prove_main_constants.kes`: `@prove(no_alloc) main()` using a heap constant (`let TABLE = [1, 2, 3]`) → "`main` can't be proven `no_alloc`: it builds the program's constants first, which ...".
    - `prove_closure.kes`: `@prove(no_alloc)` on a function that stores a capturing closure → its environment allocation.
  - Programs that pass (and run, both builds): `prove_holds.kes` (the four contracts on an xor loop over `for i from 0 to xs.len`, `@prove(no_checks)` on `get` with its `@assume`, `@prove(no_alloc)` on a recursive function that allocates nothing, `@prove(no_alloc)` on a loop using `xs.any(...)`); `prove_iface_promised.kes` (`@prove(no_alloc, no_panic)` on `Shape.area`, two implementations that keep it, and `@prove(no_alloc, no_panic)` on a function looping over `list[Shape]` calling `s.area()`).
  - `contracts_calls.rs`: `the_checks_chain_follows_calls_and_cycles` (a cycle where one function has a check: every function in it has `checks`; a clean cycle has none); `a_give_up_is_unknown_to_every_caller`; `a_cut_walk_is_unknown_to_every_caller` (KIR built by hand); `a_promised_interface_call_is_not_an_allocation` (no `Alloc` event for it).
  - `cli/tests/contracts.rs`: `release_contracts_use_the_release_build` (`@prove(no_checks)` on `pure m(a: int, b: int) -> int { a * b }`: `kestrel check` fails exit 1, `kestrel check --release` passes, `kestrel build --release` builds, `kestrel run` exits 1 and runs nothing); `the_report_prints_when_contracts_fail` (exit 1, the report on stdout, the error on stderr); `a_function_the_analysis_gave_up_on_fails_every_contract` (300 nested `if`s with `@prove(no_alloc)`: "range analysis stopped early in it (its code nests too deeply), so nothing can be proven about it; split it into smaller functions"); `a_callee_given_up_on_fails_its_callers`; `full_diagnostics_show_the_help` (the help line for an unpromised interface call and for a function value).
  - `cli/tests/report.rs`: `a_promised_interface_call_lists_no_allocation`; the exact fixture unchanged.
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-cli --test errors --test contracts -- --test-threads=2`. Expected: no contract errors yet.
- [ ] **Step 3: Implement.** Show by mutation that each rule is held by a test (the task report has the table): the header left out of a loop region, a `par for` body not followed, a callee's `checks` chain ignored, `keeps` trusted without the implementations being checked, `unknown` ignored, `m.init` not followed.
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-kir -- --test-threads=2`; the cli `errors`, `contracts`, `report` and `modules` targets; `KESTREL_TEST_ONLY=assume_,prove_,iface_ cargo test -p kestrel-cli --test programs -- --test-threads=2`; clippy; `cargo check` in `kestrel/godot` and `kestrel/bench/kernels` (the `CallVirtual` change).
- [ ] **Step 5: Commit:** "Fail the build when a contract doesn't hold, following calls".

### Task 5: Measure and document — **Sonnet**

**Files:** `docs/superpowers/specs/2026-10-18-at-annotations-design.md` (an "As built" after the addendum), `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md` (an `@` section: `@prove`, `@assume`, the contracts, the warning, the run-time error, `no_panic` not counting running out of memory, a false trusted assumption under `--release`), `README.md` (`@prove` and `@assume` next to the report), `docs/superpowers/plans/2026-10-09-core-2a-followups.md` (close the Round B items of "Found while building range analysis"; add "Found while building `@prove` and `@assume`"), `docs/superpowers/plans/2026-10-10-handoff.md` ("Start here"). Touch `kestrel/bench/voxels3d/results.md` only if the tuned-copy agent's work there is committed (`git status`); otherwise record the numbers in the spec's "As built" only.

- [ ] **Step 1: Check first.** `git status`: if the follow-ups file or `kestrel/bench/voxels3d/` has changes that aren't this round's, stop and ask the controller (another agent's work); never stage them.
- [ ] **Step 2: Compile time, no annotations.** Build the compiler at the commit before Task 1 in a worktree at `C:\kt\rb-old` (the long-path limit) and the current one; `kestrel build` of `kestrel/bench/voxels3d/main.kes` (the frozen benchmark, no annotations), old and new taking turns, checked and `--release`, 9 runs each, medians and ranges, as Round A's table (`kestrel/bench/voxels3d/results.md`, "Compile time": 181 ms (172-195) checked, 160 ms (153-211) `--release`). Target: no difference beyond the run-to-run range. Also `kestrel check --report` of `voxels3d.kes`, checked and `--release`, identical between old and new.
- [ ] **Step 3: Compile time, with annotations.** A scratch copy of the voxel world outside the repo with `@prove(no_alloc)` on `hash`, `lattice` and `octave` and one `@assume` (adjust until the contracts hold): its `kestrel build` time against the unannotated file, same method; and `kestrel check` time with and without the assumption. Record both; say what the driver step costs.
- [ ] **Step 4: Docs,** each in its file's style, honest numbers included. The spec's "As built" lists what differs from the addendum, the rulings made during the build, the holes reviews found, and what was owed. The follow-ups file gets at least: contracts on function types; natives marked safe; `@prove` on a native in a built-in module; warnings in `kestrel run` and the Godot add-on; facts across calls (an assumption at a call site doesn't reach the callee); and the two syntax picks if the user hasn't ruled.
- [ ] **Step 5: The full suite,** only if the user says the machine is free: `cargo test --workspace --no-fail-fast` (including the exe pass without the proof check), `cargo clippy --workspace --all-targets -- -D warnings`, and clippy in `kestrel/godot` and `kestrel/bench/kernels`; otherwise record it as owed in the handoff.
- [ ] **Step 6: Commit:** "Measure and document @prove and @assume".
