# Plan 2a follow-ups

Carried forward from plan 2a's task reviews and final whole-branch review.
Input for writing plans 2b and 2c.

## Open language decisions (need the user)

- **Narrowing integer conversions.** `u8(300)` currently gives 44 and `u8(-1)` gives 255 (truncation, like Rust `as`). The alternatives are a runtime panic when checks are on, or saturation (as float→int already does).
- **`while true { ... return ... }` as a function's last statement** is rejected ("body can finish without a value"), because the checker doesn't treat an infinite loop as never finishing.

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

## Do in 2c

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

## Performance (from kestrel/bench/2a/results.md)

- **Division and remainder by a constant** emit a real `idiv` behind the zero and `MIN/-1` checks; Rust/C use shifts or magic multiplies. Collatz is 5.2x slower than Rust because of this (Rust with a hidden divisor: 407 ms vs Kestrel 518 ms). Fix in codegen: a non-zero constant divisor other than -1 skips the checks and strength-reduces. Power-of-two shifts + check elision done in 59dd0f2 (collatz 518 → 237 ms). Still open: magic-number multiply for other constants; `x % 2^k == 0` as a mask test.
- **Call overhead** on recursive code (fib 2.3x vs Rust): no inlining in Cranelift. Sub-project 5 (release backend) or a small KIR inliner for pure leaf functions.

## Measured

- `kestrel run tests/programs/fib.kes` (release CLI, PowerShell, median of 7) takes 16.6 ms total, against 15.8 ms for bare process start. Git Bash adds about 60 ms of spawn overhead to any measurement.
- `kestrel build` (codegen + MSVC link) takes about 100 ms beyond process start.
