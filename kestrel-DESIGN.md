# Kestrel — Language Design Notes

*A working name. A kestrel is a small falcon that hovers motionless in
mid-air while scanning the ground — fast, precise, and economical with
energy. That's the goal for this language: no wasted motion.*

This document collects the design ideas discussed so far. It's a living
draft, not a spec — some of these ideas are standard practice (borrow
checking, AOT compilation), and some are original combinations/extensions
that haven't been tried together in one language. Both are marked below.

---

## 1. Persistent cross-run optimization cache
**Status: novel combination**

Most languages force a choice:
- **AOT (ahead-of-time) compilation** — compile once, run fast, but the
  compiler has to guess how the program will actually be used.
- **JIT (just-in-time) compilation** — watch the program run and optimize
  hot paths on the fly, but re-learn everything from scratch every time
  the program starts (JVM/V8-style "warmup").

**Kestrel's approach:** after every run, the runtime writes a small
profile file next to the binary — which functions were hot, what shapes
of data they saw, which branches were taken. The next run reads that
file before executing a single instruction and starts pre-specialized.
Over repeated runs on the same machine, the program keeps getting faster
and plateaus at what a JIT would eventually reach, but without ever
paying the warmup cost more than once per machine.

Trade-off to be honest about: this only helps for programs run
repeatedly on the same machine (servers, CLIs, dev loops) — a one-shot
script gets no benefit.

## 2. Effect-tracked purity
**Status: extension of known ideas (Haskell's purity + Rust's ownership)**

Rust's borrow checker proves memory-safety at compile time so there's no
need for a garbage collector. Kestrel extends the same idea to a
function's *effects*: does it allocate, do I/O, or mutate something
another part of the program can see?

A function marked `pure` is checked by the compiler to guarantee none of
that happens. Once proven pure, the compiler is free to:
- run it early or speculatively
- run multiple calls in parallel with zero risk of data races
- cache ("memoize") results automatically
- reorder it relative to other code

This is Haskell's purity guarantee, but layered on top of Rust-style
ownership instead of replacing it — you get *both* deterministic memory
management *and* algebraic freedom for the optimizer.

## 3. Layout polymorphism
**Status: novel extension of data-oriented design**

CPUs read contiguous memory much faster than scattered memory. Game
engines exploit this with "struct-of-arrays" layouts instead of the more
natural "array-of-structs," but it's tedious and easy to get wrong by
hand.

**Kestrel's approach:** the programmer writes normal, "array of structs"
-looking code. The compiler tracks how each field is actually accessed
across the hot paths of the program and is free to silently pick a
different physical memory layout per call site — recompiling a function
against a struct-of-arrays layout if that's what the access pattern
calls for. The logical shape of your data and its physical layout become
fully decoupled.

## 4. Proof-carrying optimization
**Status: extension of known ideas (dependent types / Idris / Agda)**

Even Rust keeps some runtime safety nets (e.g. array bounds checks)
because proving they're unnecessary in general is hard. Fully
dependently-typed languages let you write proofs inline that make
certain classes of bugs structurally impossible — but they're research
tools, not practical languages.

**Kestrel's approach:** a *lightweight* proof system — not full dependent
types — focused specifically on the checks that are expensive at
runtime: bounds checks, overflow checks, aliasing checks. Example:

```
fn get_safe(arr: [i32; N], i: usize) -> i32
    where i < N
{
    arr[i]   // compiler proves this in bounds at every call site,
             // so no runtime check is emitted at all
}
```

If the compiler can't prove the `where` clause at a call site, it's a
compile error, not a runtime check — you fix the call site, or fall back
to an explicitly-checked variant of the function.

---

## What's implemented so far (this prototype)

Two backends share the same front end (lexer, parser, purity checker,
bounds-proof notes) and are semantics-identical — every example program
produces the same output, and every error case throws the same
`KestrelError` message, on either one:

- **`Kestrel.run`** — a tree-walking interpreter directly over the AST.
  This is what let the language's *semantics* get tested and iterated on
  before investing in a faster backend.
- **`Kestrel.runFast`** — compiles each function to a flat bytecode
  instruction list first, then executes it on a stack-based VM where
  variables are array-index slots instead of name-keyed object
  properties (see `docs/SYNTAX.md` for how it's built).

Both support:
- variables, arithmetic, `if`/`else`, `while`
- functions, including `pure fn` with a real (if simplified) purity
  checker: a pure function is rejected at compile time if it calls an
  impure function, does I/O, or mutates anything outside its own locals
- fixed-size arrays with `where i < N`-style bounds proofs, checked
  statically where possible and falling back to a runtime check
  otherwise (with a warning), rather than silently trusting the
  programmer
- a `print` builtin

**Honest performance status:** `runFast` is not uniformly faster than
`run` yet. Measured with `node`, best-of-N, on this machine:

| Workload | `run` (tree-walk) | `runFast` (bytecode VM) |
|---|---|---|
| Tight loop, 20M iterations, arithmetic only | 5155 ms | 3082 ms (~**40% faster**) |
| Tight loop, 3M iterations, array indexing | 869 ms | 403 ms (~**54% faster**) |
| `fib(30)`, naive recursion | 404 ms | 519 ms (~**28% slower**) |

The loop/array wins are exactly the "array slots beat dictionary-mode
property lookups" argument this design doc has always made. The
recursion regression is a different, so-far-unsolved cost: every Kestrel
function call is a real recursive JS call in the VM (`call()` calling
itself), and the per-call frame bookkeeping — even reduced to index math
on one shared stack array — still loses to how aggressively V8's own
JIT optimizes the tree-walker's small, simple, monomorphic
`evalExpr`/`execStmt` functions. Making calls cheaper (a trampoline
instead of JS recursion, or inlining small pure functions at compile
time) is the next thing to attack here, before reaching for a native
backend — a native compiler pays a *different* set of costs and this
regression would need to not still be there underneath it.

Not yet implemented (future work, roughly in priority order):
1. Making the bytecode VM's call/return path actually faster than the
   tree-walker's, not just its loops and array access
2. A native backend (LLVM/Cranelift) — see the discussion above on why
   this comes after fixing the call-overhead regression, not before
3. The persistent cross-run optimization cache
4. Layout polymorphism
5. A more general proof system beyond simple bounds checks

## Naming

"Kestrel" is a placeholder. Happy to rename — the interpreter and file
extension (`.kes`) can change with a find-and-replace once a name is
picked.
