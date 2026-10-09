# For-loop syntax: two-tier design

## Status

Approved via conversational brainstorming (not the full one-question-at-a-time
flow — condensed here from that discussion). Prerequisite for a future SIMD
auto-vectorization pass (deferred, separate brainstorm) — this document is
scoped to the language feature only: new syntax, normal (non-vectorized)
codegen across all three backends. No vectorization is implemented by this
plan.

## Problem

Kestrel has no `for` loop today — only `while`. A future SIMD pass needs to
recognize "simple counted loop over an array" at compile time and prove it's
safe to vectorize (fixed start, fixed end, step exactly 1, no early exit).
Pattern-matching that shape out of an arbitrary `while` loop is real,
error-prone analysis — easy to miss, easy to get wrong on unusual code.

## Goal

Add real `for` syntax as two deliberately different forms:

- **Range-for**: restricted to a fixed start/end and step-1 ascending count.
  The restriction is the point — it's what makes the shape trivially provable
  for a future SIMD pass, no analysis required, guaranteed by the grammar
  itself.
- **General-for**: unrestricted three-clause form (arbitrary init/condition/
  step). Pure syntax sugar for a `while` loop — same power, same performance,
  never a vectorization target, exists so range-for's restriction never
  blocks anyone from writing a for-loop shape at all.

Neither form regresses anything: general-for compiles to identical code to
today's `while`; range-for is new code path that (for now) also compiles to
an equivalent scalar loop — pure upside, no downside, until a future SIMD
pass gives range-for a second, faster lowering.

## Syntax

```
for i from 0 to n {
    out[i] = square(arr[i]);
}
```
Range-for. `i` is bound fresh inside the loop (i64, matches other integer
locals — no separate index type). `from`/`to` are new reserved keywords.
End is **exclusive** (loop runs `start, start+1, ..., end-1`), matching
`0 to arr.len()` covering exactly the valid index range with no off-by-one —
chosen over an inclusive end specifically because "loop over every index of
an N-element array" is the dominant real use case. Start/end are arbitrary
integer expressions, evaluated once at loop entry (not re-evaluated each
iteration). If `start >= end`, the loop runs zero times (same as a `while`
whose condition is false immediately) — not an error.

```
for i = 0, i < n, i = i + 2 {
    print(arr[i]);
}

for i = n - 1, i >= 0, i = i - 1 {
    print(arr[i]);
}
```
General-for. Three comma-separated clauses, no parens (distinct from both
C's `for(init; cond; step)` and Rust's `for x in iter`): `init` (always
`ident = expr`, declares a fresh variable scoped like any other `let`),
`cond` (arbitrary bool expression), `step` (always `ident = expr`, and the
`ident` must be the *same* identifier `init` declared — enforced as a parse
error otherwise, since the step clause exists to update the loop variable,
not an unrelated one). No new keywords — reuses `for` plus existing `=` and
`,`.

Both forms use the same `for` keyword; the parser disambiguates by whether
the token after the loop variable is `from` (range-for) or `=` (general-for).

## Architecture

### New AST node: `Stmt::RangeFor`

```rust
RangeFor { var: Symbol, start: Expr, end: Expr, body: Vec<Stmt>, span: Span }
```

Added to `ast.rs`'s `Stmt` enum. Kept as a **first-class, distinct node all
the way through codegen** — not desugared to `Let` + `While` at parse time.
This is the one non-obvious design decision worth stating explicitly: if
range-for were desugared early, a future SIMD pass would face exactly the
"is this actually a simple counted loop" analysis problem this whole feature
exists to avoid. Keeping `RangeFor` distinct means that future pass can just
pattern-match one AST variant, no proof needed — the grammar already did the
proving.

For **this** plan (no vectorization yet), every backend's `RangeFor` lowering
constructs the identical IR shape its existing `While` lowering already
does for `let var = start; while (var < end) { body...; var = var + 1; }` —
implemented as a small private per-backend helper so the logic isn't
duplicated inline, but the AST node itself stays intact for later reuse.

### General-for: parse-time desugaring, no new AST node

General-for parses directly into existing `Stmt::Let` + `Stmt::While` nodes
(`Let { name: var, value: init_expr }` followed immediately by
`While { cond, body: [...original body, Assign { name: var, value: step_expr }] }`,
both wrapped in the returned `Vec<Stmt>` in place of the single `for`
statement). Zero new AST surface, zero new resolve/purity/typecheck/codegen
work anywhere — it's handled by every existing `Let`/`While`/`Assign` code
path with no changes, because after parsing it *is* a `Let`+`While`.

### Files touched (RangeFor only — general-for touches only parser.rs)

- `lexer.rs`: three new reserved keywords, `for`, `from`, `to`.
- `ast.rs`: new `Stmt::RangeFor` variant.
- `parser.rs`: new `for` statement parsing — branches to `RangeFor` or the
  general-for desugar based on the token after the loop variable.
- `resolve.rs`: `RangeFor` arm in `resolve_stmt` — resolve `start`/`end`
  expressions, insert `var` into the (flat, function-wide — this codebase
  has no block scoping today, confirmed by reading `resolve_stmt`'s existing
  `locals: &mut HashSet<Symbol>` threading) locals set, resolve `body`.
- `purity.rs`: `RangeFor` arm wherever `While`'s two arms are (an
  effect-checking walk and a purity-checking walk) — same treatment as
  `While`'s existing arms, `start`/`end` checked like `While`'s `cond`,
  `body` checked like `While`'s `body`.
- `typecheck.rs`: `RangeFor` arm — `start`/`end` must type-check as
  integers, `var` is bound as `i64` for the body's type environment, body
  type-checked normally.
- `codegen.rs` (native AOT): `RangeFor` arm in the statement-lowering match,
  plus the three existing helper-walk functions that already have a `While`
  arm (slot-collection walk, span walk, and the main statement lowering) —
  four touch points total, matching `While`'s own four appearances in this
  file.
- `wasm_codegen.rs`: same shape, three touch points (matching `While`'s
  three appearances there).
- `jit_codegen.rs`: same shape, four touch points (matching `While`'s four
  appearances there) — **plus** adding `RangeFor` to whatever check
  currently whitelists supported statement kinds for JIT eligibility
  (`check_jit_supported`), so a `RangeFor`-using program stays JIT-eligible
  instead of silently falling back to AOT.
- `fusion.rs` / `cse.rs`: both already have an exhaustive-ish statement walk
  (`fuse_body`'s `If`/`While` match, `cse_block`'s `Stmt` match) that
  currently has no `RangeFor` arm at all — needs one in each, matching
  their existing `While` treatment (recurse into `body` with a fresh
  scope/table, same as `While`'s body already gets). Without this, a
  `RangeFor` body's contents would silently never get fused/CSE'd, not a
  correctness bug but a real missed-optimization regression versus writing
  the same loop as `while`.

### Not touched

- `wasm_codegen.rs`'s WASM path doesn't currently run `cse.rs` or
  `fusion.rs`'s output through a different pipeline — no change needed
  beyond `cse.rs`/`fusion.rs` themselves gaining the `RangeFor` arm noted
  above (both already run before the WASM/AOT split in `main.rs`/`lib.rs`).
- No changes anywhere related to `where`-clause bounds proof or SIMD
  codegen — both explicitly out of scope, separate future work.
- `devtool`'s `runner.rs` calls the same public library functions
  `main.rs`/`watch.rs` do, so it needs no direct changes — it inherits
  `RangeFor` support automatically once `jit_codegen.rs`/`codegen.rs`
  support it.

## Testing

- Lexer: `for`/`from`/`to` tokenize correctly; `from`/`to` don't break
  identifiers that happen to contain those substrings (e.g. a variable
  named `format`) — existing keyword-vs-identifier lexing already handles
  this class of case for `fn`/`for`-adjacent keywords like `pure`, no new
  mechanism needed.
- Parser: range-for produces `Stmt::RangeFor` with correct fields; general-for
  produces the exact `Let`+`While` desugaring (assert on the resulting AST
  shape, not just "it parses"); general-for's step clause targeting a
  different identifier than its init clause is a parse error with a clear
  message.
- Resolve/purity/typecheck: range-for's `var` is visible inside its body;
  using it after the loop is a resolve error exactly like a `while` loop's
  internal `let` would be (confirms it's using the same flat-scope
  mechanism, not accidentally introducing real block scoping only for this
  one construct); non-integer start/end is a type error; range-for inside a
  `pure fn` is allowed (matches `while`).
- Codegen (all three backends): a range-for summing an array produces the
  same output as the equivalent hand-written `while` loop, for AOT, WASM,
  and JIT execution paths independently (three separate tests, mirroring
  how existing `While`-based tests are already split by backend elsewhere
  in `tests/integration.rs`). A general-for produces the same output as its
  desugared equivalent (fewer tests needed here since it shares every
  downstream code path with existing `Let`/`While` tests already).
- Fusion/CSE: a `parallel_map` call inside a range-for body still
  participates in fusion (regression guard for the fusion.rs touch point
  above); a repeated pure call inside a range-for body still gets CSE'd.

## Open questions resolved during brainstorming

- **Inclusive vs. exclusive end**: exclusive, decided above.
- **Should range-for support a custom step**: no — that's exactly what
  general-for is for. Keeping range-for locked to step-1 is deliberate.
- **Should this implement SIMD too**: no, explicitly deferred as a separate
  follow-up brainstorm once this lands, so that discussion isn't stuck
  guessing at syntax that doesn't exist yet.
