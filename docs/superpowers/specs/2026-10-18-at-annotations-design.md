# `@` annotations: performance contracts

Status: designed 2026-10-18, alongside closures (`2026-10-18-closures-design.md`). **Built (2026-10-21):** the range analysis and `kestrel check --report` came first (Round A, `2026-10-19-range-analysis-design.md` with its "As built"), and `@prove` and `@assume` are Round B, built on top of it (commits 169d03b..5be3f31, then the measuring and docs commit). The last section, "As built (Round B)", lists what differs from the addendum, the holes reviews found, the rulings made while building, and what was measured and is still owed. **The user picked (2026-10-20): an assumption ends at its block's `}`, like a `let`** (option A: Round B adds forgetting at the block's end; less precise than keeping the fact, never unsound, and easy to read). The user took the proposal's pick (`docs/superpowers/plans/2026-10-18-closures-interfaces-proposal.md`, section 4). This spec fixes the syntax and the meaning so closures and interfaces are built in a way contracts can use. **Round B's design (2026-10-20, revised after the plan review)** is in the addendum at the end: parsing, where contracts are checked, what each counts, `@assume`'s block and soundness rules, the warning, and interface and function-value calls (two syntax picks there wait for the user).

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

- `@prove(...)` goes before a function, a method, an interface method, a `for`, `par for` or `while` loop.
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
- **Interface methods:** a contract on an interface method binds every implementation, checked at the implementing struct ("`Goblin.area` can't be proven `no_alloc`, which `Shape.area` requires"). So a call through the interface keeps the contract without knowing the concrete type.

## Rulings made on the user's behalf

1. **Contract names:** `no_alloc`, `no_checks`, `no_panic`, `inline`, grouped under `@prove(...)` (the user's pick was `@prove(...)`; the names are from the 2026-10-12 idea). Cost: renaming later breaks programs that use them.
2. **Contracts are decided on KIR after Kestrel's own optimisations,** not on Cranelift's output. Cost: a check Cranelift removes on its own still counts as present.
3. **`@assume` is checked when checks are on, and one used for bounds stays checked under `--release`** (the spec review found that a trusted assumption would let `--release` read outside a list, since `--release` keeps bounds checks). Cost: a run-time check per assumption in debug builds, and one per bounds assumption in release.
4. **Contracts on function types wait.** Cost: a callback can't promise `no_alloc` yet; inline it or use an interface method.

## Round B design addendum (2026-10-20)

What the sections above leave open, decided for Round B (plan: `docs/superpowers/plans/2026-10-20-round-b-annotations.md`). Revised the same day after an Opus review of the plan (3 Critical, 6 Important, 22 Minor, all folded in) and the controller's rulings. Two points are **syntax picks for the user to confirm**. Rulings that **differ from the text above** are marked "(differs from the spec)"; the rest only fill gaps. Each ruling's cost is at the end.

### Lexing, parsing and what `@` attaches to

- `@` is a new token (`Tok::At`). An annotation is `@` directly followed by a name (no space: `@ prove` is an error), then `(...)`. The parser knows both names, so the spec's two "unknown" errors and the placement errors below are parse errors.
- **`@prove(a, b)`** takes one or more contract names (plain names, comma-separated). Several `@prove`s before one target merge, and a name listed twice counts once. It may go before: a top-level function (before `pub` and `pure`), a method of a struct or enum, an interface method, and a `for`, `par for` or `while` statement. Nothing else: not a `let`, an `if`, a `match`, a struct, a closure literal, an `@assume` or a native function. To put an assumption before an annotated loop, write the `@assume` first, then the `@prove`.
- **`@assume(cond)`** is a statement, anywhere a statement may be: a function, method or closure body, a loop body, an `if` or `match` arm, an `arena { }` or `in x { }` block. Assumptions with no statement between them (on one line, `@assume(a) @assume(b)`, or on consecutive lines) are one assumption with several conditions, the same as `@assume(a and b)`; each condition keeps its own text for its messages.
- **Syntax pick for the user to confirm (1): the annotated thing may follow on the same line, or on the next line as the spec shows** (`@prove(inline) pure lerp(a: float, b: float, t: float) -> float { ... }`, `@prove(no_checks) for i from 0 to n {`). At most one line break: a blank line between an annotation and its target is an error, so an annotation never attaches silently to something further down.
- `@prove` is kept beside the syntax tree, keyed by the span of the function or loop it is on (`Program::proves`), so no existing node changes shape; `@assume` is a statement node, `Stmt::Assume { cond, text, span }`, whose `text` is the condition as written.
- Errors (parse errors unless noted), plain sentences:
  - "unknown annotation `@fast`; Kestrel has `@prove` and `@assume`" (the spec's)
  - "unknown contract `fast`; `@prove` takes `no_alloc`, `no_checks`, `no_panic` and `inline`" (the spec's)
  - "expected an annotation name after `@`, like `@prove` or `@assume`"
  - "`@prove` needs at least one contract, like `@prove(no_alloc)`"
  - "`@prove` goes before a function, a method or a loop (`for`, `par for` or `while`)"
  - "`@prove` goes on the line right before what it applies to, or on the same line" (a blank line between)
  - "`@prove` can't go on a native function" (only built-in modules have natives, so only they can reach it)
  - "`@assume` needs a condition, like `@assume(i < xs.len)`"
  - "`@assume` goes inside a function, as a statement of its own" (at the top level, or in a struct, enum or interface body)
  - the checker: "`@assume` takes comparisons of integers joined with `and`, like `@assume(i < xs.len and n >= 0)`" (for `or`, `not`, a `bool` value, floats, anything that isn't such a comparison)
  - the checker: "`@assume` can't call a function or index a list (a `--release` build may not run it); compare names, fields, numbers and `.len`"
- **Syntax pick for the user to confirm (2): an assumption's sides are names (locals, parameters, constants), fields of them, integer literals, `.len`, `+`, `-`, `*`, unary `-` and parentheses** (differs from the spec, which says "integer expressions"). No calls (conversions included) and no `[ ]`; integer widening is implicit, so `i64` against `xs.len` needs no conversion. The narrowing is what makes skipping the condition under `--release` safe: it has no effects and can fail only by its own arithmetic. A named constant is written out at each use and its initializer may hold a division, a shift or `abs`; that is harmless (constants have no effects, and a trusted assumption isn't evaluated at all). The cost: `grid[y].len` and `parts[i].items.len` (Round A terms) can't be assumed; allowing an index that is a constant or a local, evaluated only when the assumption is checked, would recover them later.

### `@assume`: what it means and how the analysis uses it

- **Scope.** An assumption covers the rest of its block: lowering puts the statements after it, up to the block's `}`, inside a new KIR statement `Stmt::Assume { conds, check, body }` (`conds` holds each condition with its `id`). A block that gives a value computes the value inside the assumption's body, into a temporary (`Move` for a value with heap memory): an `if` or `match` arm used as a value the way `block_value` does on its cleanup path (`owned(v, false)`), a function's last expression with `owned(v, true)` (it copies a parameter's list and moves a local one, so adding an assumption adds no allocation). A returned parameter list is still copied. A function body whose rest ends in `return e` gets the same: `e` goes into the temporary inside the body and the `return` comes after it. Either way the function still ends in `return`, so the inliner still takes it.
- **The analysis runs the assumption's block twice** from the state before it, in a deciding pass:
  - the **real run** starts from the true state of the conditions, exactly as `if cond` would refine it (Round A's `condition`), with the assumptions inside the block as written. Its decisions are the ones that stand;
  - the **plain run** starts from the state before the assumption, as if no assumption in the block were written. **Everything that leaves the block comes from the plain run:** its end, `break`, `continue` (so the next iteration starts without it) and nothing for `return`. Its decisions go to a scratch table.
  - In a pass that decides nothing (an enclosing loop's fixed-point rounds) only the plain run happens: the real run's decisions would go nowhere and its exits are never used.
  - A check the real run proves and the plain run leaves is one the assumption **removes**. Each use is recorded per KIR statement (by its address, as checks are), only in deciding passes outside unreachable code; several visits of one statement fold together.
- So an assumption changes what the compiler knows only between it and its block's `}`; outside, the program is analysed as if it weren't there. This is the user's pick (it ends at the `}` even when nothing in it changed) without the precision loss of forgetting its terms at the end: a fact the code had before the block, like `i < xs.len` in the loop around it, survives the block. Inside, facts still die when a value in them changes, by Round A's forgetting rules.
- **Checked builds:** each condition is evaluated where the assumption is written, every time, in order. A false one stops the program with the spec's error: "assumption `n <= grid.len` is false on line 12 of f.kes" (exit 101; no " of f.kes" without a file name).
- **`--release`:** an assumption statement whose removed checks include a bounds check stays a check (evaluated with `--release`'s wrapping arithmetic); any other is trusted and not evaluated (`AssumeCheck::Trust`, set by the analysis on that statement). Each copy (inlining can put two copies of one assumption in one function) is decided on its own. With no analysis (`--keep-checks`), every assumption stays a check; under a checked build the analysis sets every one to `Check`.
- **`KESTREL_CHECK_PROOFS=1` evaluates every assumption,** trusted ones included, so a false assumption is reported as the user's, never as "range analysis removed ... this is a compiler bug".
- **A trusted false assumption under `--release`** can remove a division's zero check; the division then faults (the process crashes) instead of a clean error. It can never make a program read outside a list or an array. The syntax spec says so, next to what `no_panic` doesn't count.

### The warning when an assumption removes nothing

- **Computed** from the two runs: each analysed copy of an assumption statement records how many checks it removed. An assumption gets the warning when every copy removed none, in the checked build and in the `--release` build, and at least one copy was analysed (a function the analysis gave up on doesn't count as "removes nothing"; a contradiction, whose block the real run finds unreachable, counts as a use that removed nothing, so Rule 5's warning holds). A merged assumption's use carries every condition's id, so each of its `@assume`s gets the warning. Since the plain run turns off the assumptions inside the block too, an outer assumption is credited with what inner ones remove, and the conditions of one merged assumption are credited together: a useless one there gets no warning (a missed warning, never a wrong one).
- **Shown** by `kestrel check` only (with or without `--release`), as a warning at the `@assume`, in the spec's words: "warning: `@assume(n <= grid.len)` removes no check; delete it". `kestrel check` analyses both builds only when the program has an assumption (the second on a copy of the module). `kestrel run` and `kestrel build` don't repeat it.
- **It never fails anything:** exit codes, the error-test harness and contracts ignore warnings.

### Contracts: when, where and against which build

- **Decided on the final KIR,** after `optimize`, by a new `kestrel_kir::range::contracts::check`, from range analysis run in the build's own mode and the report's data (the spec's ruling 2).
- **The driver runs it** (`kestrel_driver::prove`, after `compile`) when the program has a `@prove` (and, for `kestrel check`, when it has an `@assume`, for the warning). It runs range analysis once, **in place**, and records the mode in `Module::analysed`; codegen then skips its own analysis of a copy, refuses a module analysed for the other build, and refuses an analysed module when asked to keep every check (`remove_checks: false`). A program without annotations goes through exactly the path it takes today.

| Command | Contracts checked against | Assumption warnings |
|---|---|---|
| `kestrel check f.kes` | the checked build | yes |
| `kestrel check --release f.kes` | the `--release` build | yes |
| `kestrel run`, `kestrel build` (with or without `--release`) | that build | no |
| `kestrel check --report` | as `check`; **the report still prints when only contracts fail** (exit 1): it is how to see why | yes |
| the Godot add-on | its checks setting | no |

- A failed contract is an error: exit 1, nothing runs or is built. `bench/kernels` and other harnesses that call codegen directly don't check contracts.

### What a contract covers

- **A contract covers its target's own compiled copy and every copy inlining made of it.** The compiled code is what the contract promises about, and copies really differ: a caller's inlining budget or depth limit, its 64 terms, or its giving up can leave a check, a call or an assumption in its copy that the own copy doesn't have. A failure in an inlined copy names the caller.
  - For a function or method: its own function (analysed with its parameters at their types' ranges, whether or not anything calls it), and every place the inliner expanded it: the inliner wraps the expanded body of a callee that has a contract in `Stmt::Prove { id, at }` (`id` the contract's site, `at` the packed line of the call), with the returned value computed into a temporary inside it; the arguments, the caller's code, stay outside; a callee with two sites (its own and an interface's) gets one block per site.
  - For a loop: every copy of its `Stmt::Prove { id }` block, wherever lowering, inlining or specialisation put it. Lowering wraps every KIR statement the loop lowers to in that block. Where lowering moves statements into a new function (a `par for` body, a closure's code), the site is re-pointed to it. If no function holds the block, the contract fails ("`@prove` lost track of the loop at line 12; this is a compiler bug"), never passes.
- **A loop's contract covers the whole loop statement:** what its header evaluates once (the range ends of `for i from a to b`, the list of `for x in xs.filter(...)`, a `par for`'s per-thread arenas) and every iteration, a `while`'s condition included. So `@prove(no_alloc)` on a `par for` always fails, with the help "a `par for` makes an arena for each thread; put `@prove(no_alloc)` on the function its body calls".
- **What a region holds:** its own checks left and events (allocations, ways to fail, calls left); a `par for` body it runs, whole; a closure made in it (its code may run); and, through each call left, what the callee summarises.
- **Through calls:** a call the inliner left is followed through the call graph with the report's summaries, extended per contract: `allocates`, `fails` (Round A), `checks` (the first check left in the callee or below) and `unknown` (an unknown call, a give-up or a cut below, decided per contract: see the next section). Cycles are followed to a fixed point for `no_alloc` and `no_checks`. **For `no_panic`, any recursion below or in the region fails it** (a direct call to itself or a cycle through others; the stack may run out), as the spec's "Through calls" says.

| Contract | Fails on |
|---|---|
| `no_checks` | an overflow, shift, division, narrowing or bounds check left, in the region or below a call; an unknown call for `no_checks`, in the region or below a call. An assumption's own test and the runtime's internal checks (`remove_at`, `s[i]`, the map-changed check) are ways to fail, not "checks" here. |
| `no_alloc` | an allocation the report lists (the runtime's own included: `sort`, `sort_by`, a `par for`'s per-thread arenas, growth, copies, stored closures, boxed interface values, `arena { }`, `in x { }`), in the region or below a call; an unknown call for `no_alloc` |
| `no_panic` | everything `no_checks` fails on; every way to fail the report lists except running out of resources (below): a native, `remove_at`, `split`, `array[T](n)` with a bad `n`, a map replaced in its loop, `Trap`, `Fail`; an assumption evaluated in this build (always in a checked build); recursion; an unknown call for `no_panic`. `no_panic` includes `no_checks`. |
| `inline` | a call to a function or closure of the program that the inliner left, with its recorded reason, in the region, in a closure's code made there, and from a compiler-made function there into program code (a `sort_by` comparison calling the key's closure); a call through a function value or an interface (never inlined, even when the interface method promises `inline`). Natives, the runtime and the compiler-made functions themselves (`filter`'s loop, printing, copies) aren't counted: `no_alloc` and `no_panic` cover what they do. A `par for` isn't a call; its body is in the region, so its calls count. |

- **Running out of resources is not a way to fail for `no_panic`** (differs from the spec, which counts every run-time error): running out of memory, and a list or map growing past 2,147,483,647 items (`add`, `insert`, a map key set, the built-ins that build a list by growing it). Only that part: `insert` with an index outside the list and a map key set that adds a key while looping over the map are logic failures and still count. Both are capacity limits, and counting them would make `no_panic` fail on any code that grows a list; `no_alloc` is the contract for that. A creation sized by a value (`array[T](n)` with a negative or too large `n`) is a logic failure and still counts. The report keeps listing all of them. The syntax spec says what `no_panic` doesn't count: running out of memory, the item limit on growth, and (under `--release`) a false trusted assumption's divide fault.
- **Round A's four report gaps:**
  1. **Gave up, cut or undecided means failed.** A function the analysis gave up on, whose events walk stopped at its depth limit (`MAX_DEPTH = 4096`, now recorded as an event), or with a check site the analysis never decided, fails every contract that covers code in it, and is an unknown reason for every caller that reaches it: "range analysis stopped early in it (its code nests too deeply), so nothing can be proven about it; split it into smaller functions".
  2. **No totals.** Contracts never use the report's totals or counts: a contract looks for a first item, in its regions or in a chain, and its error names that one item and no number.
  3. **Running out of memory** doesn't count for `no_panic` (above).
  4. **The constants are charged to `main`.** A contract on `main` also follows `m.init` (the constants are built before it starts): "it builds the program's constants first, which ...". No other function is charged with them.

### Calls through interfaces, function values and natives

- **Unknown calls are events of their own** (`What::Unknown`: a function value, an interface call with what its method promises, a native), in the region or below a call, and each contract decides them: a function value is unknown for `no_checks`, `no_alloc` and `no_panic`; a native for `no_alloc` and `no_panic`; an interface call for each of the three its method doesn't promise. The report renders them with Round A's words.
- **A call through a function value** (a parameter, a field, a stored closure) breaks all four contracts: its body isn't known where the call is (the spec's rule for `no_alloc`, `no_panic` and `inline`; for `no_checks` it is an extension of the spec). Contracts on function types wait (the spec's ruling 4). Help: "contracts can't see into a function value yet; call a named function or an interface method instead".
- **A call through an interface** always breaks `inline`. For the other three, it keeps exactly the contracts its interface method declares: `@prove(no_alloc)` on `Shape.area` makes `s.area()` count as not allocating. Lowering puts the promised set on the call (`CallVirtual { ..., keeps: ContractSet }`, `no_panic` bringing `no_checks`), and the report drops that call's "may allocate" or "may fail" item accordingly. An unpromised one gets the help "add `@prove(no_alloc)` to the interface method `Shape.area`; then every implementation must keep it".
- **Why this is sound:** a value can only become a `Shape` from a type that implements `Shape`, and lowering turns the interface method's contract into a contract on every implementing method (from the checker's list of implementations, not from the method tables lowering makes on demand), checked like any function contract, with the spec's wording: "`Goblin.area` can't be proven `no_alloc`, which `Shape.area` requires: ...", at the implementing method's name. When the method also has its own `@prove` with the same contract, only the interface's error is given.

### Error messages

One error per failed contract, at that contract's name inside `@prove(...)` (an interface's at the implementing method's name), naming the first item in the order the report lists them; the item's own line is in the message:

- "`step` can't be proven `no_checks`: it has a bounds check on line 14: `xs[i + 1]` may be outside `xs` (`i` is 0 to 98; nothing shows it is below `xs.len`)"
- "the loop at line 12 can't be proven `no_alloc`: it allocates on line 13: `add` may grow `ys`"
- "`f` can't be proven `no_panic`: it can fail on line 9: the assumption `n <= xs.len` is checked here"
- through calls, the report's chain after "it ": "`step` can't be proven `no_alloc`: it calls `spawn` (line 40), which adds to `ps` (line 12) and may grow the list" (the spec's); for a check: "... it calls `g` (line 5), which has a bounds check (line 9): `xs[i]` may be outside `xs` (`i` can be any `int`)"
- recursion: "`f` can't be proven `no_panic`: it calls `walk` (line 5), which can call itself (line 9), so the stack may run out"; its own: "... it can call itself (line 9), so the stack may run out"
- in an inlined copy: "`get` can't be proven `no_checks` where it is inlined into `step` (line 40): it has a bounds check on line 3: ..."; a loop: "the loop at line 12 can't be proven `no_checks` where it is inlined into `step`: ..."
- `inline`: "`f` can't be proven `inline`: it calls `big` (line 7), which isn't inlined: it has an early `return`"; "... it calls the function value `f` (line 8), which can't be inlined"
- unknown: "... it calls the function value `f` (line 8); its body isn't known here", "... it calls a method of the interface value `s` (line 8); its body isn't known here", "... it calls the native function `draw.clear` (line 4); its body isn't known here" (the report's words)
- A line in another file is written `geom.kes:41`, as in the report. The generic help is "`kestrel check --report` lists everything left in it".

### Soundness rules an assumption must not break

1. **It teaches only what `if cond` teaches on its true branch** (Round A's `condition`, with the build's arithmetic: under `--release` a side that could wrap has no linear form), never more. Anything the analysis can't represent is dropped, never guessed.
2. **Its conditions have no effects and can't fail except by their own arithmetic** (syntax pick 2; constants included, see there), so evaluating them or skipping them can't change the program.
3. **It dominates what it covers:** lowering puts only the statements after it, in its block, in its body; the conditions run before any of them.
4. **Nothing outside its block sees it:** what leaves the block comes from the plain run, loop back edges and jumps out included.
5. **A false assumption the analysis can see proves nothing:** if the conditions contradict what is known, the real run finds the block unreachable, and unreachable code keeps every check (Round A's rule). `@assume(1 > 2)` removes nothing, and gets the warning.
6. **Under `--release`, an assumption statement is trusted only when its plain run proves every bounds check its real run proves.** The plain run turns off the assumptions inside the block too, so two assumptions that each suffice can't vouch for each other: the outer one stays evaluated, and the inner one can be trusted because a checked one already covers it. (By induction from the outermost: a bounds check is removed only when it holds without every trusted assumption around it.) Trust is decided per statement, never per source assumption, since two copies in one function can differ.
7. **What the plain run learns from a check the real run removed still holds.** In a checked build the assumption is evaluated. Under `--release` a trusted assumption removes only division zero checks, and codegen's plain divide (Cranelift `sdiv`, `srem`, `udiv`, `urem`) traps on zero, so "the divisor isn't 0" after the division still holds. This depends on `codegen/src/func/divide.rs` never emitting a non-trapping divide for a `Proven` division; `range/assume.rs` says so.
8. **The facts die with their values,** by Round A's forgetting rules, inside the block too.
9. **A function the analysis gives up on keeps every assumption evaluated** (the default) and every check.
10. **The proof check (`KESTREL_CHECK_PROOFS=1`) evaluates every assumption.**

### What this costs at compile time

Round A's compile-time miss (+30-44% on the voxel programs, follow-ups file) must not grow:
- **A program with no annotations shows no measurable change:** under 2% on `kestrel build` of the voxel world (Round A: 181 ms checked, 160 ms `--release`, `kestrel/bench/voxels3d/results.md`). No driver step runs, and codegen analyses a copy exactly as now.
- **With annotations,** range analysis still runs once per build (with the texts, which Round A measured as free), in the driver instead of codegen; the report's walk and summaries add a pass linear in the program's size. An assumption adds one extra run of its block per deciding pass (none in the fixed-point rounds) and one per level of nested assumptions; the frames are moved aside, not cloned. **The extra runs have their own budget:** a function's statement budget is its base, `max(200_000, 100 * statements)`, scaled by `(statements + extra) / statements`, where `extra` counts each statement once more for every assumption around it (amended to what was built: the base alone, with the extra statements counted against it, left a function under 2,000 statements at the 200,000 floor with nothing extra), so adding an assumption can't push a function past its budget. A function without an assumption has the base, as before. Targets: the voxel world with a few annotations builds within 10% of the unannotated one, and `kestrel check` of a program with an assumption takes at most 2.2 times as long as without; a miss goes to the follow-ups.

### Rulings made on the user's behalf (Round B)

1. **An assumption's block is analysed twice in a deciding pass, and what leaves it comes from the run without it** — the user's block-end pick, with no fact from outside the block lost — cost if wrong: a block with an assumption costs one more run per deciding pass (and per nested level).
2. **A contract covers the target's own copy and every inlined copy, and a failure in a copy names the caller** — the compiled code is what it promises about, and copies differ — cost if wrong: spurious failures that blame callers (a helper whose own copy keeps a check its callers' constants remove fails too); relaxing later is easy.
3. **A loop's contract covers its header too, so `@prove(no_alloc)` on a `par for` always fails** (its per-thread arenas, which the runtime really makes) — `for x in xs.filter(...)` must not pass `no_alloc` — cost if wrong: put `no_alloc` on the function the body calls; the help says so.
4. **An assumption that is evaluated counts as a way to fail for `no_panic`** (always in a checked build), as the spec makes a false one a run-time error — cost if wrong: a function-level `@prove(no_panic)` and an `@assume` in that function can't both hold in a checked build (moving the assumption to the caller doesn't help: facts don't cross calls); put the contract on a loop and the assumption before it.
5. **Recursion fails `no_panic`** (the spec's rule; the first draft of this addendum had let cycles pass) — cost if wrong: recursive code can't carry `no_panic`; loosen later.
6. **A function value breaks all four contracts (for `no_checks` an extension of the spec); an interface call keeps what its method declares, and every implementation is checked** — cost if wrong: callbacks can't be in contracted code until function types carry contracts.
7. **Running out of memory and the item limit on growth don't count for `no_panic`** (differs from the spec) — cost if wrong: a `no_panic` function can still stop on huge growth; `no_alloc` with it rules that out.
8. **The warning comes only from `kestrel check`, over both builds** — cost if wrong: `kestrel run` users don't see it; `kestrel check` analyses twice when the program has an assumption.
9. **Contracts are checked in the driver, which analyses the module in place; codegen reuses that and refuses one analysed for the other build or with every check kept** — one analysis per build — cost if wrong: the module carries its mode (`Module::analysed`), and a host that skips the driver step skips contracts (the kernels harness does, on purpose).
10. **The report still prints when only contracts fail** — cost if wrong: `kestrel check --report` exits 1 with output on standard output.
11. **One error per failed contract, naming its first item and no count** — totals double-count inlined code — cost if wrong: fixing one item may reveal the next; the report shows them all.
12. **Assumptions with no statement between them are one assumption** (the spec's "`@assume(a) @assume(b)` means `@assume(a and b)`") — each would otherwise nest the rest of the block one level deeper — cost if wrong: they are trusted, kept and credited together.

## As built (Round B, 2026-10-21)

Built on `main` as plan `docs/superpowers/plans/2026-10-20-round-b-annotations.md` (revised after an Opus plan review and a scoped re-review), subagent-driven, in six commits: 169d03b (the syntax and the checker), 7e1f68c (`@assume` lowered and checked at run time), 93cb443 (range analysis uses assumptions, `--release` trust), e8429d4 and ce6bef0 (contract plumbing: sites, regions, inlined copies, the driver step), 5be3f31 (deciding contracts through calls), then the measuring and docs commit. Tasks 1a, 1b, 2 and 4 passed their reviews with no Critical or Important finding; Task 3's review found one Important (below). Every task ran only focused tests in light mode (`-p <crate>`, named test targets, `KESTREL_TEST_ONLY` filters). **The full `cargo test --workspace --no-fail-fast` (with the exe pass without the proof check) and the whole clippy run are owed** until the user says the machine is free. The final whole-round review and its fix wave are still to come.

### What differs from the addendum

1. **The statement budget** is the base scaled by `(statements + extra) / statements`, not the base with extra statements counted against it (the addendum's first wording, now amended above). A function under 2,000 statements stays at the 200,000 floor, so the literal reading gave it nothing extra, and an assumption could push a function near its budget into giving up. The result is never below the literal formula and identical for a function with no assumption. A test fails with the literal one.
2. **Regions carry the copy**, not only the site: `Region { id, at, copy }` (the site, the `Prove` block's call line, and the block's number among the function's `Prove` blocks in `visit::walk` order). Two inlined copies of one contracted function in one caller are told apart, so the error names the right call line. This was Task 3's review finding; the addendum's regions held only the id.
3. **`CallVirtual` gained `keeps` and `method`.** `keeps` is in the addendum. `method` (the interface method's own stub function) is extra: KIR had no other way to name the interface in the help line "add `@prove(no_alloc)` to the interface method `Shape.area`; ...".
4. **Loop sites are made when the loop is lowered**, keyed by the annotation, so a loop lowered twice shares one id. A loop that is never lowered (inside a closure literal of a field default that is never constructed) has no compiled copy and gets no site. In debug builds lowering asserts that every loop site's function holds its `Prove` block.
5. **The inliner reads a contracted callee's value back with `Load`**, not `Move`, from the temporary inside the `Prove` block: it has no checker tables to tell heap types apart, and the temporary is never dropped.
6. **A function body's tail value inside an assumption** follows what the function would do without it: `owned(v, true)` when the body's scope has no cleanup, and `owned(v, false)` plus a `Move` of the temporary when it has one (an owning local such as an `Arena`), as `block_value` does on its cleanup path. A final `return e` is `owned(e, true)`, as `return_stmt` does. An `if` or `match` arm that gives a value is always `owned(v, false)`.
7. **Conditions in the plain run** are evaluated for their own checks and teach nothing, not even the facts from checks that passed. Under `--release` the real run drops those facts too, since a trusted condition never runs. Today no condition can teach one (the form forbids it), so this closes a latent hole.
8. **`What::Cut` counts as a deep give-up for contracts**; the report still doesn't render it (a cut function is always one the analysis gave up on, and that is printed). **`What::Unknown`** replaced the report's string-shaped allocation and failure pair for function values, interface calls and natives. The report's output didn't change (its exact fixture and the range-report counts pass unchanged).
9. **Failure events carry a part** (`Whole`, `Logic`, `Resource`). `insert` and a map key set are two failures each: the logic part (an index outside the list; a key added while looping over the map) counts for `no_panic`, and the item-limit part doesn't. The report prints the combined words as before.
10. **Wording the addendum left open:**
    - a chain into an assumption reads "checks the assumption `n > 0`";
    - a function other than the contract's own that range analysis gave up on reads "range analysis stopped early in `big` (its code nests too deeply), so nothing can be proven about it; split it into smaller functions";
    - an undecided site reads "range analysis missed a check in it (a compiler bug), so nothing can be proven about it";
    - the constants' reason for `main` has no line, and chains through calls get the default help line;
    - one error per (site, contract), the contract's own copy first, then each inlined copy.
11. **Parsing details:**
    - a comment line between an annotation and its target counts as the second line break (an error);
    - `@assume(a) foo()` on one line is "expected end of line" (only another annotation may follow an `@assume` on its line);
    - after an annotation error the rest of that line is skipped, so one mistake gives one error;
    - an assumption's text is its tokens from the source joined with one space, so a `//` comment inside a multi-line condition never reaches the run-time message.
12. **Test harness:** `// expect-exit-proofs:` and `// expect-stderr-proofs:` directives give the proof-check run its own expectation (a trusted false assumption fails differently there). The release pass of the programs test also fails on "can't be proven" (every `prove_*` program must hold in both builds).

### Holes the reviews found (all closed)

- **Plan review (Opus):**
  - C1: a function value or interface call one call below a contracted region was invisible to `no_checks` (now `What::Unknown`, decided per contract).
  - C2: a loop contract inside a `par for` body was attached to the outer function and passed without checking anything (the site is re-pointed when lowering moves statements into a new function, and a site whose function doesn't hold its block fails the contract).
  - C3: trust decided per assumption id let the wrong copy of an inlined assumption be trusted under `--release` and read outside a list (trust is per statement address).
  - I1: a contract covered code that may never run and not the inlined copies that do (every inlined copy is a `Prove` block and is checked).
  - I2: recursion passed `no_panic`.
  - I3: spilling a returned parameter list's tail aliased the caller's buffer (the `owned(v, false)` pattern; test `assume_tail_copy.kes`).
  - I4: the real run was wasted in non-deciding passes and the doubled visits spent the budget (only a deciding pass runs it, and the budget scales).
  - I5: `no_panic` counted the item limit, so any growth failed it.
- **Re-review:**
  - N1: marking `insert` and a map key set as resource failures would have let `no_panic` pass on a bad index, or on a key added in a loop over the map (the two-part failures, item 9).
  - N2: a test that couldn't pass, because Round A learns nothing about a divisor of any `int` (the divisor is a `u8` now).
  - Preflight conflicts P1-P4 (file names, line numbers).
- **Task 3's review:** two inlined copies of one contracted function in one caller were indistinguishable, so Task 4 would have named the wrong call line (item 2).
- **Task 4's own check of itself:** recursion through an interface call whose method promises `no_panic` was invisible (a tree whose `weight` calls `k.weight()` on its children). A promised interface call is now an edge to every implementation in the recursion check, and a promised call in a region fails `no_panic` when an implementation can recurse.
- **Task 1b's review:** the brief's literal tail rule could return an arena-backed value uncopied; the implementer's cleanup rule (item 6) keeps copy, move and drop identical to code with no assumption.
- **Task 2's review:** no counterexample found. Its carried finding (conditions teaching the plain run) is item 7.
- **Task 4's review:** fourteen probe programs found no unsound pass. The mutation tables in the task reports show which test fails when each rule is broken.

### Rulings made while building

Made on the user's behalf, with what each costs if wrong:
- **Every inlined copy is covered (I1):** a failure in a copy names the caller. Cost: spurious failures that blame callers; relaxing is easy.
- **Recursion fails `no_panic` (I2):** the spec is binding. Cost: recursive code can't carry `no_panic`.
- **`no_panic` excludes running out of memory and the item limit on growth (I5),** but not a sized creation (`array[T](n)`) nor `insert`'s index. Cost: a `no_panic` function can still stop on huge growth; `no_alloc` rules growth out.
- **Task 3 ran on Opus, and Task 1 was split** into parsing (Sonnet) and the run-time `@assume` lowering (I6). Cost: some Opus time.
- **The two syntax picks** (the addendum's "(1)" and "(2)") were built as written, each in one place (`MAX_LINE_BREAKS` in `parser/annot.rs`; `assume_side` in `check/src/body/assume.rs`), and **are still waiting for the user.**
- **No re-review of the N1 and N2 fix wave as a plan:** Task 4's and Task 2's task reviews covered it. Cost: a plan defect would have surfaced in the task loop (none did).
- **Items 1, 2, 3 and 6 above:** the budget formula, the per-copy fix in Task 3 rather than as Task 4's first step (cost: a little scope in Task 3), `CallVirtual { method }`, and Task 1b's tail rule.
- **An `@assume` in a function with a `no_panic` contract can't hold in a checked build** (ruling 4 above). Moving the assumption to the caller doesn't help, because facts don't cross calls.

### Measured (2026-10-21)

Method: the compiler at a0f42fc (the commit before Task 1a, built in a worktree at `C:\kt\rb-old`, removed afterwards) against the one at 5be3f31 (release builds, `CARGO_BUILD_JOBS=2`). `kestrel build` of `kestrel/bench/voxels3d/main.kes` (the frozen benchmark, no annotations) to an exe with the MSVC link, one warm-up, then 9 runs of each, taking turns, timed with `Measure-Command`. The machine was lightly loaded. Medians, with ranges. The tables and the scratch programs are in `kestrel/bench/voxels3d/results.md` ("Round B").
- **No annotations (target: no measurable change, under 2%): met, at the edge.**
  - Checked: 178 ms (170-190) old, 181 ms (171-191) new; run again, 178 (174-198) and 181 (174-185). The ranges overlap entirely, and the medians differ by 3 ms (+1.7%) in both sets.
  - `--release`: 157 (151-185) and 159 (149-178); then 162 (157-180) and 160 (153-166). No change.
  - `kestrel check --report` of the program, checked and `--release`, is byte-identical between the two compilers (184 and 164 lines). `kestrel check` without annotations is 20 ms in both.
- **With `@prove(no_alloc)` on `hash`, `lattice` and `octave` (target: within 10% of unannotated): met.** `kestrel build` checked 183 ms (178-187) against 175 (169-180): +4.6%. `--release` 159 (150-165) against 155 (146-158): +2.6%. All three contracts hold in both builds.
- **With one `@assume` as well (same target): missed.**
  - The assumption sits at the top of `solids`: `cx`, `cy` and `cz` between 0 and 1,000, `seed` between 0 and 1,000,000. The chunk offsets and `seed + 1`, `seed + 2`, `y - 64` need it.
  - Checked 296 ms (290-303), **+69%** over unannotated and +62% over the contracts alone. `--release` 255 ms (246-269), **+65%**.
  - The cause is the analysis: `solids` holds the inlined noise code (46 of the 75 ms Round A measured), and the assumption's block runs twice in the deciding pass. It is more than twice that cost, and I did not profile it.
  - The same assumption at the top of `bench`, whose block is small, costs nothing measurable (185 ms (175-280) against 187 with the contracts alone; `--release` 169 against 165). The cost follows the size of the block an assumption wraps, so write an assumption close to the code it helps.
- **`kestrel check` with an assumption (target: at most 2.2 times without): missed.**
  - Without annotations `check` is the front end only: 20 ms. With the contracts it runs the analysis once: 98 ms (89-101) checked, 72 ms (68-77) `--release`.
  - With the assumption added: 278 ms (264-283) checked and 271 ms (264-289) `--release`, **2.84 and 3.76 times** the contracts-only time.
  - `kestrel check` analyses both builds when a program has an assumption (for the warning), so it pays the doubling twice.
- **An assumption can cost precision.**
  - On the voxel world the assumption removes 12 checks in `solids` (the nine chunk offsets, `seed + 1`, `seed + 2`, `y - 64`), and 27 come back in `mesh`. Checked: 303 removed and 36 left, against 318 and 21 without it. `--release`: 4 removed and 4 left, against 7 and 1.
  - Part of it reproduces in a small program: an `@assume` that wraps the end of a function loses what is known about the array the function returns, so `solid[i]` in the caller keeps its bounds check (the repro is in the follow-ups file). It also happens with `@assume(seed >= 0)`, which removes nothing and gets the warning.
  - The other 24 (`base + 1` to `base + 3` in the six inlined `Mesh.quad` copies) did not reproduce in a small program and are not explained. Not fixed.

### Still owed

- The full suite and clippy (above), and the final whole-round Opus review with its fix wave. The wave has one must-fix from Task 2's review: the deciding path's plain run doesn't forget the conditions' writes the way `plain_block` does. It is latent, since the checker forbids calls and indexes in conditions, but it breaks Round A's "conditions that write" rule; the fix extends `a_condition_that_writes_is_forgotten` with an index after the block.
- The deferred minors from every task, and the compile-time and precision misses above: `docs/superpowers/plans/2026-10-09-core-2a-followups.md`, "Found while building `@prove` and `@assume`".
