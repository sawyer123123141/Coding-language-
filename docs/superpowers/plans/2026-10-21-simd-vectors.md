# SIMD Vectors Implementation Plan: `vec4`, `ivec4`, `ivec2`, `ivec3` and wrapping operators

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Build on `main`, with the ledger under `.superpowers/sdd/2026-10-21-simd-vectors/`.

**Goal:** `vec4` and `ivec4` as one-register 4-lane SIMD values (Cranelift `F32X4`/`I32X4`) with makes, lanes, swizzles, operators, built-ins, storage and per-lane overflow checks that range analysis can remove; `ivec2`/`ivec3` as plain built-in structs; one rule for every vector type; and the wrapping operators `*% +% -%`, which never check.

**Architecture:** The checker gets `Ty::Vec4`/`Ty::IVec4` (plain values) and two more built-in structs (`ivec2` = `StructId(2)`, `ivec3` = `StructId(3)`), with one table of what each vector type has (`check/src/vectors.rs`). KIR gets `KTy::F32X4`/`KTy::I32X4` (register values, one leaf each, never split), `VecLit`, `Splat`, `Shuffle`, `LaneOf`, a trailing `Proj::Lane`, and `BinOp::{AddWrap, SubWrap, MulWrap}`. Codegen maps them to Cranelift vector types and builds the per-lane checks from lane tricks (`func/simd_check.rs`). Range analysis tracks an `ivec4` local with one interval for all lanes (`range/vector.rs`).

**Tech Stack:** Rust, Cranelift 0.116.1 (x64 lowers only 128-bit vectors). Tests are Rust unit tests plus `.kes` program, error, release and CLI tests.

**Spec:** `docs/superpowers/specs/2026-10-21-simd-vectors-design.md` (from the decided proposal `docs/superpowers/plans/2026-10-21-simd-proposal.md`). Every message, rule and number comes from it; the spec wins any conflict. **Four syntax picks wait for the user** (the spec's "Syntax picks"): build them as written, each in the one place the spec names, and don't build on them elsewhere.

## Global Constraints

- **Soundness first.** Critical bugs: a removed check that could fire (a lane write that doesn't widen the range, a condition on a lane that refines the vector, a vector given a linear form or made a fact term, a wrapping result given an unwrapped range); a per-lane check that misses an overflowing lane; a vector check codegen emits that `range::sites::checks_of` doesn't list (a contract would pass wrongly); a vector split into four lanes or kept on the stack where a register would do; a vector load or store carrying Cranelift's `aligned` flag. Where a rule is unclear, keep the check.
- **Messages,** exactly (the spec's "Errors"); `{T}`, `{op}` and the like are filled in:
  - W1: "`{op}` wraps integers; for floats use `{plain}`" (`*%`/`*`, `+%`/`+`, `-%`/`-`, `*%=`/`*=`, `+%=`/`+=`, `-%=`/`-=`).
  - V1: "`{T}` has {n} components, but these values give {k}; give {n}, one value for all of them, or none for zeros".
  - V2: "`{T}` holds ints, found {ty}; convert it with `int(...)` first".
  - V3: "`{T}` takes ints and int vectors, found {ty}; convert it with `{U}(...)` first" and "`{T}` takes floats and float vectors, found {ty}; convert it with `{U}(...)` first" (`{U}` the vector of the wanted kind and the found size).
  - V4: "labels only work when every component is a number, like `vec4(x: 1, y: 2, z: 3, w: 4)`".
  - V5: "int vectors have no `/`; for a power of two use `>>`, or divide each component, like `v.x / n`". V6: "int vectors have no `%`; for a power of two use `&`, or take each component, like `v.x % n`" (also for `/=` and `%=`).
  - V7: "`{op}` can't mix {ivecN} and float; convert with `{vecN}(...)` first".
  - V8: "`{op}` doesn't compare vectors; compare components, like `a.x < b.x`".
  - V9: "`{op}` shifts every component by one `int`; shift each component on its own for different amounts".
  - V10: "`{f}` needs floats; convert with `{vecN}(...)` first". V11: "`{m}` needs a float vector; convert with `{vecN}(...)` first". V12: "`cross` needs a vec3; {vec2|vec4} has no cross product" (today's vec2 message, generalised).
  - Kept as they are: "`{n}` doesn't fit in int" (a literal), "`%` doesn't work on vectors" (float vectors), "`{op}` needs integers, found {T}", "`{T}` has no field `{name}`", "`{name}` is a built-in name; pick another", "missing field `y` for `vec2`" (its help becomes "give every component, one value for all of them, or none for zeros: `vec2()`"), the natives' "native functions can only take ..." and the export error.
  - C1 (codegen, at build): "`vec4` and `ivec4` need a CPU with SSE4.1 (every x64 CPU since about 2011); this one doesn't have it".
  - Run time, unchanged: "integer overflow on line N of f.kes", "shift amount out of range on line N of f.kes" (exit 101).
  - Report (Task 4): a vector check's head "`{what}` may exceed `int` in a lane"; a vector operand's evidence "the lanes of `{x}` {phrase}", phrases "are exactly 5", "are 0 to 99", "are 0 or more", "are at most 10", "can be any `int`" (numbers with commas from five digits, as today).
- **Numbers and names:** `IVEC2 = StructId(2)`, `IVEC3 = StructId(3)`, `BUILTIN_STRUCTS = 4` (`VEC2`/`VEC3` stay 0 and 1); `vec4`, `ivec2`, `ivec3`, `ivec4` join the reserved names in `check/src/collect.rs`. `F32X4`/`I32X4`: size 16, alignment 16, lane `i` at byte `4 * i`; `MAX_LEAVES` stays 16 (a vector is one leaf). Closure environment blocks: alignment 16 (was 8) for heap and stack environments. Lanes add left to right: `((x + y) + z) + w`. `round` halves away from zero.
- **Syntax picks** (build as written; each in one place): (1) `*%= +%= -%=` (lexer three-character table, `check/src/body/wrap.rs`); (2) hex bit patterns next to a wrapping operator (`wrap_literal_ok` in `check/src/body/wrap.rs`); (3) the shader rule for mixed constructors (`check/src/body/vec_make.rs`); (4) one rule for every vector type (`check/src/vectors.rs`).
- **File sizes:** under about 400 lines; split before passing it. Near it already: `check/src/tables.rs` (385: `vec_dims` moves to the new `check/src/vectors.rs`; don't grow it), `check/src/body/structs.rs` (289: `swizzle` moves to new `body/lanes.rs`), `check/src/body/mod.rs` (316), `kir/src/range/mod.rs` (392), `range/report.rs` (407), `range/terms.rs` (395), `range/stmt.rs` (363), `range/writes.rs` (348) (new arms in those only delegate to new `range/vector.rs`), `syntax/src/ast.rs` (338), `syntax/src/lexer.rs` (275), `codegen/src/func/print.rs` (252), `func/place.rs` (242), `func/arith.rs` (212). New code goes in the new files each task names.
- **New KIR forms, every walker:** `ExprKind::{VecLit, Splat, Shuffle, LaneOf}`, `Proj::Lane` and the three wrapping `BinOp`s get an arm in every exhaustive match (`kir/src/visit.rs`, `visit_mut.rs`, `rename.rs`, `inline/descend.rs`, `split.rs`, `promote.rs`, `stack_envs.rs`, `lower/lines.rs`, `lower/sort.rs`, `range/eval.rs`, `range/writes.rs`, `range/events`, `range/show.rs`, codegen `func/expr.rs` and `func/place.rs`; the compiler lists others). **`Proj::Lane` is only ever a place's last step:** `split.rs` and `promote.rs` keep a trailing `Lane` when they replace a field path by a leaf local (`leaves::field_path` stops before it), and codegen loads and stores a lane of a register local with `extractlane`/`insertlane`.
- **Vector memory access:** every load and store whose type may be `F32X4`/`I32X4` goes through one codegen helper, `mem_flags(ty)` (in `codegen/src/func/simd.rs`: `MemFlags::new().with_notrap()` for vectors, `MemFlags::trusted()` otherwise): `load_place`, `store_place`, `copy_fields`, `assign`'s leaf stores, list and map element access, environments, interface boxes, the par-for and print paths. Grep `MemFlags::trusted()` (90 uses) and convert each one a field, element or capture type reaches.
- **Never a bare `link.exe`:** Git's coreutils `link.exe` shadows the MSVC linker on PATH (`find-msvc-tools` finds the real one).
- **LIGHT MODE testing** (the user's machine lags under load):
  - Every cargo command runs with `CARGO_BUILD_JOBS=2` (PowerShell: `$env:CARGO_BUILD_JOBS=2`) and tests with `-- --test-threads=2`.
  - Run focused targets only: `-p <crate>`, a named `--test`, a test-name filter, and the programs target filtered with `KESTREL_TEST_ONLY` (comma-separated file-name prefixes).
  - Run long commands at idle CPU priority under a watcher that waits for them to end; don't poll in a loop.
  - No window, GPU or demo tests (`window_smoke*`, `voxels_demo`, `demos`, `window_print_flush`).
  - **The full `cargo test --workspace --no-fail-fast` runs only in Task 7, and only when the user says the machine is free.** Until then it is owed, and every task report says so.
  - Clippy per task on the crates touched: `cargo clippy -p <crate> --all-targets -- -D warnings`. `kestrel/godot` and `kestrel/bench/kernels` are separate workspaces: `cargo check` and clippy them when a task changes `KTy`, `ExprKind` or an API they use (Tasks 1, 3).
- **Test facts:**
  - Program tests: `kestrel/tests/programs/*.kes`, expectation lines (`// expect:`, `// expect-exit:`, `// expect-stderr:`) **at the end of the file**, so line numbers count from the top as written. The JIT pass runs with `KESTREL_CHECK_PROOFS=1`, the exe pass also with `KESTREL_TEST_VERIFY_EXE=1`, and the release pass (`programs_verify_release`) runs every program with `--release` and fails only on a failed proof or contract.
  - Release tests: `kestrel/tests/release/*.kes` (`release_programs_run_as_executables` in `cli/tests/programs/release.rs`): built with `--release`, run, compared, then again under `KESTREL_CHECK_PROOFS=1`.
  - Error tests: `kestrel/tests/errors/*.kes` with `// error <line>: <message>` (`<message>` is a substring), run through `kestrel check --diagnostics=short`, exit 1. Each needs a `main() {}`.
  - Values print as today: `2.0` prints `2`; several `print` values are separated by one space; a vector prints `vec4(x: 1.5, y: 3, z: 4.5, w: 6)`.
  - No string literals inside `{...}` in program sources.
  - To keep a value unknown to the analysis or a call from being inlined, use a recursive helper: `id(x: int, d: int) -> int { if d == 0 { return x } \n id(x, d - 1) }`.
  - `kestrel_codegen::dump(&m, opts, name)` prints a function's Cranelift IR and machine code (`codegen/tests/dump.rs`); each check's failure path ends in `trap user2`; a stack slot shows as `explicit_slot`. `codegen/tests/common` has `optimized`, `jit` and `call0`.
  - KIR range tests reuse `kir/tests/range_common/mod.rs` (`optimized`, `analysed`, `checks`, `counts`, `left_of`, `proven_of`, `line`).
- **Off-limits:** `kestrel/test.kes`, `voxels.exe`, `kestrel/b.exe`, the line-ending-only change in `kestrel/check/src/body/enums.rs`, the uncommitted edit of `docs/superpowers/plans/2026-10-21-simd-proposal.md` (the controller's), and `kestrel/bench/voxels3d/` (frozen; R24). Never use `git add -A`, `git stash` or `git clean`; stage files by name.
- **Commits:** plain-sentence messages ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`, on `main`; the controller pushes.

## Review Focus

1. **A removed vector check that could fire:** a lane write (`v.y = big`) not widening `v`'s range; `if v.x < 10` or `@assume(v.x < 10)` narrowing `v`; a vector local made a term or given a linear form (`va.plus(...)` on a vector); a `Splat`/`VecLit`/`Shuffle`/`LaneOf` range not covering every lane; a wrapping result given its unwrapped range; the shift amount's check decided from the vector instead of the amount. (Tasks 1, 4.)
2. **A per-lane check that is wrong:** it misses an overflowing lane in any position, or fires on a good one; the edges `MAX + 1`, `MIN - 1`, `-MIN`, `46341 * 46341` (overflow), `46340 * 46341`, `-65536 * 32768` (exactly `MIN`, fine), `65536 * 32768` (overflow), `MIN * -1`; under `Verify` it reports a failed proof, not an overflow. (Task 3.)
3. **A site and its check out of step:** codegen emits a vector check that `sites::checks_of` doesn't list, or the reverse (the report and `@prove` would be wrong). (Tasks 3, 4.)
4. **A vector split or spilled:** `scalarize`, `split.rs` or `promote.rs` breaking a vector into lanes or dropping a trailing `Lane` step; a hot vector kept in a stack slot; a vector load or store with `aligned`; a layout not 16-aligned (struct field, list block, optional payload, map entry, environment, interface box, global, stack slot). (Task 3.)
5. **Order and once-only evaluation:** the parts of a mixed constructor, `vs[f()].x += 1`, a scalar operand that fills lanes, `a.dot(b)` with side effects. (Tasks 2, 3, 5.)
6. **Programs without the new types that changed:** `vec2`/`vec3` code (the particle step's dump keeps no `explicit_slot`), the frozen voxel world's `--report` and build time, the `* %` tokens next to existing `%=`. (Tasks 1, 2, 7.)

---

### Task 1: Wrapping operators `*% +% -%` — **Sonnet**

**Goal:** `a *% b`, `a +% b`, `a -% b` and `*%= +%= -%=` on `int`, `i64` and `u8`, wrapping in every build with no check; hex bit patterns next to them (syntax pick 2). Useful alone for scalar hashes. (Int vectors get them in Tasks 2 and 3.)

**Files:**
- `syntax/src/token.rs` (`Tok::StarPercent`, `PlusPercent`, `MinusPercent`, `StarPercentEq`, `PlusPercentEq`, `MinusPercentEq`, their symbols, all in `continues_line`), `lexer.rs` (the three-character forms `*%=`, `+%=`, `-%=` are tried before the two-character `*%`, `+%`, `-%`; `%=` and `%` unchanged), `ast_ops.rs` (`BinaryOp::{MulWrap, AddWrap, SubWrap}` with `symbol()` and `AssignOp`s mapping to them), `parser/expr.rs` (`MulWrap` in `mul_expr`, `AddWrap`/`SubWrap` in `add_expr`), the statement parser's compound-assignment table, `sexpr.rs`.
- New `check/src/body/wrap.rs`: the operands (integers meet at one type as `*` does; W1 for floats and float vectors; int vectors arrive in Tasks 2-3), and `wrap_literal_ok` (pick 2: a hex literal operand up to `0xFFFFFFFF` in `int`, `0xFFFFFFFFFFFFFFFF` in `i64`, `0xFF` in `u8`, means its bit pattern; the literal's value is rewritten to the wrapped number for KIR). `body/ops.rs` and `body/place.rs` (compound forms) delegate to it. The lexer must keep the literal's spelling (hex or not) visible to the checker: add `is_hex: bool` to the integer literal node if it isn't there.
- KIR: `ir_expr.rs` (`BinOp::{AddWrap, SubWrap, MulWrap}`), `lower/expr.rs`, `lower/stmt.rs` (compound), every exhaustive `BinOp` match; `range/sites.rs` (no check for them), `range/arith.rs` (result `iv::settle(exact, ty, false)` in every build; a linear form only when `exact.fits(ty)`), `range/show.rs` (`*%`, `+%`, `-%`). Codegen `func/arith.rs`: plain `iadd`/`isub`/`imul` on the operand type (`u8` included).

- [ ] **Step 1: Write the failing tests.**
  - `syntax/tests/lexer.rs`: `wrapping_operators_are_tokens` (`a *% b +% c -% d` and `h *%= 3 +%= -%=` lex to the new tokens with adjacent spans; `a %= b` and `a % b` lex as before; `a * %` is `Star`, `Percent`).
  - `syntax/tests/parser.rs`: `wrapping_operators_bind_like_their_plain_ones`: `a +% b *% c` prints `(+% a (*% b c))`; `a *% b ^ c` prints `(^ (*% a b) c)`; `a -% b -% c` is left to right; `h *%= 3` is a compound assignment.
  - New `check/tests/wrap.rs`: the result types (`int *% int` int, `int *% i64` i64, `u8 +% u8` u8, a literal takes the other side's type); `1 *% 0x9E3779B1` checks as `int`; `1 *% 0x1FFFFFFFF` gives "`8589934591` doesn't fit in int"; `let x = 0x9E3779B1` alone still doesn't fit.
  - `tests/programs/wrap_ops.kes` (runs in the checked JIT pass; none of these stop):
    ```
    pure mix32(h: int) -> int {
        var x = h
        x = (x ^ (x >> 16)) *% 0x45D9F3B
        x = (x ^ (x >> 16)) *% 0x45D9F3B
        x ^ (x >> 16)
    }

    main() {
        let lo = -2147483647 - 1
        print(2147483647 +% 1, lo -% 1, 65536 *% 65536)
        let big: i64 = 9223372036854775807
        print(big +% 1)
        let a: u8 = 200
        print(a +% 100, a -% 201, u8(16) *% 16)
        var h = 12345
        h *%= 0x5BD1E995
        h +%= 2147483647
        h -%= -5
        print(h)
        print(mix32(1), mix32(123456789))
    }
    ```
    expect `-2147483648 2147483647 0`, `-9223372036854775808`, `44 255 0`, `1300820529`, `824515495 1952335732`.
  - `tests/programs/wrap_hex.kes` (pick 2, one file so a change is cheap): `print(1 *% 0x9E3779B1, 0x9E3779B1 *% 5)`, `var h = 7 \n h *%= 0x9E3779B1 \n print(h)`, `let w: i64 = 3 \n print(w *% 0x9E3779B97F4A7C15)` → `-1640531535 387276917`, `1401181143`, `-2691343689449507777`.
  - `tests/programs/prove_wrap.kes`: `@prove(no_checks, no_panic)` on `mix32` above (shifts by literals are proven) plus a `main` printing `mix32(1)`: builds and prints `824515495` in both builds.
  - `tests/release/wrap_release.kes`: `wrap_ops.kes`'s body under `--release`, same output.
  - `tests/errors/wrap_ops.kes`: `1.5 *% 2.0` → W1 with `*%`/`*`; `var f = 1.0 \n f +%= 2.0` → W1 with `+%=`/`+=`; `vec2(1, 2) -% vec2(1, 1)` → W1 with `-%`/`-`; `let x = 0x9E3779B1` → "`2654435761` doesn't fit in int".
  - New `kir/tests/range_wrap.rs`: `a_wrapping_op_has_no_check` (`pure f(a: int, b: int) -> int { a *% b }`: no check left, none removed, both builds); `its_result_has_the_wrapped_range` (`pure f(h: int) -> int { (h & 255) *% 3 + 1 }`: no check left; `pure g(h: int) -> int { h *% 3 + 1 }`: the `+ 1` keeps its overflow check).
  - `codegen/tests/dump.rs`: `a_wrapping_multiply_never_traps` (checked build of `pure f(a: int, b: int) -> int { a *% b }`: no `trap user2`; `a * b`: one).
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-syntax --test lexer --test parser -- --test-threads=2`. Expected: the tokens don't exist.
- [ ] **Step 3: Implement.** Tokens and AST first (the compiler lists the matches), then the parser, checker, KIR, range analysis, codegen.
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-syntax -p kestrel-check -p kestrel-kir -p kestrel-codegen -- --test-threads=2`; `cargo test -p kestrel-cli --test errors --test report -- --test-threads=2`; `KESTREL_TEST_ONLY=wrap_,prove_wrap,bitwise,arith cargo test -p kestrel-cli --test programs -- --test-threads=2`; `cargo test -p kestrel-cli --test programs release_programs_run_as_executables -- --test-threads=2`; clippy on the touched crates; `cargo check` in `kestrel/godot` and `kestrel/bench/kernels`.
- [ ] **Step 5: Commit:** "Add the wrapping operators *%, +% and -%".

### Task 2: One rule for every vector; `ivec2` and `ivec3` — **Sonnet**

**Goal:** `ivec2`/`ivec3` as built-in structs of ints; the fill, mixed (pick 3) and conversion makes for `vec2 vec3 ivec2 ivec3`; int-vector operators (`+ - *` checked per component as `int`, `& | ^ ~ << >>`, `-v`, the wrapping operators); `==`/`!=`; lane-by-lane built-ins and `sum()` (pick 4); the errors V1-V12. `vec4`/`ivec4` don't exist yet, but the table and the checker paths are written for them (Task 3 adds two rows).

**Files:**
- New `check/src/vectors.rs`: `pub struct VecKind { pub dims: u32, pub int: bool, pub simd: bool }`, `Tables::vector(ty) -> Option<VecKind>` (replaces `vec_dims`, moved out of `tables.rs`), `vector_ty(dims, int) -> Ty`, the names, and what each kind has (operators, built-ins, methods).
- `check/src/tables.rs` (`IVEC2`, `IVEC3`, `BUILTIN_STRUCTS = 4`), `collect.rs` (`add_builtin_vectors` adds `ivec2 {x, y: int}`, `ivec3 {x, y, z: int}`; reserved names), `lib.rs` (exports).
- New `check/src/body/vec_make.rs` (positional makes: zero, fill, copy, convert, mixed by the shader rule; V1-V4; labelled makes keep `check_construct`), `body/call.rs` (vector names go there before `check_construct`), `body/structs.rs` (`check_construct` keeps labels and its help text change), new `body/lanes.rs` (`swizzle` moved from `structs.rs`; results by `vector_ty`; int vectors give int vectors).
- New `check/src/body/vec_ops.rs` (`vec_binary` moved from `ops.rs` and extended: the table in the spec's "Operators", V5-V9, a scalar operand coerced to the lane type, `==`/`!=` giving `bool`), `body/ops.rs` (unary `-`/`~` on vectors, the routing), `body/place.rs` (compound forms on vectors: `/=` and `%=` on int vectors give V5/V6).
- New `check/src/body/vec_builtins.rs` (lane-by-lane `min max clamp abs floor ceil round sqrt lerp` with a number filling any argument; `sum`; the float-only methods V11, `cross` V12); `body/builtins.rs` routes vector arguments there; `builtins.rs` adds `Builtin::Sum` to `METHODS`.
- `check/src/body/consts.rs` (`is_const_expr` accepts the new makes and swizzles).
- KIR: `kir/src/lower/vector.rs` (component type from the struct's field type instead of `KTy::F32`; int operators per component through the scalar paths, so each gets its normal check), new `lower/vec_make.rs` (fill, mixed, convert: each part evaluated once, in order, then its components read), new `lower/vec_builtins.rs` (per component through `math` and `int_call`; `sum` left to right; `==` as `And` of component `Eq`s on spilled operands, `!=` its `Not`).
- Tests: `check/tests/vectors.rs`, `check/tests/messages.rs` and `tests/errors/construction.kes` (line 2 becomes `vec3(1, 2)`, error V1; `vec3(1)` now fills), new programs and error tests below.

- [ ] **Step 1: Write the failing tests.**
  - `check/tests/vectors.rs`: `ivec_types_and_swizzles` (`ivec3(1, 2, 3).zx` is `ivec2`, `.x` is `int`; `vec2(1, 2).xyx` is `vec3`); `makes_resolve` (fill, copy, convert and mixed each type as the target).
  - `check/tests/messages.rs`: `a_vector_needs_every_component_or_none` updated: `vec3(1, 2)` → V1 "`vec3` has 3 components, but these values give 2; give 3, one value for all of them, or none for zeros"; `vec2(x: 1)` → "missing field `y` for `vec2`" with the new help; `vec3(1)` is fine.
  - `tests/programs/vector_fill.kes`:
    ```
    main() {
        print(vec2(0.5), vec3(2), ivec2(7), ivec3(-1))
        let v2 = vec2(1, 2)
        print(vec3(v2, 3), vec3(0.5, v2))
        let iv = ivec3(3, -4, 5)
        print(vec3(iv), ivec3(vec3(1.9, -1.9, 2.5)))
        print(ivec2(), ivec3(), ivec2(x: 4, y: 5))
    }
    ```
    expect `vec2(x: 0.5, y: 0.5) vec3(x: 2, y: 2, z: 2) ivec2(x: 7, y: 7) ivec3(x: -1, y: -1, z: -1)`, `vec3(x: 1, y: 2, z: 3) vec3(x: 0.5, y: 1, z: 2)`, `vec3(x: 3, y: -4, z: 5) ivec3(x: 1, y: -1, z: 2)`, `ivec2(x: 0, y: 0) ivec3(x: 0, y: 0, z: 0) ivec2(x: 4, y: 5)`.
  - `tests/programs/ivec_ops.kes`:
    ```
    main() {
        let a = ivec3(1, 2, 3)
        let b = ivec3(10, 20, 30)
        print(a + b, b - a, a * 2, 3 * a, -a)
        print(b & 6, a | ivec3(8), a ^ b, ~a, b >> 1, a << 2)
        print(a *% 2, ivec2(2147483647, 0) +% 1)
        print(a.zyx, a.xz, a.x + a.y, a == ivec3(1, 2, 3), a != a)
        var c = a
        c.y = 9
        c += ivec3(1)
        print(c)
    }
    ```
    expect `ivec3(x: 11, y: 22, z: 33) ivec3(x: 9, y: 18, z: 27) ivec3(x: 2, y: 4, z: 6) ivec3(x: 3, y: 6, z: 9) ivec3(x: -1, y: -2, z: -3)`, `ivec3(x: 2, y: 4, z: 6) ivec3(x: 9, y: 10, z: 11) ivec3(x: 11, y: 22, z: 29) ivec3(x: -2, y: -3, z: -4) ivec3(x: 5, y: 10, z: 15) ivec3(x: 4, y: 8, z: 12)`, `ivec3(x: 2, y: 4, z: 6) ivec2(x: -2147483648, y: 1)`, `ivec3(x: 3, y: 2, z: 1) ivec2(x: 1, y: 3) 3 true false`, `ivec3(x: 2, y: 10, z: 4)`.
  - `tests/programs/ivec_overflow.kes`: `let a = ivec2(5, 2147483647)` then `print(a + 1)` → `// expect-stderr: integer overflow on line N`, `// expect-exit: 101`.
  - `tests/programs/vector_math.kes`:
    ```
    main() {
        let v = vec3(-1.5, 2.25, 9)
        print(abs(v), floor(v), ceil(v), round(v))
        print(sqrt(vec2(4, 9)), min(v, 1), max(v, vec3(0)), clamp(v, -1, 2))
        print(lerp(vec2(0, 10), vec2(10, 20), 0.5))
        let iv = ivec3(-3, 4, 7)
        print(abs(iv), min(iv, 5), max(iv, ivec3(0)), clamp(iv, 0, 5))
        print(v.sum(), iv.sum(), vec2(1, 2) == vec2(1, 2))
    }
    ```
    expect `vec3(x: 1.5, y: 2.25, z: 9) vec3(x: -2, y: 2, z: 9) vec3(x: -1, y: 3, z: 9) vec3(x: -2, y: 2, z: 9)`, `vec2(x: 2, y: 3) vec3(x: -1.5, y: 1, z: 1) vec3(x: 0, y: 2.25, z: 9) vec3(x: -1, y: 2, z: 2)`, `vec2(x: 5, y: 15)`, `ivec3(x: 3, y: 4, z: 7) ivec3(x: -3, y: 4, z: 5) ivec3(x: 0, y: 4, z: 7) ivec3(x: 0, y: 4, z: 5)`, `9.75 8 true`.
  - `tests/programs/vector_make_order.kes`: a mixed make `vec3(next(var n), side(var n))` with helpers that print and change `n` shows the parts run once, left to right.
  - `tests/errors/vector_rules.kes`, one line each: `ivec3(1, 2)` V1; `ivec2(1.5)` V2; `ivec3(vec2(1, 2), 3)` V3 (`ivec2(...)`); `vec3(ivec2(1, 2), 3)` V3 the float form (`vec2(...)`); `vec3(x: 1, vec2(1, 2))` V4; `ivec2(1, 2) / 2` V5; `ivec2(1, 2) % 2` V6; `var q = ivec2(1, 2) \n q /= 2` V5; `ivec2(1, 2) * 0.5` V7; `vec3(1, 2, 3) < vec3(1, 2, 3)` V8; `ivec2(1, 2) >> ivec2(1, 1)` V9; `sqrt(ivec2(4, 9))` V10; `ivec2(1, 2).length()` V11; `vec2(1, 2).cross(vec2(1, 2))` V12 (`vec2`); `vec2(1, 2) & 1` "`&` needs integers, found vec2"; `let ivec2 = 1` "`ivec2` is a built-in name; pick another".
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-check --test vectors --test messages -- --test-threads=2`. Expected: `ivec2` is unknown.
- [ ] **Step 3: Implement.** The table first, then makes, operators, built-ins, then KIR. `vec2`/`vec3` code that compiled before must lower to the same KIR (the particle step's dump test and `scalarize` tests unchanged).
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-check -p kestrel-kir -p kestrel-codegen -- --test-threads=2` (`codegen/tests/dump.rs` and `vectors.rs` unchanged); `cargo test -p kestrel-cli --test errors -- --test-threads=2`; `KESTREL_TEST_ONLY=vector,ivec_,zero_vectors,scalarize,struct_constants,math_generic,wrap_ cargo test -p kestrel-cli --test programs -- --test-threads=2`; clippy on `check`, `kir`.
- [ ] **Step 5: Commit:** "Add ivec2 and ivec3, and one rule for making and using every vector".

### Task 3: `vec4` and `ivec4` as SIMD values — **Opus**

**Goal:** Everything in the spec's "The types" to "Storage, layout and calls" for `vec4`/`ivec4`, except the built-in functions (Task 5): types, makes (all forms), lanes, swizzles, every operator with the per-lane checks (R6-R8), `==`, conversions, printing and interpolation, storage and calls, SSE4.1 (C1). Range analysis lists every vector check as a site and keeps it (Task 4 removes them).

**Files:**
- Checker: `ty.rs` (`Ty::Vec4`, `Ty::IVec4`: names, `from_name`, not aggregates), `vectors.rs` (two rows, `simd: true`), `body/lanes.rs` (`.x .y .z .w` of a 4-lane vector recorded in a new `Tables::lanes: HashMap<ExprId, u32>`; 4-letter swizzles), `body/place.rs`, `lists.rs`, `regions.rs`, `pop_owner.rs`, `captures.rs` (a lane of a place is a place, like a field), `natives.rs` (refused, today's message), `collect_ty.rs`.
- KIR: `ir.rs` (`KTy::F32X4`, `KTy::I32X4`, `is_vector`, `lane`, `Proj::Lane(u8)`), `ir_expr.rs` (`VecLit(Vec<Expr>)`, `Splat(Box<Expr>)`, `Shuffle(Box<Expr>, [u8; 4])`, `LaneOf(Box<Expr>, u8)`), `layout.rs` and `lower/layout.rs` (16/16), `leaves.rs` (one leaf), `lower/env.rs` and codegen `func/fnvalue.rs` (environment blocks and stack environments aligned 16), new `lower/simd.rs` (makes, lanes, swizzles, operators: a number operand becomes `Splat`; a shift keeps its `I32` amount; `abs` comes in Task 5), `lower/mod.rs` (`kty`), `split.rs`, `promote.rs` (trailing `Lane`, Global Constraints), every walker.
- Range analysis (sites only): `range/sites.rs` (`checks_of`: `Binary(Add | Sub | Mul)` and `Unary(Neg)` of `I32X4` are `Overflow`, `Shl`/`Shr` of `I32X4` are `Shift`, in a checked build), `range/interval.rs` (`of_type(I32X4)` is the `int` range), `range/arith.rs` (vectors: decide the site as not proven, no linear form), `range/show.rs` (texts of the new forms: `vec4(a, b, c, d)`, `ivec4(n)` for a `Splat`, `v.wzyx`, `e.x`; `type_name` of a vector is its name).
- Codegen: `func/mod.rs` (`cl_type`), new `func/simd.rs` (`mem_flags`; `VecLit` as a `splat` of lane 0 then `insertlane`s; `Splat`; `Shuffle` through a little-endian `bitcast` to `I8X16`; `LaneOf`; lane loads and stores; vector `Binary`, `Unary`, `Cast`, `Eq`/`Ne` with `vall_true`), new `func/simd_check.rs` (R6's checks for `+`, `-`, `*`, `-v`, the scalar shift amount, through `fail_if` so `Emit`, `Skip` and `Verify` all work), `func/place.rs` (`place_ty`, `place_addr` with `Lane`, a register root's lane), `func/expr.rs`, `func/print.rs` (lanes with the type's pieces), `lib.rs` or `isa.rs` (C1 when a module uses a vector type and the ISA lacks `has_sse41`), `export.rs` (`ExportTy::Other("vec4")`, `"ivec4"`).
- Tests: new `kir/tests/simd.rs`, new `codegen/tests/simd.rs`, programs, release and error tests below.

- [ ] **Step 1: Write the failing tests.**
  - `kir/tests/simd.rs`: `vector_layouts` (`size_align(F32X4) == (16, 16)`; `struct P { id: u8, pos: vec4 }` has `pos` at 16, size 32, align 16; a `list[vec4]` element is 16 bytes; `vec4?` has its payload at 16; `map[int, vec4]`'s `val_off` and `entry_align` are 16; a closure capturing a `u8` and a `vec4` has the `vec4` at a multiple of 16); `vector_math_is_one_value` (`pure f(a: vec4, b: vec4) -> vec4 { a * b + a }` lowers to `Binary(Add, Binary(Mul, ..), ..)` of `F32X4`, no `StructLit`); `a_number_operand_is_splat` (`v * 2.0`); `a_swizzle_is_a_shuffle` (`v.wzyx` is `Shuffle(_, [3, 2, 1, 0])`, `v.xy` is a `vec2` `StructLit` of two lane reads); `a_lane_write_is_a_place` (`v.x = 1.0` assigns to a place ending in `Lane(0)`); `scalar_replacement_keeps_vectors_whole` (a local `struct P { pos: vec4, vel: vec4 }` splits into two `F32X4` locals and no `F32` local, and `p.pos.y` reads become `Lane(1)` of the `pos` leaf).
  - `codegen/tests/simd.rs`: `vec4_math_stays_in_registers` (dump of `f` above, `--release`: contains `f32x4` and `fmul`, no `explicit_slot`); `an_ivec4_add_has_one_check` (checked dump of `pure g(a: ivec4, b: ivec4) -> ivec4 { a + b }`: one `trap user2`; `--release`: none; `a +% b` checked: none); `vector_calls_follow_the_abi` (JIT: `pass(a: vec4, b: vec4, c: vec4, d: vec4, e: ivec4, n: int) -> vec4 { if n == 0 { return a + b + c + d + vec4(e) } \n pass(b, c, d, a, e, n - 1) }` called through `t() -> float { pass(vec4(1), vec4(2), vec4(3), vec4(4), ivec4(5), 3).w }` gives `15.0`); `lanes_through_function_values_and_interfaces` (JIT: a `(vec4) -> vec4` closure value and an interface method returning `vec4`).
  - `check/tests/vectors.rs`: `vec4_types` (`vec4(1).xy` is `vec2`, `ivec4(1).wzyx` is `ivec4`, `vec3(1, 2, 3).xyzz` is `vec4`, `vec4(1).w` is `float`).
  - `tests/programs/vec4_basics.kes`:
    ```
    main() {
        let a = vec4(1, 2, 3, 4)
        let b = vec4(0.5)
        let c = a * b + a
        print(c, c.x)
        var d = c
        d.w = 9
        d.x += 1
        print(d, d.wzyx, d.xy, d.xyz, vec3(1, 2, 3).xyzz, vec4())
        print(vec4(vec3(1, 2, 3), 4), vec4(vec2(1, 2), 3, 4), vec4(vec2(1, 2), vec2(3, 4)))
        print(a / 2, 12 / a, -a, a - 1, 1 - a)
        print(a == vec4(1, 2, 3, 4), a != vec4(1, 2, 3, 5), a == vec4(1, 2, 3, 5))
        print("v = {a}")
    }
    ```
    expect `vec4(x: 1.5, y: 3, z: 4.5, w: 6) 1.5`; `vec4(x: 2.5, y: 3, z: 4.5, w: 9) vec4(x: 9, y: 4.5, z: 3, w: 2.5) vec2(x: 2.5, y: 3) vec3(x: 2.5, y: 3, z: 4.5) vec4(x: 1, y: 2, z: 3, w: 3) vec4(x: 0, y: 0, z: 0, w: 0)`; `vec4(x: 1, y: 2, z: 3, w: 4) vec4(x: 1, y: 2, z: 3, w: 4) vec4(x: 1, y: 2, z: 3, w: 4)`; `vec4(x: 0.5, y: 1, z: 1.5, w: 2) vec4(x: 12, y: 6, z: 4, w: 3) vec4(x: -1, y: -2, z: -3, w: -4) vec4(x: 0, y: 1, z: 2, w: 3) vec4(x: 0, y: -1, z: -2, w: -3)`; `true true false`; `v = vec4(x: 1, y: 2, z: 3, w: 4)`.
  - `tests/programs/ivec4_basics.kes`:
    ```
    main() {
        let h = ivec4(1, 2, 3, 4)
        let k = (h ^ (h >> 1)) & 0xFFFF
        print(k, ~h, h | 8, h -% ivec4(5))
        print(h *% 0x5BD1E995, h +% 2147483647)
        print(ivec4(7), ivec4(), ivec4(ivec3(1, 2, 3), 4), h.wzyx, h.xy, h.xyz)
        print(vec4(k) / 2.0, ivec4(vec4(1.9, -1.9, 3000000000.0, -3000000000.0)))
        print(ivec4(vec4(sqrt(-1.0))), int(sqrt(-1.0)))
        print(h + 1, h - h, h * h, -h, h << 2, h >> 1)
        print(h == ivec4(1, 2, 3, 4), h != ivec4(1, 2, 3, 4))
    }
    ```
    expect `ivec4(x: 1, y: 3, z: 2, w: 6) ivec4(x: -2, y: -3, z: -4, w: -5) ivec4(x: 9, y: 10, z: 11, w: 12) ivec4(x: -4, y: -3, z: -2, w: -1)`; `ivec4(x: 1540483477, y: -1214000342, z: 326483135, w: 1866966612) ivec4(x: -2147483648, y: -2147483647, z: -2147483646, w: -2147483645)`; `ivec4(x: 7, y: 7, z: 7, w: 7) ivec4(x: 0, y: 0, z: 0, w: 0) ivec4(x: 1, y: 2, z: 3, w: 4) ivec4(x: 4, y: 3, z: 2, w: 1) ivec2(x: 1, y: 2) ivec3(x: 1, y: 2, z: 3)`; `vec4(x: 0.5, y: 1.5, z: 1, w: 3) ivec4(x: 1, y: -1, z: 2147483647, w: -2147483648)`; `ivec4(x: 0, y: 0, z: 0, w: 0) 0`; `ivec4(x: 2, y: 3, z: 4, w: 5) ivec4(x: 0, y: 0, z: 0, w: 0) ivec4(x: 1, y: 4, z: 9, w: 16) ivec4(x: -1, y: -2, z: -3, w: -4) ivec4(x: 4, y: 8, z: 12, w: 16) ivec4(x: 0, y: 1, z: 1, w: 2)`; `true false`.
  - `tests/programs/vec4_storage.kes`: a struct `Body { id: u8, pos: vec4, vel: vec4 = vec4(0, 1, 0, 0) }`, `b.pos += b.vel`, `b.pos.w = 0`, `bump(var b.pos)` (`bump(var v: vec4) { v.x += 1 }`), `print(b)` → `Body(id: 1, pos: vec4(x: 2, y: 3, z: 3, w: 0), vel: vec4(x: 0, y: 1, z: 0, w: 0))` for `pos: vec4(1, 2, 3, 4)`; a `list[vec4]` grown to 3 by `add(vec4(i))`, `vs[2].y = 7`, printed (`[vec4(x: 0, y: 0, z: 0, w: 0), vec4(x: 1, y: 1, z: 1, w: 1), vec4(x: 2, y: 7, z: 2, w: 2)]`) with `vs.has(vec4(1))` (`true`); `[ivec4(1), ivec4(2)][1].z` (`2`); a closure `(v: vec4) -> vec4 { v * 2 }` and one capturing `k = vec4(1, 2, 3, 4)` returning `k + 1`; `pass(...)` from the codegen test (`vec4(x: 15, y: 15, z: 15, w: 15)`); a `map[int, vec4]` value read back with `let ... else`; a `vec4?`; a constant `let K = ivec4(1, 2, 3, 4)`; `par for v in vs { v = v * 2 \n v.w = 1 }` then print. Write the expectations from these values.
  - Overflow programs (checked, each `// expect-exit: 101` and "integer overflow on line N" with N the operation's line; the bad lane moves): `ivec4_add_overflow.kes` (lane y: `ivec4(0, 2147483647, 0, 0) + ivec4(0, 1, 0, 0)`), `ivec4_sub_overflow.kes` (lane z: `ivec4(0, 0, -2147483647 - 1, 0) - 1`), `ivec4_mul_overflow.kes` (lane w: `ivec4(1, 1, 1, 46341) * ivec4(1, 1, 1, 46341)`), `ivec4_neg_overflow.kes` (lane x: `-ivec4(-2147483647 - 1, 0, 0, 0)`), `ivec4_shift_overflow.kes` (`ivec4(1) << id(32, 3)` → "shift amount out of range on line N"). And `ivec4_mul_edges.kes`, which must not stop: `print(ivec4(46340, -65536, -2147483647 - 1, 2147483647) * ivec4(46341, 32768, 1, -1))` → `ivec4(x: 2147441940, y: -2147483648, z: -2147483648, w: -2147483647)`, and the same with the lanes rotated.
  - `tests/release/ivec4_wraps.kes`: under `--release`, `ivec4(2147483647, 1, 2, 3) + 1` → `ivec4(x: -2147483648, y: 2, z: 3, w: 4)`, `ivec4(65536) * 65536` → zeros, `-ivec4(-2147483647 - 1)` → `ivec4(x: -2147483648, y: -2147483648, z: -2147483648, w: -2147483648)`.
  - `tests/errors/vec4_rules.kes`: `vec4(1, 2, 3)` V1; `ivec4(1.5)` V2; `ivec4(1) / 2` V5; `ivec4(1) % 2` V6; `ivec4(1) * 0.5` V7; `vec4(1) < vec4(2)` V8; `ivec4(1) >> ivec4(1)` V9; `vec4(1) & 1` "`&` needs integers, found vec4"; `vec4(1) % 2.0` "`%` doesn't work on vectors"; `vec4(1) *% 2` W1; `vec3(1, 2, 3).w` "`vec3` has no field `w`"; `vec4(1) == vec3(1)` "`==` can't mix vec4 and vec3".
  - `tests/errors/prove_vector_sites.kes`: `@prove(no_checks)` on `pure f(a: ivec4, b: ivec4) -> ivec4 { a + b }` (body on line 3) → "`f` can't be proven `no_checks`: it has an overflow check on line 3" (the wording after it is Task 4's).
  - `check/tests/natives.rs`: a built-in module's native taking a `vec4` gets "native functions can only take ...".
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir --test simd -- --test-threads=2`. Expected: `KTy::F32X4` doesn't exist.
- [ ] **Step 3: Implement.** Checker, then KIR and every walker, then codegen. Then show by mutation that the check tests hold the checks (the task report has the table): each of R6's four checks weakened (the `*` check on the low pair only; `vall_true` for `vany_true`; the sign test on `a ^ r` only) is caught by a program above; a load with `MemFlags::trusted()` on an 8-aligned environment is caught by `vec4_storage.kes` under `KESTREL_TEST_VERIFY_EXE=1` or argued in the report.
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-check -p kestrel-kir -p kestrel-codegen -- --test-threads=2`; `cargo test -p kestrel-cli --test errors --test report --test contracts -- --test-threads=2`; `KESTREL_TEST_ONLY=vec4_,ivec4_,vector,ivec_,wrap_,closure,iface,par cargo test -p kestrel-cli --test programs -- --test-threads=2`, then the same with `KESTREL_TEST_VERIFY_EXE=1`; `cargo test -p kestrel-cli --test programs release_programs_run_as_executables -- --test-threads=2`; clippy on the touched crates; `cargo check` and clippy in `kestrel/godot` and `kestrel/bench/kernels`.
- [ ] **Step 5: Commit:** "Add vec4 and ivec4 as SIMD values with per-lane overflow checks".

### Task 4: Range analysis and the report for `ivec4` — **Opus**

**Goal:** The spec's "Range analysis" and "Report and `@prove`" (R16-R22): `ivec4` checks removed when the widest lane range proves them, the report's lane wording, and contracts over vector code.

**Files:**
- New `kir/src/range/vector.rs`: the ranges of `VecLit` (hull), `Splat` (its value's), `Shuffle` and `LaneOf` (their vector's), a `Lane` load of a tracked local (its range), a vector `Cast` from `F32X4` (any `int`), `Math(Min | Max)` on `I32X4` (ready for Task 5), and the vector arms of `arith` (`+ - *`, `-v`, bit operators, shifts with the amount's own range, the wrapping operators: the scalar interval rules on lane ranges; `exact.fits(I32)` decides the site; never a linear form); a lane store to a tracked local joins its range with the stored value's.
- `range/tracked.rs` (`I32X4` locals that hold their own value are tracked, with a slot, but never a node), `range/terms.rs` (no term is a vector or a place with a `Lane` step), `range/refine.rs` (a comparison with a vector or a `Lane` load refines nothing), `range/eval.rs`, `range/arith.rs`, `range/stmt.rs`, `range/writes.rs` (arms delegate to `vector.rs`; a `Lane` store writes its root).
- `range/show.rs` (the vector head "in a lane" and the lanes phrases, Global Constraints; numbers with commas).
- Tests: new `kir/tests/range_simd.rs`, `cli/tests/report.rs`, programs and error tests below.

- [ ] **Step 1: Write the failing tests.**
  - `kir/tests/range_simd.rs` (checked unless named):
    - `masked_lanes_lose_their_checks`: `pure f(h: ivec4) -> ivec4 { let k = h & 0xFFFF \n k * 3 + ivec4(0, 1, 0, 1) }`: no check left, two removed.
    - `an_unbounded_product_keeps_its_check`: `pure f(a: ivec4, b: ivec4) -> ivec4 { a * b }`: one overflow check left.
    - `splats_and_makes_carry_ranges`: `ivec4(n & 255) * ivec4(n & 255)`, `ivec4(n & 255, 0, 0, 1) * 1000` and `ivec4(n & 255).y * 1000` (a scalar product of a lane): no check left.
    - `a_lane_write_widens_the_range`: `var v = ivec4(0) \n v.y = n \n v + 1` keeps its check with `n` a parameter; with `v.y = 5`, removed.
    - `a_lane_condition_teaches_nothing`: `if v.x < 10 { print(v * 1000) }` with `v` a parameter keeps the check; the same with `@assume(v.x < 10)` keeps it.
    - `the_shift_amount_decides_the_shift_check`: `h >> n` keeps one shift check; `h >> 13` none; `h >> (n & 31)` none.
    - `a_wrapping_lane_result_wraps`: `(h & 255) *% 3 + 1` no check; `h *% 3 + 1` keeps the `+`.
    - `a_converted_float_is_any_int`: `ivec4(v) + 1` with `v: vec4` keeps its check.
    - `vectors_are_never_terms`: a loop `for i from 0 to n { let v = ivec4(i) \n out[i] = (v + 1).x }` analyses with no vector term (assert on `Tracked`/terms through the test helper) and `out[i]`'s bounds check removed as before.
    - `release_has_no_vector_checks`: under `--release` none of the above list a vector check.
  - `cli/tests/report.rs`: `vector_checks_read_as_lanes`: `kestrel check --report` of `pure f(a: ivec4, h: ivec4) -> ivec4 { a * (h & 0xFFFF) }` and `main() {}` has "may exceed `int` in a lane", "the lanes of `a` can be any `int`" and "are 0 to 65,535" on one item line.
  - `tests/errors/prove_vector_checks.kes`: `@prove(no_checks)` on the `f` of `an_unbounded_product_keeps_its_check` → "`f` can't be proven `no_checks`: it has an overflow check on line 3: `a * b` may exceed `int` in a lane (the lanes of `a` can be any `int`; the lanes of `b` can be any `int`)". Task 3's `prove_vector_sites.kes` stays.
  - `tests/programs/prove_vector_hash.kes` (holds in both builds):
    ```
    @prove(no_alloc, no_checks, no_panic, inline)
    pure hash4(x: ivec4, y: ivec4, seed: int) -> ivec4 {
        var h = x *% 0x27D4EB2D ^ y *% 0x165667B1 ^ seed *% 0x5BD1E995
        h = (h ^ (h >> 15)) *% 0x2C1B3C6D
        h = (h ^ (h >> 12)) *% 0x297A2D39
        h ^ (h >> 15)
    }

    main() {
        print(hash4(ivec4(0, 1, 0, 1), ivec4(0, 0, 1, 1), 7))
    }
    ```
    expect `ivec4(x: 226841987, y: 1939458299, z: 504135051, w: 220676886)`.
  - Soundness programs (checked; they must stop, also under `KESTREL_CHECK_PROOFS=1`): `ivec4_lane_condition.kes` (`var v = ivec4(1, 2147483647, 0, 0)`; `if v.x < 10 { print(v + 1) }` → overflow on that line, exit 101); `ivec4_lane_write.kes` (`var v = ivec4(0)`, `v.y = id(2147483647, 3)`, `print(v + 1)` → overflow).
  - `tests/programs/ivec4_range.kes`: the masked-hash shape of `masked_lanes_lose_their_checks` over a loop, printing a checksum (compute it from the program as written), so the JIT pass verifies every removal.
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 cargo test -p kestrel-kir --test range_simd -- --test-threads=2`. Expected: every vector check left.
- [ ] **Step 3: Implement.** Then show by mutation that each rule is held by a test (the task report has the table): a lane write that replaces the range instead of joining it; refine through a lane; a vector made a term; a linear form on a vector `+ constant`; a `Splat` given the full range (precision only); a wrapping op given its exact range.
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-kir -- --test-threads=2` (Round A's and Round B's range tests unchanged); the cli `errors`, `report` and `contracts` targets (the exact report fixture unchanged); `KESTREL_TEST_ONLY=ivec4_,vec4_,prove_,range_,assume_ cargo test -p kestrel-cli --test programs -- --test-threads=2`, then the same with `KESTREL_TEST_VERIFY_EXE=1`; `cargo test -p kestrel-cli --test programs release_programs_run_as_executables -- --test-threads=2`. **Then the required proof check,** as Round A's and B's: every program in all three passes (JIT, exe with `KESTREL_TEST_VERIFY_EXE=1`, release), one batch of `KESTREL_TEST_ONLY` prefixes at a time, at idle priority: no "range analysis removed", no release-pass timeout. The task isn't done without it. Clippy on `kir`.
- [ ] **Step 5: Commit:** "Let range analysis remove ivec4 checks with one range for all lanes".

### Task 5: Built-in functions on `vec4` and `ivec4` — **Sonnet**

**Goal:** The spec's "Built-in functions" for the SIMD types, each one instruction or a short sequence with no call: `min max clamp abs floor ceil round sqrt lerp`, `sum`, `dot`, `length`, `normalized`, `distance_to`.

**Files:**
- `check/src/vectors.rs` (the rows say what `vec4`/`ivec4` have), `check/src/body/vec_builtins.rs` (V10-V12 for them).
- New `kir/src/lower/simd_builtins.rs`: `Math` on `F32X4` (`Sqrt Floor Ceil Round Abs Min Max`) and `I32X4` (`Min Max`); `clamp` as `min(max(x, lo), hi)`; an int vector's `abs` as `max(v, -v)` with `v` evaluated once (its check is the `Neg`'s); `lerp` as `a + (b - a) * Splat(t)`; `sum` as `LaneOf` adds left to right (checked on `ivec4`); `dot` as the `sum` of a product; `length` as `sqrt` of `dot(v, v)`; `normalized` as `v / Splat(len)` when `len > 0`, else zeros; `distance_to` as the `length` of the difference.
- New `codegen/src/func/simd_math.rs`: `sqrt`, `floor`, `ceil`, `fabs`, `fmin`, `fmax`, `smin`, `smax` on vectors; `Round` as the scalar formula with vector operations (`trunc`, `fsub`, `fabs`, `fcmp` to a mask, a copysign of 1.0 built with `band`/`bor` on the sign bit through `bitcast`, `bitselect`); `func/math.rs` routes vectors there.
- `range/vector.rs`: `Math(Min | Max)` on `I32X4` (if Task 4 didn't already).
- Tests below.

- [ ] **Step 1: Write the failing tests.**
  - `tests/programs/vec4_math.kes`:
    ```
    main() {
        let v = vec4(-1.5, 2.25, 9, 0.5)
        print(abs(v), floor(v), ceil(v), round(v))
        print(sqrt(vec4(4, 9, 16, 0)), min(v, 1), max(v, vec4(0)), clamp(v, -1, 2))
        print(lerp(vec4(0), vec4(10, 20, 30, 40), 0.25))
        let iv = ivec4(-3, 4, 7, 0)
        print(abs(iv), min(iv, 5), max(iv, ivec4(0)), clamp(iv, 0, 5))
        print(v.sum(), iv.sum(), vec4(1, 2, 3, 4).dot(vec4(2)), vec4(2, 0, 0, 0).length())
        print(vec4(0, 3, 0, 4).normalized(), vec4().normalized(), vec4(1).distance_to(vec4(1, 1, 1, 3)))
        print(round(vec4(2.5, -2.5, 0.49999997, -0.5)))
        print(round(2.5), round(-2.5), round(0.49999997), round(-0.5))
    }
    ```
    expect `vec4(x: 1.5, y: 2.25, z: 9, w: 0.5) vec4(x: -2, y: 2, z: 9, w: 0) vec4(x: -1, y: 3, z: 9, w: 1) vec4(x: -2, y: 2, z: 9, w: 1)`; `vec4(x: 2, y: 3, z: 4, w: 0) vec4(x: -1.5, y: 1, z: 1, w: 0.5) vec4(x: 0, y: 2.25, z: 9, w: 0.5) vec4(x: -1, y: 2, z: 2, w: 0.5)`; `vec4(x: 2.5, y: 5, z: 7.5, w: 10)`; `ivec4(x: 3, y: 4, z: 7, w: 0) ivec4(x: -3, y: 4, z: 5, w: 0) ivec4(x: 0, y: 4, z: 7, w: 0) ivec4(x: 0, y: 4, z: 5, w: 0)`; `10.25 8 20 2`; `vec4(x: 0, y: 0.6, z: 0, w: 0.8) vec4(x: 0, y: 0, z: 0, w: 0) 2`; `vec4(x: 3, y: -3, z: 0, w: -1)`; `3 -3 0 -1` (the vector `round` agrees with the scalar one lane by lane: halves away from zero, and `0.49999997` stays below).
  - `tests/programs/ivec4_abs_overflow.kes`: `abs(ivec4(1, 2, -2147483647 - 1, 3))` → "integer overflow on line N", exit 101.
  - `tests/errors/vec4_builtins.kes`: `floor(ivec4(1))` V10 (`vec4(...)`); `ivec4(1).dot(ivec4(1))` V11; `vec4(1).cross(vec4(1))` V12 (`vec4`); `sin(vec4(1))` "`sin` needs a number here, found vec4".
  - `codegen/tests/simd.rs`: `vector_math_makes_no_calls` (`--release` dumps of `round`, `floor`, `min`, `clamp` and `normalized` on `vec4` and `min` on `ivec4`: the IR has no `call`, and `f32x4`/`i32x4` appear).
  - `kir/tests/range_simd.rs`: `min_and_max_keep_lane_ranges` (`min(h & 255, 100) * 1000` on `ivec4`: no check left).
- [ ] **Step 2: Run them.** `CARGO_BUILD_JOBS=2 KESTREL_TEST_ONLY=vec4_math cargo test -p kestrel-cli --test programs -- --test-threads=2`. Expected: the checker refuses the calls.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify.** The new tests; `cargo test -p kestrel-check -p kestrel-kir -p kestrel-codegen -- --test-threads=2`; `cargo test -p kestrel-cli --test errors -- --test-threads=2`; `KESTREL_TEST_ONLY=vec4_,ivec4_,vector,math cargo test -p kestrel-cli --test programs -- --test-threads=2`; clippy on the touched crates.
- [ ] **Step 5: Commit:** "Add the lane-by-lane and across-lane built-ins for vec4 and ivec4".

### Task 6: Measure — **Opus**

**Goal:** Honest numbers for what `vec4`/`ivec4` buy on 2D value noise: four corners at once and four samples at once, against the same noise in scalar Kestrel and in Rust (scalar and SSE4.1 intrinsics), checked and `--release`. All five give the same checksum.

**Files:** new `kestrel/bench/kernels/noise_scalar.kes`, `noise_corners.kes`, `noise_wide.kes`; new `kestrel/bench/kernels/src/noise4.rs` (scalar and `std::arch::x86_64` versions, used from `src/bin/rust.rs` through `#[path = "../noise4.rs"] mod noise4;`, modes `noise_scalar` and `noise_wide`); `measure.py` (the new kernels and modes); `results.md` (a section "SIMD vectors (2026-10-2x)"). No compiler code changes; a bug found goes back to the task that owns it.

**The kernels** (one noise, three shapes; `SEED = 1337`; `n` a multiple of 4):
- `hash(x, y, seed) -> int` as `prove_vector_hash.kes`'s `hash4`, on `int`s; `lattice(h) = float(h & 0xFFFF) / 65535.0`; `smooth(t) = t * t * (3.0 - 2.0 * t)`.
- `noise(x, y)`: `cx = x >> 4`, `cy = y >> 4`, `fx = smooth(float(x & 15) / 16.0)`, `fy` the same; corners `a` at `(cx, cy)`, `b` at `(cx + 1, cy)`, `c` at `(cx, cy + 1)`, `d` at `(cx + 1, cy + 1)`; `top = a + (b - a) * fx`, `bottom = c + (d - c) * fx`, result `top + (bottom - top) * fy`.
- `noise_scalar.kes`: four `hash` calls. `noise_corners.kes`: one `hash4` of `ivec4(cx) + ivec4(0, 1, 0, 1)` and `ivec4(cy) + ivec4(0, 0, 1, 1)`, `vec4(h & 0xFFFF) / 65535.0`, the blend from its lanes. `noise_wide.kes`: four samples `x .. x + 3` at once: four `hash4` calls (one per corner) over lanes of samples, the blend lane by lane in `vec4`s.
- `pub bench(n: int, iters: int) -> float`: `var sum: f64 = 0.0`; for `k` in `0 .. iters`, for each sample `i` in `0 .. n`, `x = i & 1023`, `y = (i >> 10) + k`; `sum += noise(x, y)` in sample order (the wide kernel adds its four lanes `x, y, z, w` one by one, so the f64 sum is the same); return `float(sum)`.
- Rust: the same arithmetic in `f32`, wrapping `i32` (`wrapping_mul`, arithmetic `>>`), the sum in `f64` in sample order; `noise_wide` with `_mm_mullo_epi32`, `_mm_xor_si128`, `_mm_srai_epi32`, `_mm_and_si128`, `_mm_add_epi32`, `_mm_cvtepi32_ps` and the `_ps` arithmetic under `#[target_feature(enable = "sse4.1")]`, checked with `is_x86_feature_detected!`.

- [ ] **Step 1: Equal checksums.** `CARGO_BUILD_JOBS=2 cargo build --release` in `kestrel/bench/kernels`; run each Kestrel kernel (checked and `--release`) and both Rust modes at `--n 4096 --iters 4`: one checksum for all seven runs. A mismatch is a bug to find (float order, the shift, the wrap) before any timing.
- [ ] **Step 2: Look at the code.** `target/release/kernels.exe noise_wide.kes --release --dump noise` (and `hash4`): `pmulld`, `pxor`, `psrad` on whole registers, no `explicit_slot`, no lane extracts in the hash; the corners kernel's lane reads are where they're expected. Record surprises.
- [ ] **Step 3: Time.** On a quiet machine (ask the controller first), `py measure.py --rounds 9 --only noise --threads 1`, checked and `--release` columns, ns per sample, medians with ranges, as the earlier sections of `results.md`. Then once more taking turns with the scalar kernel.
- [ ] **Step 4: Write it up** in `results.md`: the table, the speedups (wide against scalar, corners against scalar, each against Rust), what the checked build costs on vector code, and what the dump showed. Say plainly where Kestrel loses and why; each gap goes to the follow-ups file with a line ("Found while measuring SIMD vectors").
- [ ] **Step 5: Verify and commit.** `cargo clippy --all-targets -- -D warnings` in `kestrel/bench/kernels`. Commit: "Measure vec4 and ivec4 on value noise".

### Task 7: Docs — **Sonnet**

**Files:** `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md` (the Types table rows `vec2 vec3 vec4` and `ivec2 ivec3 ivec4` (which are SIMD, sizes, alignment); the "Vectors" bullet (makes, fill, mixed, conversions, lanes, swizzles, the operator table, `==`); the "Operators" bullet (`*% +% -%` and their compound forms, precedence, "never checked"); the "Overflow" bullet (int vectors per lane; wrapping operators); "Built-in functions" (the lane-by-lane functions and the vector methods with `sum`)); `README.md` (a short vectors and wrapping-operators part in its style); `docs/superpowers/specs/2026-10-21-simd-vectors-design.md` ("As built": what differs from the spec, the rulings made while building, what reviews found, the measured numbers in one paragraph, what is owed); `docs/superpowers/plans/2026-10-09-core-2a-followups.md` (a section "Found while building SIMD vectors": Godot exports of the four types; natives taking `vec4`/`ivec4`; lane-by-lane comparisons and `select`; per-lane shift amounts; hex bit patterns for `& | ^`; `/` and `%` on int vectors; anything Tasks 3-6 recorded; the four syntax picks while the user hasn't ruled); `docs/superpowers/plans/2026-10-10-handoff.md` (a new "Start here (2026-10-2x): SIMD vectors built" above the 2026-10-21 one, with the commits, the numbers, the picks waiting for the user, and next steps).

- [ ] **Step 1: Check first.** `git status`: if the follow-ups file or the handoff has changes that aren't this round's, stop and ask the controller; never stage them.
- [ ] **Step 2: Docs,** each in its file's style, in GDScript terms where the user reads them, honest numbers included.
- [ ] **Step 3: The full suite,** only if the user says the machine is free: `cargo test --workspace --no-fail-fast`, `cargo clippy --workspace --all-targets -- -D warnings`, and clippy in `kestrel/godot` and `kestrel/bench/kernels`; and the frozen voxel world's `kestrel check --report` (checked and `--release`) byte-identical to the one before Task 1 (no vector code in it). Otherwise record each as owed in the handoff.
- [ ] **Step 4: Commit:** "Document SIMD vectors and the wrapping operators".
