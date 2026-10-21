# `@` annotations: performance contracts

Status: designed 2026-10-18, alongside closures (`2026-10-18-closures-design.md`). **Partly built (2026-10-20):** the range analysis and `kestrel check --report` are built (Round A, `2026-10-19-range-analysis-design.md` with its "As built"), so `no_checks` now has an analysis behind it. `@prove` and `@assume` themselves are not built; they are Round B, on top of that analysis, and the analysis has the hooks they need (`refine` for `@assume`, and the report's data for the four contracts). **The user picked (2026-10-20): an assumption ends at its block's `}`, like a `let`** (option A: Round B adds forgetting at the block's end; less precise than keeping the fact, never unsound, and easy to read). The user took the proposal's pick (`docs/superpowers/plans/2026-10-18-closures-interfaces-proposal.md`, section 4). This spec fixes the syntax and the meaning so closures and interfaces are built in a way contracts can use. **Round B's design (2026-10-20)** is in the addendum at the end: parsing, where contracts are checked, what each counts, `@assume`'s block and soundness rules, the warning, and interface and function-value calls (two syntax picks there wait for the user).

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

## Round B design addendum (2026-10-20)

What the sections above leave open, decided for Round B (plan: `docs/superpowers/plans/2026-10-20-round-b-annotations.md`). Two points are **syntax picks for the user to confirm** and are marked so; everything else is Claude's ruling on the user's behalf, with its cost at the end.

### Lexing, parsing and what `@` attaches to

- `@` is a new token (`Tok::At`). An annotation is `@`, a name, and `(...)`. The parser knows both names, so the spec's two "unknown" errors and the placement errors below are parse errors.
- **`@prove(a, b)`** takes one or more contract names (plain names, comma-separated). Several `@prove`s before one target merge, and a name listed twice counts once. It may go before: a top-level function (before `pub` and `pure`), a method of a struct or enum, an interface method, and a `for`, `par for` or `while` statement. Nothing else: not a `let`, an `if`, a struct, a closure literal or a native function.
- **`@assume(cond)`** is a statement, anywhere a statement may be: in a function, method or closure body, a loop body, an `if` or `match` arm, an `arena { }` or `in x { }` block. Two may share a line (`@assume(a) @assume(b)`), which is the same as `@assume(a and b)`.
- **Syntax pick for the user to confirm (1): the annotated thing may follow on the same line** (`@prove(inline) pure lerp(a: float, b: float, t: float) -> float { ... }`, `@prove(no_checks) for i from 0 to n {`), as well as on the next line as the spec shows. The parser just skips line ends after an annotation; requiring the next line would be one extra check.
- `@prove` is kept beside the syntax tree, keyed by the span of the function or loop it is on (`Program::proves`), so no existing node changes shape; `@assume` is a statement node, `Stmt::Assume { cond, text, span }`, whose `text` is the condition as written (for messages).
- Errors (parse errors unless noted), plain sentences:
  - "unknown annotation `@fast`; Kestrel has `@prove` and `@assume`" (the spec's)
  - "unknown contract `fast`; `@prove` takes `no_alloc`, `no_checks`, `no_panic` and `inline`" (the spec's)
  - "expected an annotation name after `@`, like `@prove` or `@assume`"
  - "`@prove` needs at least one contract, like `@prove(no_alloc)`"
  - "`@prove` goes before a function, a method or a loop (`for`, `par for` or `while`)"
  - "`@prove` can't go on a native function"
  - "`@assume` needs a condition, like `@assume(i < xs.len)`"
  - "`@assume` goes inside a function, as a statement of its own" (at the top level, or in a struct, enum or interface body)
  - the checker: "`@assume` takes comparisons of integers joined with `and`, like `@assume(i < xs.len and n >= 0)`" (for `or`, `not`, a `bool` value, floats, anything that isn't such a comparison)
  - the checker: "`@assume` can't call a function or index a list (a `--release` build may not run it); compare names, fields, numbers and `.len`"
- **Syntax pick for the user to confirm (2): an assumption's sides are names (locals, parameters, constants), fields of them, integer literals, `.len`, `+`, `-`, `*`, unary `-` and parentheses.** No calls (conversions included) and no `[ ]`. The spec says "integer expressions"; the narrowing is needed because a `--release` build may skip the condition, so it must have no effects and no way to fail other than its own arithmetic. Widening it later breaks nothing.

### `@assume`: what it means and how the analysis uses it

- **Scope.** An assumption covers the rest of its block: lowering puts the statements after it, up to the block's `}`, inside a new KIR statement `Stmt::Assume { id, cond, check, body }`. A block that gives a value (a function's last expression, an `if` or `match` arm used as a value) computes that value into a temporary at the end of the assumption's body, so the value is covered and the function still ends in `return` (the inliner needs that).
- **The analysis runs the assumption's block twice** from the state before it:
  - the **real run** starts from the true state of `cond`, exactly as `if cond` would refine it (Round A's `condition`), with the assumptions inside the block as written. Its decisions are the ones that stand;
  - the **plain run** starts from the state before the assumption, as if no assumption in the block were written. **Everything that leaves the block comes from the plain run:** its end, `break`, `continue` (so the next iteration starts without it) and nothing for `return`. Its decisions go to a scratch table.
  - A check the real run proves and the plain run leaves is one the assumption **removes**.
- So an assumption changes what the compiler knows only between it and its block's `}`; outside, the program is analysed as if it weren't there. This is the user's pick (it ends at the `}` even when nothing in it changed) without the precision loss of forgetting its terms at the end: a fact the code had before the block, like `i < xs.len` in the loop around it, survives the block. Inside, the facts still die when a value in them changes, by Round A's forgetting rules (an assignment, a `var` argument, a length change, on every path, loop back edges included).
- **Checked builds:** the condition is evaluated where the assumption is written, every time. False stops the program with the spec's error: "assumption `n <= grid.len` is false on line 12 of f.kes" (exit 101; no " of f.kes" without a file name). The text is the condition as written.
- **`--release`:** an assumption whose removed checks include a bounds check stays a check (evaluated as in a checked build, with `--release`'s wrapping arithmetic); any other is trusted and not evaluated at all (`AssumeCheck::Trust`, set by the analysis). With no analysis (`--keep-checks`), every assumption stays a check. Each inlined copy is decided on its own.
- **`KESTREL_CHECK_PROOFS=1` evaluates every assumption,** trusted ones included, so a false assumption is reported as the user's, never as "range analysis removed ... this is a compiler bug".
- **A trusted false assumption under `--release`** can remove a division's zero check; the division then faults (the process crashes) instead of a clean error. It can never make a program read outside a list or an array. The syntax spec says so.

### The warning when an assumption removes nothing

- **Computed** from the two runs: each analysed copy of an assumption (its own function and every function it was inlined into) records how many checks it removed. An assumption gets the warning when every copy removed none, in the checked build and in the `--release` build, and at least one copy was analysed (a function the analysis gave up on doesn't count as "removes nothing"). Since the plain run turns off the assumptions inside the block too, an outer assumption is credited with what the inner ones remove: a useless one around a useful one gets no warning (a missed warning, never a wrong one).
- **Shown** by `kestrel check` only (with or without `--release`), as a warning diagnostic at the `@assume`, in the spec's words: "warning: `@assume(n <= grid.len)` removes no check; delete it". `kestrel check` analyses both builds only when the program has an assumption (the second on a copy of the module). `kestrel run` and `kestrel build` don't repeat it.
- **It never fails anything:** exit codes, the error-test harness and contracts ignore warnings.

### Contracts: when, where and against which build

- **Decided on the final KIR,** after `optimize`, by a new `kestrel_kir::range::contracts::check`, from range analysis run in the build's own mode and the report's data (the spec's ruling 2).
- **The driver runs it** (`kestrel_driver::prove`, after `compile`) when the program has a `@prove` (and, for `kestrel check`, when it has an `@assume`, for the warning). It runs range analysis once, **in place**, and records the mode in `Module::analysed`; codegen then skips its own analysis of a copy (and refuses a module analysed for the other build). A program without annotations goes through exactly the path it takes today.

| Command | Contracts checked against | Assumption warnings |
|---|---|---|
| `kestrel check f.kes` | the checked build | yes |
| `kestrel check --release f.kes` | the `--release` build | yes |
| `kestrel run`, `kestrel build` (with or without `--release`) | that build | no |
| `kestrel check --report` | as `check`; **the report still prints when only contracts fail** (exit 1): it is how to see why | yes |
| the Godot add-on | its checks setting | no |

- A failed contract is an error: exit 1, nothing runs or is built. `bench/kernels` and other harnesses that call codegen directly don't check contracts.

### What a contract covers

- **A contract is about the code as written, in its own function, for any arguments.** For a function or method: its own compiled copy, analysed with its parameters at their types' ranges, whether or not anything calls it. For a loop: the code of that loop in its own function's copy. Where the function is inlined into a caller, that copy belongs to the caller: the caller's contracts and report cover it. So adding a caller can never break a contract, and a contract's result doesn't depend on where code was inlined. (A helper whose checks only go away with its callers' constant arguments needs an `@assume` to carry `no_checks` on its own.)
- **A loop's contract covers the whole loop statement:** what its header evaluates once (the range ends of `for i from a to b`, the list of `for x in xs.filter(...)`, a `par for`'s threads) and every iteration, a `while`'s condition included. Lowering wraps every KIR statement the loop lowers to in a no-op block `Stmt::Prove { id, body }`; the analysis and the report's walk tag what they find inside with `id`, so two loops on one line are never confused.
- **What a region holds:** its own checks left and events (allocations, ways to fail, calls left); a `par for` body it runs, whole; and, through each call left, what the callee summarises.
- **Through calls:** a call the inliner left is followed through the call graph, cycles included (a recursive function that allocates nothing keeps `no_alloc`), with the report's summaries: `allocates` and `fails` (Round A) and two new chains, `checks` (the first check left in the callee or below) and `unknown` (below). A closure literal made in the region counts as called (Round A's `What::Closure` edge).

| Contract | Fails on |
|---|---|
| `no_checks` | an overflow, shift, division, narrowing or bounds check left, in the region or below a call; an unknown call (below). An assumption's own test, and the runtime's internal checks (`remove_at`, `s[i]`, the map-changed check), are not "checks" here; they are ways to fail. |
| `no_alloc` | an allocation the report lists (the runtime's own included: `sort`, `sort_by`, a `par for`'s per-thread arenas, growth, copies, stored closures, boxed interface values, `arena { }`, `in x { }`), in the region or below a call; a native; an unknown call |
| `no_panic` | everything `no_checks` fails on, and every way to fail the report lists (a native, `remove_at`, `split`, the item limit, a map replaced in its loop, `Trap`, `Fail`), and an assumption that is evaluated in this build (always in a checked build). `no_panic` includes `no_checks`. |
| `inline` | a call to a function or closure of the program that the inliner left, with its recorded reason; a call through a function value or an interface (never inlined, even when the interface method promises `inline`). Natives, the runtime and the functions the compiler makes for built-ins (`filter`'s loop, printing, copies) aren't counted: `no_alloc` and `no_panic` cover what they do. A `par for` isn't a call; its body is in the region, so its calls count. |

**Round A's four report gaps:**
1. **Gave up or cut means failed.** A function the analysis gave up on (the budget or the nesting limit), or whose events walk stopped at its depth limit (`events.rs`, `MAX_DEPTH = 4096`, which now records that it stopped), fails every contract on it or on a loop in it, and is an `unknown` reason for every caller that reaches it: "range analysis stopped early in it (its code nests too deeply), so nothing can be proven about it; split it into smaller functions".
2. **No totals.** Contracts never use the report's totals or counts (they count an inlined body twice): a contract looks only for a first item, in the region or in a chain, and its error names that one item and no number.
3. **Running out of memory is not a way to fail** for `no_panic`: every allocation could, so counting it would make `no_panic` mean `no_alloc` as well. `no_alloc` is the contract for that; the syntax spec says so.
4. **The constants are charged to `main`.** A contract on `main` also follows `m.init` (the constants are built before it starts): "it builds the program's constants first, which ...". No other function is charged with them.

### Calls through interfaces and function values

- **A call through a function value** (a parameter, a field, a stored closure) breaks all four contracts: its body isn't known where the call is, so nothing can be proven about it (the spec's rule for `no_alloc`, `no_panic` and `inline`, and the same reason for `no_checks`). Contracts on function types wait (the spec's ruling 4). Help: "contracts can't see into a function value yet; call a named function or an interface method instead".
- **A call through an interface** always breaks `inline`. For the other three, it keeps exactly the contracts its interface method declares: `@prove(no_alloc)` on `Shape.area` makes `s.area()` count as not allocating. Lowering puts the promised set on the call (`CallVirtual { ..., keeps: ContractSet }`, `no_panic` implying `no_checks`), and the report drops that call's "may allocate" or "may fail" item accordingly. A call to an interface method without the promise is an unknown call, and the error's help says what to do: "add `@prove(no_alloc)` to the interface method `Shape.area`; then every implementation must keep it".
- **Why this is sound:** a value can only become a `Shape` from a type that implements `Shape`, and lowering turns the interface method's contract into a contract on every implementing method, checked at its definition like any function contract (for any arguments), with the spec's wording: "`Goblin.area` can't be proven `no_alloc`, which `Shape.area` requires: ...", at the implementing method's name. So every body the call can reach keeps the promise.

### Error messages

One error per contract that fails, at that contract's name inside `@prove(...)` (an interface's contract at the implementing method's name), naming the first item in the order the report lists them:

- "`step` can't be proven `no_checks`: it has a bounds check on line 14: `xs[i + 1]` may be outside `xs` (`i` is 0 to 98; nothing shows it is below `xs.len`)"
- "the loop at line 12 can't be proven `no_alloc`: it allocates on line 13: `add` may grow `ys`"
- "`f` can't be proven `no_panic`: it can fail on line 9: the assumption `n <= xs.len` is checked here"
- through calls, the report's chain after "it ": "`step` can't be proven `no_alloc`: it calls `spawn` (line 40), which adds to `ps` (line 12) and may grow the list" (the spec's); for a check: "... it calls `g` (line 5), which has a bounds check (line 9): `xs[i]` may be outside `xs` (`i` can be any `int`)"
- `inline`: "`f` can't be proven `inline`: it calls `big` (line 7), which isn't inlined: it has an early `return`"; "... it calls the function value `f` (line 8), which can't be inlined"
- unknown: "... it calls the function value `f` (line 8); its body isn't known here" and "... it calls a method of the interface value `s` (line 8); its body isn't known here" (the report's words)
- A line in another file is written `geom.kes:41`, as in the report. The generic help is "`kestrel check --report` lists everything left in it".

### Soundness rules an assumption must not break

1. **It teaches only what `if cond` teaches on its true branch** (Round A's `condition`, with the build's arithmetic: under `--release` a side that could wrap has no linear form), never more. Anything the analysis can't represent is dropped, never guessed.
2. **Its condition has no effects and can't fail except by its own arithmetic** (the syntax rule above), so evaluating it or skipping it can't change the program.
3. **It dominates what it covers:** lowering puts only the statements after it, in its block, in its body; the condition runs before any of them.
4. **Nothing outside its block sees it:** what leaves the block comes from the plain run (rule above), loop back edges and jumps out included.
5. **A false assumption the analysis can see proves nothing:** if the condition contradicts what is known, the real run finds the block unreachable, and unreachable code keeps every check (Round A's rule). `@assume(1 > 2)` removes nothing, and gets the warning.
6. **Under `--release`, an assumption is trusted only when its plain run proves every bounds check its real run proves.** The plain run turns off the assumptions inside the block too, so two assumptions that each suffice can't vouch for each other: the outer one then removes the check and stays evaluated, and the inner one can be trusted because a checked one already covers it. (By induction from the outermost: a bounds check is removed only when it holds without every trusted assumption around it.)
7. **The facts die with their values,** by Round A's forgetting rules, inside the block too.
8. **A function the analysis gives up on keeps every assumption evaluated** (the default) and every check.
9. **Every analysed copy is decided on its own,** and the proof check (`KESTREL_CHECK_PROOFS=1`) evaluates all of them.

### What this costs at compile time

Round A's compile-time miss (+30-44% on the voxel programs, follow-ups file) must not grow:
- **A program with no annotation pays nothing new:** no driver step runs, and codegen analyses a copy exactly as now. The frozen voxel benchmark has none, so its build time must stay within noise of Round A's (181 ms checked, 160 ms `--release`, `kestrel/bench/voxels3d/results.md`).
- **With annotations,** range analysis still runs once per build (with the texts, which Round A measured as free), in the driver instead of codegen; the report's walk and summaries add a pass linear in the program's size; each assumption analyses its block twice (nested ones add one run per level). `kestrel check` analyses a second build only for the warning. The measure task records the voxel world with a scratch copy that has a few annotations.

### Rulings made on the user's behalf (Round B)

1. **An assumption's block is analysed twice, and what leaves it comes from the run without it** — the user's block-end pick, with no fact from outside the block lost — cost if wrong: a block with an assumption costs twice its analysis (nested: one more run per level).
2. **A contract is about the function's (or loop's) own copy, for any arguments; inlined copies belong to their callers** — results never depend on callers — cost if wrong: a helper whose checks only its callers' constants remove can't carry `no_checks` without an `@assume`.
3. **A loop's contract covers its header too, so `@prove(no_alloc)` on a `par for` always fails** (its per-thread arenas, which the report lists) — "what is on that line and in the braces" is the simplest reading, and `for x in xs.filter(...)` must not pass `no_alloc` — cost if wrong: `no_alloc` on a `par for` is useless; put it on the function the body calls.
4. **An assumption that is evaluated counts as a way to fail for `no_panic`** (always in a checked build) — the spec says a false one is a run-time error — cost if wrong: `@prove(no_panic)` and an `@assume` in the same function can't both hold in a checked build; put the assumption before the loop the contract is on.
5. **A function value breaks all four contracts; an interface call keeps what its method declares, and every implementation is checked** — the spec's rules, made exact — cost if wrong: callbacks can't be in contracted code until function types can carry contracts.
6. **Running out of memory doesn't count for `no_panic`** — cost if wrong: `no_panic` alone doesn't promise the code never stops; `no_alloc` with it does.
7. **The warning comes only from `kestrel check`, over both builds** — an assumption written for overflow checks removes nothing under `--release` by design — cost if wrong: `kestrel run` users don't see it; `kestrel check` analyses twice when the program has an assumption.
8. **Contracts are checked in the driver, which analyses the module in place; codegen reuses that and refuses one analysed for the other build** — one analysis per build — cost if wrong: the module carries its mode (`Module::analysed`), and a host that skips the driver step skips contracts (the kernels harness does, on purpose).
9. **The report still prints when only contracts fail** — it is how to see why — cost if wrong: `kestrel check --report` exits 1 with output on standard output.
10. **One error per failed contract, naming its first item and no count** — totals double-count inlined code — cost if wrong: fixing one item may reveal the next on the following build; the report shows them all at once.
