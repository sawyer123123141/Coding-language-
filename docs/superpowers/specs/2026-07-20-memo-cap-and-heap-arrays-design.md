# Memoization cap + heap-allocated large array literals — design

## Status

Approved scope, not yet implemented.

## Problem

Two real bugs found while building the Kestrel-vs-C benchmark suite
(`benchmarks/results.md` has the full measured evidence):

1. **Unbounded memoization.** Any eligible `pure fn` (single scalar
   param, not a `parallel_map` callback) gets an unconditional memo
   slot (`codegen.rs:534`). A function called with a different
   argument on every call (e.g. applied over a loop counter) never
   gets a cache hit, but every call still pays to grow and insert into
   an ever-larger hash table. Measured: 200,000,000 such calls used
   3GB+ RAM and didn't finish in several minutes.
2. **Stack-allocated array literals crash on large data.**
   `codegen.rs:310`'s own comment confirms array literals are
   deliberately stack-allocated. A 500,000-element `i64` literal
   (4MB) reliably crashes with `STATUS_STACK_OVERFLOW` against
   Windows' default 1MB thread stack — no compile-time warning, an
   opaque OS-level crash at runtime.

Both are native-only (`codegen.rs`); confirmed `wasm_codegen.rs`
already bump-allocates array literals into wasm's linear memory, never
the (non-addressable) wasm call stack, so it isn't affected by #2, and
doesn't implement memoization at all (native-only feature), so it
isn't affected by #1 either.

## Fix 1: cap memo table growth per slot

Add a max-capacity constant to `kestrelc_runtime.c` (same file that
already has `KESTRELC_MEMO_MAX_ARGS`, `KESTRELC_MEMO_INITIAL_CAP`,
etc.) — e.g. `KESTRELC_MEMO_MAX_SLOT_ENTRIES`. `kestrelc_memo_store`'s
existing grow-before-insert check (`kestrelc_memo_grow`, doubling)
stops growing once a slot's capacity would exceed this cap — instead
of growing, the store silently drops the entry (same "caching is
always optional" pattern the file already uses for allocation
failures). `kestrelc_memo_lookup` is unaffected: a capped-out slot
just has a permanently full-ish table that legitimately holds no
newer entries, exactly like a real cache eviction policy without
needing to implement actual eviction.

This bounds worst-case memory to (roughly) `num_memoized_functions *
KESTRELC_MEMO_MAX_SLOT_ENTRIES * sizeof(kestrelc_memo_entry)` —
constant, not proportional to call count. A function whose calls
genuinely repeat (the case memoization exists for) still benefits
fully as long as its distinct-argument count stays under the cap; a
function called with ever-new arguments (the pathological case) now
costs a bounded, fixed amount instead of growing forever.

No eviction policy (LRU or similar) — deliberately out of scope. This
is the same posture as every other cache in this codebase: "an
optimization, capped, never a correctness dependency," not a general
LRU cache implementation.

## Fix 2: heap-allocate array literals above a size threshold

`codegen.rs`'s array-literal handling (`gen_binding`'s `ArrayLit` arm,
and any other stack_slot-based array construction) gains a size check:

- **At or below 4KB** (`elems.len() * 8 <= 4096`, i.e. up to 512
  `i64` elements): unchanged — stack-allocated via Cranelift's
  existing `StackSlot` mechanism. This covers the overwhelming
  majority of real array literals (the existing examples all use
  4-6 elements) with zero behavior change and zero overhead.
- **Above 4KB, up to 100MB**: heap-allocated via `malloc` (already
  available — `kestrelc_runtime.c` already includes `<stdlib.h>`).
  Never explicitly freed: matches the language's existing
  no-lifetime-tracking memory model (arrays are immutable,
  constructed once, no concept of scope-based cleanup exists
  anywhere in the language today) — the allocation lives for the
  process's duration, same as how a stack-allocated array's "lifetime"
  today is really just "as long as the enclosing function's stack
  frame," which for `main`-level arrays is also effectively the whole
  process. This is a pragmatic match to the existing model, not a new
  one.
- **Above 100MB**: compile error at `resolve.rs` (same stage
  `check_struct_decls` already runs at) — `"array literal '<n>
  elements' is too large to compile (over 100MB) — this is almost
  certainly a mistake"`. A safety net against a literal so large it
  would itself cause compile-time or runtime memory problems
  regardless of allocation strategy, not a meaningful real-program
  limit.

Both the small (stack) and large (heap) cases produce the exact same
`Slot::Array { ptr, len, .. }` shape codegen already uses everywhere
downstream (indexing, passing as a parameter, `parallel_map`) — `ptr`
is just a Cranelift `Variable` holding an address, agnostic to whether
that address came from a stack slot or `malloc`. No downstream code
(indexing, bounds checks, parameter passing) needs to change; only the
construction site does.

## Explicitly out of scope

- Any changes to `wasm_codegen.rs` (confirmed not affected by either
  bug).
- A real LRU/eviction policy for memoization (a cap, not eviction).
- Freeing heap-allocated arrays, or any general memory-lifetime
  tracking (matches the language's current model — a real future
  project, not this one).
- Raising the default thread stack size (rejected in favor of the
  heap-allocation approach, which fixes the root cause rather than
  moving the ceiling).

## Testing plan

- **Fix 1**: a Rust-level or integration test that calls a memo-eligible
  `pure fn` with more than `KESTRELC_MEMO_MAX_SLOT_ENTRIES` distinct
  arguments and confirms the program completes with correct output and
  bounded time/memory (not a strict memory assertion — a wall-clock
  regression test, e.g. "completes within N seconds" for a call count
  that would have taken much longer unbounded, is the practical
  signal). Re-run this session's own pathological case (the
  200,000,000-call scenario, or a scaled-down version) as a manual
  verification, same as `results.md`'s existing evidence.
- **Fix 2**: integration tests for (a) a small array literal (existing
  behavior, still stack-allocated, still correct), (b) an array literal
  just above the 4KB threshold (exercises the new heap path, correct
  output, no crash — this is the direct regression test for the
  crash this design fixes), (c) an array literal above the 100MB cap
  (clean compile error, not a crash). Re-run this session's actual
  500,000-element crashing case as manual verification that it now
  compiles and runs correctly instead of crashing.
