# Plan 2a follow-ups

Carried forward from plan 2a's task reviews and final whole-branch review.
Input for writing plans 2b and 2c.

## Language decisions (resolved 2026-10-10)

- **Narrowing integer conversions** (`u8(x)`, `int(x)` from a wider type) panic when the value doesn't fit and checks are on; `--release` truncates. A literal that doesn't fit is a compile error. Float->int still saturates.
- **`while true` with no `break`** counts as never finishing, so it can end a value-returning function.

## Do before or at the start of 2b

Done in 2b-i: labelled arguments evaluate in source order; `func/mod.rs` split; parenthesised spans include the parens; KIR has `Place` + aggregates (struct layouts, return slots, field-by-field copies); compound assignment takes its type from the place; exclusivity works on place roots. Still open from this list: recursion depth limits, interned `Ty` ids for enums, `Diagnostic` labels, `match` arm newline note, `list`/`array`/`map` type constructors, thin KIR-shape tests.


- **Labelled arguments evaluate in parameter order, not source order** (`kir/src/lower/expr.rs` `call_parts`). The ignored test `labelled_arguments_evaluate_in_source_order` in `codegen/tests/jit.rs` documents the expected behaviour. The fix needs synthetic temporaries, and the locals list must be able to grow during lowering. Struct construction with labels will hit the same problem.
- **Split `codegen/src/func/mod.rs`** (378 lines) before adding aggregates: move print/string/call helpers into their own files.
- **Parenthesised expression spans exclude the parens**, which misplaces carets on parent nodes. Fix when postfix `.`/`[]` parsing lands.
- **Recursion depth.** The CLI and runtime now run on 256 MiB stacks, but the parser, checker, lowering and codegen are still recursive with no depth limit.
- **KIR needs places and aggregates.** `Stmt::Assign(LocalIdx, _)` and `Arg::Ref(LocalIdx)` can only target a local. Fields and indexes need a `Place` (Local/Field/Index/Deref). `KTy`, `cl_type` and codegen's `Slot` model are scalar-only, and `expr() -> Value` can't return aggregates (needs destination-passing). `LocalKind::RefParam` is tied to `var`, but spec §2 also passes read-only aggregates by pointer.
- **`Ty` needs interned ids** for structs/enums. `Ty::name() -> &'static str` and `resolve_type` must take the type table, and `collect` needs a two-phase declare. The AST has no ids for patterns, field labels or variant names yet.
- **`Diagnostic` needs secondary labels** (spec §3; the arena-escape error names two lifetimes).
- **Parser notes for `match` and generics.** `newlines.rs` drops a newline before `else` and `}`, so `match` arms like `Air { false }\n else { true }` arrive without a separator, and arm parsing must not require one. `array[int](n)` / `list[Chunk]()` look like indexing followed by a call, so treat `list`/`array`/`map` followed by `[` as type constructors.
- **Compound assignment** takes the operand type from `rhs.ty`. It should come from the place once `Place` exists.
- **Thin KIR-shape tests** (while/for/break/continue/and/or/unary/convert). Add them before refactoring the KIR shape.

## Found while building the Godot demo (2026-10-10)

- **A `pure` function can't call a helper that only changes a `var` argument rooted at its own local** (for example `quad(var out, ...)` filling a local list). This is observationally pure, but the checker rejects it because the helper has `MutatesParams`. Allow calls whose `var` arguments are all rooted at the caller's locals.
- **`let x = opt else` always moves or copies.** Unwrapping an owning optional parameter is therefore an error; a view (like a `match` binding) would fix it.
- **Runtime errors don't name the file** in multi-file programs, only the line.
- **The Godot add-on is built and tested on Windows only.** The `.gdextension` file lists Linux and macOS library names, but they've never been built.

## Found while building strings anywhere (2026-10-10)

- **A `T` used as a `T?` isn't copied** (use-after-free). Lowering wraps it as `Some(Load(..))`, and `owned` treats `Some` as a fresh value. So `in h.mem { h.s = k }`, with `s: string?` or `a: [int]?`, keeps pointing into the arena `k` came from. This predates strings in arenas, which widen its reach. Fix: copy the payload of a `Some(Load(..))`.
- **Read-only copies of strings copy their bytes.** `let w = words[i]` copies, which makes `bench/maps` string keys about 35% slower. A copy that is only read could share the bytes, once the escape check models it. This includes copies of literal-backed strings: a static-data check would skip them, since literals belong to no arena.
- **A `return` inside `arena { }` can dangle.** The value is an `if` whose branch has an owning local, and the result reads a place declared outside the arena. `block_value` always uses `owned(v, false)`, but the checker's `region(.., moving = true)` assumes a returned local isn't copied. Example: `return if c { let o = Own(); x } else { "lit" }`. Fix: make the two agree on which values `block_value` copies.
- **Binary vector operators don't evaluate operands in source order.** `p.pos + shift(var p)` reads `p.pos` after the call, and binary operators have no changed-later guard. Vector methods have the guard.
- **A literal or plain-local index is still copied before a vector operation** (`vs[0].length()`). `operand` copies any indexed vector place so the index is evaluated once. This is safe but costs a copy for indexes that have no effects.
- **Precision gotchas in the new builtins.**
  - `let x: f64 = sqrt(2.0)` computes in `float`, then widens.
  - An `i64` argument to a float builtin goes through `float`.
  - Named float constants widen from `float` (`let HALF = PI / 2` is not exact against an `f64`).
  - Builtin calls don't receive the expected type, so `let b: u8 = min(7, small)` fails while `min(small, 7)` works.
- **Vector methods aren't allowed in `let` constants** (`let L = vec2(3, 4).length()`), while `sqrt`, `max` and `lerp` are.
- **Cost of `round`, trig and `exp`.** `round` and the trigonometric and exponential functions go through `kestrel_rt_math` (a call) or a short instruction sequence, so a hot loop full of `sin` pays for the call.
- **The random generator is per thread.** `rand_seed` seeds the calling thread only. Random calls are an error in `pure` functions and `par for` bodies, so this only matters if they are ever allowed there (say, one stream per iteration for repeatable parallel noise).

## Maps (built 2026-10-09)

- **Not yet:**
  - values that own an arena;
  - struct, enum and vector keys (they need `==` first);
  - `==` on maps;
  - maps across the Godot bridge (a load error says so).
- **Map constants are rebuilt at every use** (all constants are re-lowered at each use). A lookup-table constant used inside a hot function builds a fresh map each call: a review measured 616 MB for 2M calls. Hoist constants into the program arena, or build each once.
- **Lookups are runtime calls.** Inlining the probe for integer keys into generated code would remove the call; on a map that fits in the cache, the call is most of the remaining cost.
- **In-order integer keys miss the cache** where C#'s identity hash walks memory in order (2.5x behind in `kestrel/bench/maps`). The scattering hash is the safer default; probing that tolerates identity hashes (as Python's does) is the alternative.
- **`print(m, m.remove(1), m)`** prints the first `m` after the removal: print arguments are evaluated first, and the first is a header copy sharing entries. Function calls reject this through `check_changed_later`; `print` isn't covered.
- **`m.get(k, d)` copies a heap-holding default twice** (once as an argument, once when the result is stored), even when the key is found. Wasted arena memory, not wrong.
- **A loop nested over the same map** makes the outer loop re-index its value on every use (`MapLock` counts as a write). Correct, but slower.

## `par for` (built 2026-10-09)

- **Not yet:** `par for` over maps, and SIMD.
- **Reading the collection conservatively:** any read of the collection (even `xs.len`) or of an alias of what it belongs to is an error while the body changes items. A finer rule would allow reads that can't see other items.
- **A failing pool thread parks for good** (unwinding through compiled code isn't safe), and the pool starts a replacement. A failure while holding the growth lock would deadlock the others; it needs an allocation near `isize::MAX`, so it isn't reachable in practice.
- **Growth in shared arenas takes one lock per job.** It's rare (growth is geometric), but a body that grows many lists made outside the loop will contend on it.

## Do in 2c

- **Strings (2b-ii scope).** Interpolated strings work anywhere now and live in arenas (see "Found while building strings anywhere"). `==` works on strings. Still missing: length, indexing and concatenation. Strings nested in a printed struct are quoted, but `"` inside them isn't escaped.
- **Exclusivity** only checks bare identifiers (`f(var a, a + 1)` passes; 3+ `var` uses of one local give duplicate diagnostics). Redesign it around place paths once aggregates are passed by pointer.
- **Cleanup stack in codegen.** `break`/`continue`/`return` jump directly, but `arena { }` exits and scope-end drops need cleanup on every exit edge.
- **Runtime stdout buffer is thread-local.** It must change before `par for` (sub-project 3).

## Cheap, fix when the area is next touched

- `-2147483648` / `i64::MIN` literals can't be written (special-case `Neg(Int)`).
- Constants are re-lowered at every use (exponential on chained constants; an overflow in a constant traps at each use).
- Runtime: `print_str` should guard `len <= 0`; `eprintln!` in `kestrel_rt_panic` can abort on a broken pipe.
- CLI: `-o` swallows the next flag; `run`/`check` silently accept `-o`/`--release`; unknown `--` flags are ignored; the temp object file has a predictable name.
- Each runtime function is registered in three places (runtime, `RtFuncs::declare`, `jit_symbols`): generate the last two from one table. The native-libs list in `link.rs` is a hardcoded snapshot of `rustc --print native-static-libs`.
- Harnesses: `//expect:` (no space) and empty `// error N:` messages aren't caught; the caret test only checks that `^` exists; `errors.rs` duplicates `programs.rs` helpers.
- Residual f32 rounding for non-literal float expressions widened to f64 (`0.1 * 2.0 * x`). This follows the typing rules.
- Diagnostics polish: CRLF caret overcount, tab misalignment, `0x` says "too large", `42abc` lexes silently as two tokens, `pure fn foo` misses the "no `fn`" hint, the loop-variable assignment message suggests `var`.
- Add `rust-version` to the workspace (`is_none_or` needs Rust 1.82+).
- A module file that exists but isn't UTF-8 (say UTF-16, from PowerShell's `>`) is skipped while loading, and then reported as missing. Say it can't be read, and why.

## Performance (from kestrel/bench/2a/results.md)

- **Division and remainder by a constant** emit a real `idiv` behind the zero and `MIN/-1` checks; Rust/C use shifts or magic multiplies. Collatz is 5.2x slower than Rust because of this (Rust with a hidden divisor: 407 ms vs Kestrel 518 ms). Fix in codegen: a non-zero constant divisor other than -1 skips the checks and strength-reduces. Power-of-two shifts + check elision done in 59dd0f2 (collatz 518 → 237 ms). Still open: magic-number multiply for other constants; `x % 2^k == 0` as a mask test.
- **Call overhead** on recursive code (fib 2.3x vs Rust): no inlining in Cranelift. Sub-project 5 (release backend) or a small KIR inliner for pure leaf functions.
- **Checks cost 2.4x on the 3D voxel world** (kestrel/bench/voxels3d/results.md: 1.2 s checked vs 0.49 s `--release`). Every overflow check is a branch that keeps its work in the loop. Range analysis would drop many (`h & 0x7FFFFFFF` times a 31-bit constant can't overflow an `i64`), and a loop-invariant check could be hoisted with its value.
- **The KIR inliner** takes callees up to 256 nodes, at most 8 deep, with a budget of 4,096 added nodes per caller. A smarter heuristic (bonus for constant arguments, single call sites) may beat a flat limit.

## Measured

- `kestrel run tests/programs/fib.kes` (release CLI, PowerShell, median of 7) takes 16.6 ms total, against 15.8 ms for bare process start. Git Bash adds about 60 ms of spawn overhead to any measurement.
- `kestrel build` (codegen + MSVC link) takes about 100 ms beyond process start.
