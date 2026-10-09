# Loop-indexed array bounds proof: design

## Context

`bounds-heavy` (see `benchmarks/results.md`) shows a real, current 16%
runtime cost from array bounds checks on the single most common
real-world array-access pattern: a loop-indexed access
(`arr[i]` inside a loop), not a literal-indexed one. Today the
`where`-clause proof system (`kestrelc/src/where_info.rs`) only elides
the check for two narrow cases: a compile-time-literal index into a
`let`-bound array literal, and the exact `arr_param[idx_param]` pattern
inside a function carrying a matching `where idx_param < N` clause.
Neither covers `bounds-heavy`'s actual shape:

```
let arr = [ /* 20000 literals */ ];
let total = 0;
let i = 0;
while (i < 20000) {
    total = (total + arr[i]) % 1000000007;
    i = i + 1;
}
```

This is a real memory-safety-adjacent feature: a bug in the proof means
a silently-elided check that should have fired (a real out-of-bounds
read/write), not just a missed optimization. The design below is
deliberately narrow and conservative — sound by construction, not by
testing — rather than maximizing coverage.

## Scope

**In scope:** local `let`-bound array literals (compile-time-known
length) indexed by a `while`-loop counter, matching one exact recognized
shape (below). This is the shape `bounds-heavy` actually uses.

**Out of scope (deferred):** array *parameters* behind the existing
`where`-clause mechanism (`arr_param[idx_param]` extended to
loop-indexed access), `for`-loop (`RangeFor`) indexing, and any shape
with nested control flow inside the loop body. Each would need its own
follow-up design — this pass intentionally does not try to generalize
beyond the one proven-necessary case.

## Mechanism

A new check, run only when `codegen.rs` is about to compile a
`Stmt::While`. It produces a proof `(idx: Symbol, bound: i64)`, valid
for the remainder of that loop's body, if and only if **all** of the
following hold:

1. The statement immediately preceding the `while` in the same block is
   `let idx = 0;` (an exact literal `0`).
2. The while condition is exactly `idx < N` (`BinOp::Lt`, left is
   `Ident(idx)`, right is `ExprKind::Num(N)`).
3. The loop body contains **no nested control flow at all** — no
   `if`, `while`, `for`, `return`, `break`, or `continue`. Flat
   statements only (`let`, assignment, expression-statement, `print`).
4. The body's **last** statement is exactly `idx = idx + 1;`
   (`Stmt::Assign` with value `Binop::Add(Ident(idx), Num(1))`), and
   `idx` is not assigned anywhere else in the body.

If any condition fails, the proof is `None` and every access in that
loop takes today's full runtime check — no partial credit, no
heuristic fallback, matching this codebase's existing "never guess"
posture (see `typecheck.rs`'s module doc, `where_info.rs`'s doc
comment).

**Why this is sound:** `idx` starts at `0` (so `idx >= 0` always holds
by construction). The *only* mutation of `idx` anywhere in the loop is
the `+1` as the body's literal last statement. So at any point in the
body *before* that statement — which, since there's no nested control
flow, means everywhere else in the body, unconditionally — `idx` still
holds the value that satisfied `idx < N` at loop entry. No branch,
early return, or nested loop can skip the increment, run it twice, or
reach an access after it, because none of those constructs are allowed
to appear at all.

**Where the proof is stored during codegen:** a new stack, parallel to
the existing `loop_stack: Vec<(Block, Block)>` (`codegen.rs`), pushed
with the computed `Option<(Symbol, i64)>` when entering a `Stmt::While`
body and popped on exit — same lifetime pattern already used for
`loop_stack`.

**Where the proof is consulted:** a new fast-path in `gen_expr`'s
`ExprKind::Index` arm (`codegen.rs`, alongside the existing literal-index
and where-clause fast-paths). Given `target[index]`:
- If `index` is `Ident(i)`, `target` is `Ident(arr)`, the innermost
  loop-bounds-proof stack entry is `Some((proven_idx, bound))`, `i ==
  proven_idx`, and `arr`'s statically-known length (`self
  .static_array_len`, already tracked via `Slot::Array { literal_len,
  .. }`) is `>= bound as usize` — elide the check, same codegen as the
  existing literal-index fast path (direct `load`, no branch).
- Otherwise, fall through unchanged to the existing runtime check.

Decoupling the proof from a specific array symbol (it proves a fact
about `idx` alone: "0 <= idx < bound for the rest of this body") means
one proof can validate an access against *any* array literal in scope
whose length happens to cover `bound` — not just one hardcoded array,
without adding any real risk (the `>= bound` check at the access site
still gates every use).

## Data flow

No AST, parser, or `resolve.rs` changes. Everything needed (the array's
static length, the loop's shape) is already visible to `codegen.rs` by
the time it reaches the `Stmt::While` node — this reuses state codegen
already tracks (`Slot::Array { literal_len }`) rather than adding a new
pre-pass, unlike `where_info.rs`'s separate AST-analysis module (that
one runs before codegen, at the whole-function level, because it needs
to check call sites across the whole program; this one is entirely
local to a single function body's codegen, so it doesn't need to be).

## Error handling / fallback

There is no error path — an unproven access is not a compile error, it
is simply the existing runtime bounds check (unchanged behavior). The
feature can only ever remove a check that was already redundant; it
never accepts a program that would otherwise be rejected, and never
rejects one that would otherwise compile.

## Testing

Existing coverage for the literal-index and where-clause fast paths
(`kestrelc/tests/integration.rs`) is entirely black-box — correct
output, or correct compile-time/runtime rejection — with no IR
inspection harness anywhere in the codebase. This follows the same
convention, with an honest limitation noted below rather than papered
over.

- **Positive:** the exact `bounds-heavy` shape (`let arr = [...]; let i
  = 0; while (i < N) { ...arr[i]...; i = i + 1; }`) produces correct
  output, including at the last valid index (`i == N-1`) — the
  boundary a bug in this proof would most likely get wrong.

- **Negative, genuinely unsafe if wrongly proven** (these are real
  regression guards — getting the proof wrong here means an actual
  out-of-bounds access, not just a missed optimization, so a black-box
  runtime-trap assertion is meaningful, the same way
  `dynamically_out_of_bounds_index_traps_at_runtime_instead_of_reading_
  garbage` already tests the existing runtime check):
  - `idx` is reassigned a second time in the body beyond the required
    `idx = idx + 1` (e.g. an extra `idx = idx + 5;`), so the value
    used to index genuinely exceeds the array — must still trap
    cleanly, not read garbage.
  - The while condition's bound exceeds every in-scope array's static
    length (`bound > static_len`) — must still trap cleanly on the
    out-of-range tail of the loop.

- **Negative, shape-only (honest limitation):** shapes like a nested
  `if` around the access, a non-adjacent or non-zero initializer, or
  `idx = idx + 1` not being the body's last statement, are shapes the
  proof must refuse — but since the loop's own `idx < N` condition
  keeps these programs genuinely memory-safe regardless of whether the
  compiler proves it, correct output alone can't distinguish "correctly
  fell back to the runtime check" from "wrongly elided it but got away
  with it." These are covered as **compiles and produces correct
  output** regression tests only (catches a proof that panics or
  miscompiles when it shouldn't have matched), not as proof of
  non-elision. If that distinction becomes important later, it needs
  an IR-inspection test harness, which does not exist in this codebase
  today and is out of scope to add here.

- Two sequential loops in the same function reusing the same `idx`
  name must each be proven independently (the second loop's proof must
  not leak from the first) — tested via the positive case run twice in
  one function with different bounds, both producing correct output.
