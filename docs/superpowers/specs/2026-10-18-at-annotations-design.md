# `@` annotations: performance contracts

Status: designed 2026-10-18, alongside closures (`2026-10-18-closures-design.md`). **Partly built (2026-10-20):** the range analysis and `kestrel check --report` are built (Round A, `2026-10-19-range-analysis-design.md` with its "As built"), so `no_checks` now has an analysis behind it. `@prove` and `@assume` themselves are not built; they are Round B, on top of that analysis, and the analysis has the hooks they need (`refine` for `@assume`, and the report's data for the four contracts). **The user picked (2026-10-20): an assumption ends at its block's `}`, like a `let`** (option A: Round B adds forgetting at the block's end; less precise than keeping the fact, never unsound, and easy to read). The user took the proposal's pick (`docs/superpowers/plans/2026-10-18-closures-interfaces-proposal.md`, section 4). This spec fixes the syntax and the meaning so closures and interfaces are built in a way contracts can use.

## Why

Kestrel's speed claims rest on what the compiler removes: overflow and bounds checks, allocations, calls. Today a program can't ask whether a loop still has a check in it, or fail the build when a change brings one back. GDScript has annotations (`@export`, `@onready`, `@export_range(0, 100)`) for the editor; Kestrel uses the same look for promises the compiler must prove.

Keywords stay for structure (`pure`, `pub`, `native`); `@` is only for hints and proofs.

## Syntax

An annotation is `@name` or `@name(args)` on the line before what it applies to (several may stack, one per line or space-separated on one line).

```
@prove(no_alloc, no_checks)
pure step(var p: Particle, dt: float) { ... }

@prove(no_checks)
for i from 0 to n { grid[i] += 1 }

@assume(n <= grid.len)

interface Shape {
    @prove(no_alloc)
    pure area(self) -> float
}
```

- `@prove(...)` goes before a function, a method, an interface method (next round), a `for`, `par for` or `while` loop.
- `@assume(cond)` is a statement of its own.
- An unknown name is an error that lists the known ones: "unknown annotation `@fast`; Kestrel has `@prove` and `@assume`". An unknown contract the same way: "unknown contract `fast`; `@prove` takes `no_alloc`, `no_checks`, `no_panic` and `inline`".
- `@` is a new token; nothing else uses it.

## `@prove`: contracts

Each contract is decided by Kestrel's own KIR analyses after inlining and the existing optimisations, never by what Cranelift happened to do, so a contract's result doesn't change with code layout or a Cranelift update.

| Contract | Holds when |
|---|---|
| `no_alloc` | Nothing in it allocates: no arena allocation, no copy of a heap value, no list growth, no stored closure built, no string built. |
| `no_checks` | No overflow, bounds, division or narrowing check is left. |
| `no_panic` | `no_checks`, and no other runtime error can happen (`map` changed in a loop, a native that can fail). |
| `inline` | Every call in it was inlined. |

`no_alloc` counts every allocation, the runtime's own included: `sort` and `sort_by` allocate (their index buffer), so they break it.

- **Through calls:** a contract on a function or loop covers what it calls, inferred through the call graph. A call that can't be followed (a recursive call, a native that isn't marked as safe) breaks `no_alloc`/`no_panic` unless its callee is known to keep them.
- **The error shows the chain:** "`step` can't be proven `no_alloc`: it calls `spawn` (line 40), which adds to `ps` (line 12), which may grow the list".
- **Under `--release`** the arithmetic checks are off (that is all `--release` turns off), but bounds checks stay, so `no_checks` still means "no bounds check left" there; every contract is checked the same way with or without `--release`.

## `@assume`

`@assume(cond)` tells the analysis a comparison it can use: `@assume(n <= grid.len)` lets it drop the bounds checks of `grid[i]` for `i` below `n`.

- `cond` is limited to comparisons the analysis uses: `<`, `<=`, `>`, `>=`, `==`, `!=` between integer expressions and `.len`, joined with `and`.
- **It holds from where it is written until any variable in it changes** (an assignment, a `var` argument, or a change to a list's length, like `grid.pop()`) on any path, loop back edges included, or its block ends (the user's pick, 2026-10-20: it ends at the `}` even when nothing in it changed). So an assumption before a loop whose body pops from `grid` doesn't hold in the loop.
- With checks on, it is checked where it is written (a failure is a runtime error: "assumption `n <= grid.len` is false on line 12 of f.kes").
- **Under `--release`** an assumption that only removes arithmetic checks is trusted and not checked. **An assumption never removes a bounds check unchecked:** if the analysis uses it for a bounds check, the assumption stays one check where it is written, even under `--release` (one check instead of one per iteration). So `@assume` can't make a program read outside a list.
- A warning when it removes nothing: "`@assume(n <= grid.len)` removes no check; delete it".

## `kestrel check --report`

Built first, before contracts fail anything (done 2026-10-20; the format is in `2026-10-19-range-analysis-design.md`, "The report"): it lists every check and allocation left in each function and loop, with its line and why it stayed ("overflow check on line 12: `a * b` may exceed `int`"; "allocation on line 30: `filter` builds a new list"). Contracts are then the same analysis as a build failure.

## Closures, function values and interfaces

- **A literal closure passed to a list method** is specialised and inlined, so its body is analysed as part of the loop: `@prove(no_alloc)` on a loop over `xs.any((e) { e.hp > 0 })` holds, and `xs.filter(...)` breaks it (it builds a list).
- **Building a stored closure with captures allocates** (its environment). A capture-less closure or a named function used as a value doesn't.
- **A call through a function value that isn't known at that point** (a parameter, a field) counts as "may allocate, may panic" for `no_alloc` and `no_panic`, and is not inlined for `inline`. The report names it: "calls the function value `f` (line 8); its body isn't known here". Once devirtualising exists (a follow-up of the closures round), a value that turns out to be a known function after inlining is analysed as that function.
- **Contracts on function types** (`@prove(no_alloc) (int) -> int`) are left for later; the syntax above reserves room for them.
- **Interface methods (next round):** a contract on an interface method binds every implementation, checked at the implementing struct ("`Goblin.area` can't be proven `no_alloc`, which `Shape.area` requires"). So a call through the interface keeps the contract without knowing the concrete type.

## Rulings made on the user's behalf

1. **Contract names:** `no_alloc`, `no_checks`, `no_panic`, `inline`, grouped under `@prove(...)` (the user's pick was `@prove(...)`; the names are from the 2026-10-12 idea). Cost: renaming later breaks programs that use them.
2. **Contracts are decided on KIR after Kestrel's own optimisations,** not on Cranelift's output. Cost: a check Cranelift removes on its own still counts as present.
3. **`@assume` is checked when checks are on, and one used for bounds stays checked under `--release`** (the spec review found that a trusted assumption would let `--release` read outside a list, since `--release` keeps bounds checks). Cost: a run-time check per assumption in debug builds, and one per bounds assumption in release.
4. **Contracts on function types wait.** Cost: a callback can't promise `no_alloc` yet; inline it or use an interface method.
