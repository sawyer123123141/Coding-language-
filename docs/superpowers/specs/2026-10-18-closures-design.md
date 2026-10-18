# Closures and list methods

Status: designed 2026-10-18. The user took every pick in the proposal (`docs/superpowers/plans/2026-10-18-closures-interfaces-proposal.md`) and Claude's advice to build in two rounds: **this round is closures, function values and the list methods they unlock; interfaces are the next round**, designed against the closure code that this round builds. `is` type tests wait (the open question; the user took the "later" lean). The `@` annotations have their own spec (`2026-10-18-at-annotations-design.md`); they are built after range analysis.

Rulings in this document that aren't the user's picks are Claude's; each is listed at the end with what it costs if wrong.

## Why

Kestrel has no way to pass behaviour around. Lists can't be filtered, mapped or sorted, which list and string basics deferred to this design. GDScript users write `enemies.filter(func(e): return e.hp > 0)` and `sort_custom` every day.

## Closures

A closure is a function with its name cut off, as GDScript's `func(e): ...` is `func name(e): ...` without the name. Kestrel functions have no keyword, so:

```
square(x: int) -> int { x * x }                    // named function
let sq = (x: int) -> int { x * x }                 // the same, as a closure
let alive = enemies.filter((e) { e.hp > 0 })       // e's type comes from context
enemies.sort_by((e) { e.pos.distance_to(player) })
let hurt = (amount: int) { print("ouch {amount}") }
hurt(3)
```

### Syntax

- **A closure literal** is `(params) [-> Type] { body }`, an expression. Parameters are written as in a function: `name: Type`, `var name: Type`, or, when the context gives the types (below), `name` and `var name`. `()` takes nothing.
- The body is a block; its last expression is the result, as in a function, and `return v` returns from the closure.
- **Telling a closure from a parenthesised expression.** In expression position, a `(` whose matching `)` is followed by `{` or `->` starts a closure. Exception: in the head of an `if`, `while`, `for ... in`, `for ... from ... to`, `match` or `in ... {` (outside any parentheses, brackets or braces of its own), `(cond) {` stays a parenthesised expression and its block, so `if (ready) { go() }` and `in (w.mem) { }` keep working. Inside parentheses, brackets, a map literal's braces or a closure's body, the exception is lifted, so a closure in a head goes inside a call's parentheses (`if xs.any((e) { e.dead }) {`). This is Rust's rule for struct literals in conditions.
- **A closure on a statement of its own** (`(x: int) { x }` as a whole statement) gets the "does nothing" error.
- **The postfix call** `expr(args)` sits with `.` and `[]` in the postfix chain (`handlers[i](3).len`).
- Newlines: a closure's `{` on the same line as its `)` (like a function's). Inside a call's parentheses newlines are already dropped, so a multi-line body inside `xs.filter((e) { ... })` works; the body's own statements are separated by newlines as in any block. **Builder:** check this against `newlines.rs` (newlines are dropped inside `(` and `[` but not `{`), and make a multi-line closure body inside call parentheses parse its statements correctly. If the existing filter can't support it, the fix is to make a `{` reached inside parentheses re-enable newlines until its `}`.
- **Function types:** `(A, B) -> R`; `(A, B)` gives nothing; `()` takes and gives nothing; `pure (A) -> R` is a pure function type; a parameter changed in place is `var A` (`(var Particle, float)`). They are types like any other: `list[(int) -> int]`, struct fields, `let` annotations, parameters, results.
- **Optional function types are not in this round.** `(int)?` would read, but `((int) -> bool)?` can't be told from a function taking a function. A callback that may be absent gets a do-nothing default instead: `on_hit: (int) = (n) { }`. The error for `T?` of a function type says so.
- **Calling:** `f(x)` when `f` is a local or parameter of function type (a local can't share a function's name: `link` already reports that clash). `self.on_hit(3)` and `b.on_click()` call a field of function type when the struct has no method of that name; a struct that declares a field and a method with the same name is refused at the declaration ("`Button` has a field and a method both called `on_click`; rename one"). Any other expression of function type is called with a postfix call: `handlers[i](3)`, `make_adder(2)(5)`.
- **A call through a function value can't change what that value's environment may live in:** the place the value is read from counts as a capture of the call, under the exclusivity rule below (overlap, or a change or move of owning type under its root), and so does the owner of any arena the value's region points into. So `w.cb(var w)` and `w.cb(var w.mem)` are refused: "can't call `w.cb` while this call changes `w`, which may hold its memory".
- **Arguments through a function value are positional:** function types have no parameter names, so `f(amount: 3)` is refused ("calls through a function value take no labels"). A `var` parameter is marked at the call site as usual: `f(var p, 0.5)`.

### Types and inference

- **Parameter types may be left out when the context gives them:** the closure is a list method's argument, the argument of a parameter of function type, the value of a `let`/`var` with a function-type annotation, a value assigned to a field or element of function type, a constructor argument for such a field, a field default, or a `return` in a function whose result is a function type. Otherwise every parameter needs a type: "give `x` a type: `(x: int) { ... }`; Kestrel can only leave it out where a function type is expected".
- **The result type is inferred** from the body (its tail and its `return`s, which must agree), unless written with `-> T`. Where a function type is expected, the body is checked against its result type, so the usual coercions apply (an `int` tail where `float` is expected).
- **A closure's type is `pure` when its body is pure** (the rules for a `pure` function: no `print`, no non-pure calls, no random numbers; it may change its own locals and its own `var` parameters). So most closures are pure.
- **Assigning function values:** the parameter types (and which are `var`) and the result type must be the same; there are no conversions between parameter types. A pure function value goes where a non-pure type is expected, not the other way round: "this closure isn't pure (it calls `print`), but `sort_by` needs a pure one".
- **Named functions are values:** `let f = score`, `xs.sort_by(score)`, and a module's `util.score`. Their type is their signature, `pure` if declared `pure`. The old error "`foo` is a function; call it with `foo(...)`" goes away for user functions. Not values in this round:
  - methods (`e.take_damage`): "methods can't be used as values yet; wrap it: `(e) { e.take_damage(1) }`"
  - built-in functions (`min`, `sqrt`, `print`), which take several number types: "`min` is built in; wrap it: `(a: int, b: int) { min(a, b) }`"
  - natives (`draw.rect`), which use their own calling rules: "native functions can't be used as values yet; wrap it in a closure"
- **No `==` or `!=`** on function values ("function values can't be compared"), nor on structs that hold one, and they can't be map keys or `has`/`find` items.
- **A function value is an aggregate** (two words, `{code, env}`), in the checker and in KIR: it is passed by pointer like a struct, so a parameter of function type is a `RefParam`, and storing or returning one copies it (the copy rules below depend on this). A struct with a function field isn't a plain struct for `scalarize`.
- **`print`** of a function value, or of a struct holding one, writes `<function>`.

### Captures

A closure **captures** the locals and parameters of the functions and closures around it that its body names (globals, constants and functions aren't captures; they are read directly).

- **Captures are by value**, as in GDScript, where "local variables are passed by value". A closure sees the values its captures had when it was made. (GDScript users may expect `self` to be live, as it is for a GDScript lambda; in Kestrel `() { self.name }` sees `name` as it was when the closure was made. To see current values, pass them as parameters.)
- **A capture is a path, not a whole variable:** what is captured is each place the body reads that is rooted outside it, followed through fields as far as the body goes and stopping at an index, a built-in property or a method call (`self.name`, `w.score`, `b.pos.x`; `xs[i]` captures `xs` and `i`; `xs.len` captures `xs`; `self.pos.distance_to(p)` captures `self.pos`; `w.fire()` and passing `w` whole capture `w`). A path inside another captured path is covered by it. A view's capture path is its source's path followed by its own steps (a loop item `b` of `for b in w.bricks` is `w.bricks[..]`, which overlaps all of `w.bricks`). So `() { self.name }` copies one string, not all of `self` (and not `self`'s other closures).
- **A stored closure can't capture a whole value that holds a function value** (`self.cb = () { self.fire() }` captures `self`, which holds `cb`): every reassignment would copy the old closure into the new one, without bound. "this closure would copy all of `self`, including its function fields; copy what it needs into a local first". A closure passed straight to a call may (it copies nothing).
- **A closure can't change a captured variable**: assigning it, `+=`, passing it as `var`, a changing method (`xs.add`), `remove`, or moving it out. "`count` is captured by this closure, so it can't change here; return a value instead, or use a `for` loop." This closes GDScript's silent gotcha (changing a captured `int` changes only the closure's copy).
- **Owning values can't be captured** (anything holding an `Arena` or a GPU object): "a closure can't capture `tex`: it owns a GPU texture; pass it as a parameter instead". So a closure is never owning.
- **Views can be captured** (a loop item, a `match` binding, a `let ... else` view): the closure gets the item's value at that moment.
- Closures nest: a closure inside a closure can capture its captures.
- `break` and `continue` can't cross a closure's boundary: "`break` can't leave this closure; it is a function of its own". `remove b` of an outer loop's item inside a closure is refused the same way.
- `self` in a method can be captured like any parameter (a copy, when the closure is stored).

### When captures are copied

A closure's environment holds its captured values (never addresses). Copying their heap memory (a captured string's bytes, a list's items) is invisible except for its cost, so the compiler avoids it where the closure can't outlive what it captured:

- **A closure literal written as the argument of `filter`, `map`, `any`, `all` or `sort_by`** (only these five) is specialised: it isn't a value at all, its body reads the captured places directly, and it allocates nothing (see "How it's built").
- **A closure literal that is directly the argument of a call** to a user function, a user method or a function value (the argument expression itself, not inside an `if`, `match` or block) gets a **stack environment**. Not for any other call: `add`, `insert`, map stores, constructors, enum variants, `print` and built-ins store or keep what they are given, so a literal there gets a heap environment. The checker decides which literals get stack environments and records them; KIR only reads that record, so the exclusivity rule and the lowering never disagree. A stack environment is: a shallow copy of the captured values (a string's or list's header, not its bytes) in the caller's frame, taken when the argument is evaluated. The callee can't keep it, because storing or returning a parameter copies it, and a copy deep-copies the captured values (below). KIR keeps the invariant: a function value with a stack environment appears only as a call argument, and `owned` always copies one.
- **Every other closure literal** (stored by `let`, a field, `add`/`insert`, a map value, `return`, a constructor argument, or reached through `if`/`match`/a block) is built with a **heap environment**: deep copies of the captured values, in the current arena.
- **Copying a function value** (any store of a non-fresh one, as for strings) deep-copies its environment into the current arena.
- **Capture-less closures and named functions allocate nothing**, ever: they have no environment.
- **Exclusivity:** a shallow copy shares heap memory with the caller's variables, so nothing may change or free those during the call. A closure literal that is directly a call's argument (stack environment or specialised) can't capture a path that conflicts with anything the call can change. **The changes:** a `var` argument, a `var self` receiver, the receiver of `sort`/`sort_by`, the place the called function value is read from (`w.cb(var w)`, `handlers[i](var handlers)`), a place changed anywhere else in the call's arguments (`f(() { n }, bump(var n))`), and a value moved into the call or into any of its arguments (`f(h, () { h.name })`, `f(() { h.name }, consume(h))`). **A capture conflicts with a change** when their paths overlap (`PathKey::overlaps`, with views expanded to their source path), or when the change is a move of, or a change to a place of owning type (or containing one) under, the capture's root: an `Arena` in a sibling field may hold the captured string's bytes (`in h.mem { h.name = ... }`, then `f(var h.mem, () { h.name })`). "`xs` is changed by this call, so the closure passed to it can't read it." So `self.items.sort_by((e) { e.pos.distance_to(self.center) })` compiles (`self.center` and `self.items` don't overlap, and sorting frees nothing), and `world.step((e) { world.g })` with `step(var self, ...)` doesn't. A heap environment needs none of this: it holds its own copies.

### Memory and lifetimes

- A stored closure with captures is a heap value, like a string: its environment lives in the arena that was current when it was built (or copied), so the escape check treats it exactly like a string. Storing one into a longer-lived place needs `in x.mem { }`; returning one built inside `arena { }` is refused with the existing messages.
- A capture-less closure or a named function holds no memory: its region is empty, like a literal's, so it can go anywhere, including out of `arena { }`, field defaults (`on_hit: (int) = (n) { }`, and `default_allocates` is false for it) and constants (`let RULES = [score, bonus]`, a heap constant built before `main` like other arrays).
- A closure literal with captures can't be a field default or a constant (there is nothing to capture there anyway, so this never comes up).
- **Inside a closure's body,** regions are checked as for a function of its own: the body runs in whatever arena is current when it is called, so an `arena { }` around the closure literal doesn't make the body's allocations scoped (`xs.map((e) { "{e}" })` inside `arena { }` is fine). Its captures count like its parameters (the caller's memory), so returning a capture copies it, and nothing can be stored into one.
- **`sort_by` frees what its key builds:** each comparison runs in a scratch arena freed after it (skipped when the key can't allocate), so `enemies.sort_by((e) { e.name.lower() })` every frame doesn't grow memory. `filter`, `map`, `any` and `all` allocate in the current arena like a hand-written loop does (wrap a per-frame loop in `arena { }`, as today).

### `pure` and `par for`

- A `pure` function, and a `par for` body, may call a function value only if its type is `pure`: "`f` isn't a pure function type, so a `pure` function can't call it; declare the parameter as `pure (int) -> int`".
- **A closure literal written inside a `pure` function or a `par for` body must be pure** (its body can't change captures anyway, so this means no `print` and no non-pure calls): "this closure calls `print`, which a `par for` body can't do".
- **A function value that isn't a literal,** passed to `filter`, `map`, `any` or `all` inside a `pure` function or a `par for` body, must have a `pure` type (the helper calls it there).
- **Captures inside `par for` are reads:** making a closure in a `par for` body reads its captured paths, and they are recorded like any read there, so the existing rules refuse capturing the collection (or a path over it) while iterations change its items.
- A stored closure built inside a `par for` body allocates in that iteration's arena, which `par for` merges into the caller's.

### Godot

A `pub` function with a function type in its signature compiles but isn't callable from Godot (`functions()` doesn't list it, like other unsupported signatures).

## List methods

GDScript's names where it has them. Lists and arrays (`[T]`) both get all of them; `sort` and `sort_by` change their receiver, so they need a `var` root as `add` does.

```
let alive = enemies.filter((e) { e.hp > 0 })         // new list
let spots = enemies.map((e) { e.pos })               // list[vec2]
if enemies.any((e) { e.boss }) { alarm() }
if enemies.all((e) { e.hp <= 0 }) { win() }
scores.sort()
enemies.sort_by((e) { e.pos.distance_to(player) })   // in place, by key
```

| Kestrel | GDScript | Meaning |
|---|---|---|
| `xs.filter(f)` | `filter` | A new `list[T]` of the items for which `f(item)` is `true`, in order. |
| `xs.map(f)` | `map` | A new `list[R]` of `f(item)` for each item, in order. |
| `xs.any(f)` | `any` | `true` when `f` is `true` for some item; stops at the first. `false` for an empty list. |
| `xs.all(f)` | `all` | `true` when `f` is `true` for every item; stops at the first `false`. `true` for an empty list. |
| `xs.sort()` | `sort` | Sorts in place, smallest first. Items are numbers, strings or bools. |
| `xs.sort_by(key)` | (`sort_custom`) | Sorts in place by `key(item)`, smallest key first. Keys are numbers, strings or bools. |

- **`f` and `key` take the item read-only:** `(T) -> bool` for `filter`/`any`/`all`, `(T) -> R` for `map`, `pure (T) -> K` for `sort_by`. Any function value of that type works (`xs.filter(is_alive)`, a stored closure, a parameter), not only a literal.
- **Order of calls:** `filter`, `map`, `any` and `all` call `f` once per item, first to last (`any`/`all` stop early), so a closure that prints shows them in order. The `sort_by` key must be `pure`: it runs about twice per comparison, so its order is not promised.
- **Sorting is stable** (equal keys keep their order). Numbers compare as numbers; strings by character (code point) order, as GDScript does; `false` before `true`. A float NaN sorts after every other number, so a sort always finishes and stays stable. Largest first: negate a number key (`-e.hp`); a reverse for strings waits for a later `reverse()`.
- **`filter` copies the kept items** into the new list in the current arena (as `xs[i]` does), so it refuses owning items: "`filter` would copy the items of `sprites`, which own GPU textures; use a `for` loop with `remove`". `map` builds new items from `f`'s results, so its results may be owning (they are fresh values moved into the new list; a closure that returns its owning parameter, `(s) { s }`, hits the existing rule against moving out of a parameter), but `R` can't be nothing (`f` must give a value: "`map` needs a closure that gives a value").
- **`sort`/`sort_by` inside a loop over the same list** (or an overlapping one, as for `remove_at`) are refused: "can't sort `xs` while looping over it; sort it after the loop". The key closure can't capture the list it sorts (the exclusivity rule above).
- `sort`/`sort_by` on a list of owning items is fine: items are moved, never copied.
- A discarded `xs.filter(f)`, `xs.map(f)`, `xs.any(f)` or `xs.all(f)` says the line does nothing, as `has` does.
- Wrong item types: "`sort` needs numbers, strings or bools; `list[vec2]` holds `vec2`; use `sort_by` with a key". `sort_by` with a key of another type: "`sort_by` needs a key that is a number, string or bool; this one gives `vec2`".

Not in this round: `reduce` (a `for` loop reads clearer), a comparator sort like `sort_custom` (a key can't be written inconsistently; later if asked), `keep_if` (`remove b` in a `for` already removes in place), `reverse`, `find_by`, `min_by`/`max_by`.

## How it's built

This is guidance for the plan; the builder may change details that don't change behaviour.

- **Checker.**
  - A new `Ty::Fn(SigId)` with an interned signature table (parameter types and `var` flags, result, `pure`), like `ElemId`; `Ty` stays `Copy`. It is not owning, and `has_heap` is true (a stored closure's environment is heap memory).
  - A closure literal's body is checked inside the enclosing `BodyChecker` with a closure frame, the way `par for` bodies are checked (`ParBody` with `first_local`): locals declared before the frame and reached from inside are captures (recorded per closure as paths), writes to them are refused, `return` returns from the frame, `break`/`continue` stop at it. The frame records the closure's inferred purity and result type. Locals then belong to a closure as well as a function; `LocalInfo.owner` and `add_local` need to say which.
  - **What the frame resets and keeps:** it starts its own region context stack (`[Function]`, with captures treated as parameters) and its own `local_ctx` entries, its own loop state (`loop_breaks`, `removing` loops, the enclosing loops for `remove`) and its own move state for its locals; it keeps the `par` stack (so capture reads are noted) and the enclosing function's purity context (for the "must be pure" rules).
  - Expected types flow into closure literals (for parameter types) from the list methods, function-typed parameters, annotated `let`s, field and element stores, constructor arguments, defaults and returns.
  - Calls: `ExprKind::Call` by a local's name, a field call through `check_method_call`'s fallback, and a new postfix call expression for other callees.
  - Several checker files are at 369-401 lines (`tables.rs`, `collect.rs`, `body/mod.rs`, `body/call.rs`, `body/regions.rs`, `body/expr.rs`, `body/lists.rs`); split them by responsibility before adding to them.
- **KIR.**
  - A function value is `KTy::Fn`, an aggregate of two words, `{code, env}`. The code is a KIR function taking the environment pointer first, then the parameters. A named function used as a value gets a generated thunk with that shape that ignores the environment. A capture-less closure has a null environment. There is **one KIR function per closure literal** (keyed by its `ExprId`), however many times it is lowered (a field default is lowered at each construction).
  - **The environment holds values.** It starts with a header (the copy function and the size), then the captured values at fixed offsets. A stack environment is a stack slot filled with shallow copies (read through the normal KIR reads, so a value `promote` keeps in a register is read correctly); a heap environment is allocated in the current arena and filled with deep copies (the existing `owned`/`copy_fn`). The copy function deep-copies each value into a new heap environment. One code body serves both.
  - **Inside the closure's code, each capture is a place that behaves like a `RefParam`** (bound to its address in the environment): reading it into a new home or returning it copies it, exactly as for a parameter. Without this, `let g = () { xs }` would hand out lists that share one buffer.
  - **Literal closures passed to `filter`/`map`/`any`/`all`/`sort_by` are specialised:** the closure body becomes a KIR function that takes its captured places as `RefParam`s (`Arg::Ref` of the paths), like a `par for` body, and the method's loop is a helper generated for that call site that calls it directly. `inline_small` then inlines the closure into the helper (and, size permitting, the helper into the caller), so `xs.filter((e) { e.hp > 0 })` runs like a hand-written loop. Because captures are passed as `Arg::Ref`, `scalarize` pins them and `promote` leaves the region around the call unpromoted, as for any `var` argument. (The builder may instead pass the literal's closure code a stack environment and call it directly; the inliner removes the call either way. The plan does that, so one closure lowering serves every case.) A function value that isn't a literal goes through a helper per (list type, function type) that makes an indirect call.
  - New nodes: making a function value (with a stack or heap environment), an indirect call, and capture places; every visitor (`visit.rs`, `visit_mut.rs`, `rename.rs`, `inline.rs`, `scalarize`'s pinning and `leaves()`, `promote`'s exclusivity) learns them. An indirect call's `var` arguments pin their places like any `Arg::Ref`.
  - **Devirtualising** (turning an indirect call of a value known to be a particular function, after inlining, into a direct call) is not in this round; it is a follow-up.
- **Codegen.** An indirect call imports a signature built from the function type (aggregate result pointer first, then the environment pointer, then the parameters, by the existing rules) and uses `call_indirect`. Function addresses use `func_addr`, which already works for the JIT and object files.
- **Sorting.** A runtime stable sort over the list's bytes, called with a compare function generated per call site (the key inlined into it; it gives before, equal or after, so the key runs exactly twice per comparison) and its environment: it sorts an index array with Rust's stable `sort_by`, then permutes the items through a temporary buffer. Items are moved as bytes, which is safe for every Kestrel value. The compare function is a total order (NaN last), so Rust's sort never sees an inconsistent comparison. A runtime error inside the key (an index out of bounds) fails as usual: the process exits, or the Godot add-on's failure hook parks the thread, as in a `par for`. The builder may write the sort as generated KIR instead if that is simpler; behaviour is the same.
- **`filter`, `map`, `any`, `all`** are generated KIR loops (the `has`/`find` pattern), with unchecked indexing proven by the loop bounds.

## Tests

Program tests for every feature above (JIT and exe), error tests for every message, and these in particular:

- a closure stored in a struct field, called through `self.on_hit(..)`, with captures of a string and a list; copied into another struct and the original changed; and stored into a longer-lived place with `in x.mem { }` (and the refusal without it);
- a closure returned from a function, called after the function's locals are gone;
- a closure built inside `arena { }` and returned out of it (refused) and used inside it (fine);
- `KESTREL_ARENA_REPORT` showing that `xs.filter((e) { e > k })` in a 100,000-frame loop inside `arena { }` stays flat, and that a closure literal passed to a function-typed parameter allocates nothing;
- `filter`/`map`/`any`/`all`/`sort_by` with a literal, a named function, a stored closure, and a function-typed parameter;
- a stable sort with many equal keys; NaN keys; strings with non-ASCII characters; an empty list and a one-item list; sorting a list of owning items (GPU-free: a struct holding an `Arena`);
- purity: a printing closure refused by `sort_by` and by a `pure (int) -> int` parameter; a pure function calling a non-pure function value refused; a closure in a `par for` body;
- the parenthesised `if (ready) { ... }` still parsing as a condition;
- nested closures capturing an outer closure's capture;
- from the spec review, each a test: `fs.add((x) { x + n })` then `n` changed and the list's closure called after the function returns (heap environment); `run(if c { (x) { x + n } } else { h })` where `run` stores its parameter; `w.cb(var w)` where the closure frees `w.mem` (refused); a stored closure in a `par for` body capturing `w.score` while iterations change `w.chunks` items (allowed) and capturing `w` (refused); `let g = () { xs }` with two results changed separately; `self.items.sort_by(... self.center ...)` (allowed) and `world.step((e) { world.g })` with `var self` (refused); `f(() { n }, bump(var n))` (refused); `self.hp = 5; apply(() { self.hp })` in a `var self` method prints 5; `self.cb = () { self.name }` reassigned inside `in self.mem { }` 1,000 and 10,000 times grows the arena by the same amount per assignment (no nesting) under `KESTREL_ARENA_REPORT`; `xs.filter(f)` in a `pure` function with a non-pure `f` (refused); `sort_by` with `e.name.lower()` every frame stays flat; returning a named function out of `arena { }` (fine).

**A measurement** in `kestrel/bench/kernels` (the existing harness): `filter`, `map` and `sort_by` against the hand-written loop (and, for the sort, against Rust's `sort_by_key` on the same data), so "runs like a hand-written loop" is checked, not assumed. Results in its `results.md`, losses included.

## Docs

The syntax spec's closure and list sections, the "Out of scope" line (closures and sorting move out; interfaces stay), and the README's feature list.

## Rulings made on the user's behalf

Each with what it costs if wrong.

1. **Two rounds** (closures now, interfaces next), Claude's advice that the user took. Cost: interfaces arrive one round later.
2. **Closure parameters may be `var`,** and function types say so (`(var Particle, float)`). Cost: a little more checker work; it keeps closures and functions the same.
3. **Optional function types are refused this round;** a do-nothing default replaces them. Cost: no "maybe there's a callback" without a default; `((A) -> B)?` needs a grouping syntax first.
4. **Calls through a function value are positional.** Cost: no labels on those calls.
5. **Methods, built-ins and natives aren't values yet;** wrap them in a closure. Cost: a little more typing.
6. **Captures are read-only.** Cost: a closure can't count or accumulate into an outer variable; a `for` loop does that.
7. **Owning values can't be captured,** so closures are never owning. Cost: a closure can't keep a texture; pass it as a parameter.
8. **Heap memory is copied only when a closure is stored;** a literal that is directly a call's argument gets a stack environment of shallow copies, under the exclusivity rule (by path, against everything the call can change). Cost: one more exclusivity rule ("`xs` is changed by this call ..."), and a literal reached through `if`/`match` allocates.
7a. **Exclusivity counts owning changes and moves under a capture's root as conflicts,** even on a disjoint path (the second spec review: a sibling `Arena` may hold the captured bytes). Cost: `f(var h.mem, () { h.name })` is refused even when `h.name` isn't in `h.mem`.
7b. **A stored closure can't capture a whole value holding a function value.** Cost: `self.cb = () { self.fire() }` needs the needed fields copied into locals first. A closure that names a function-valued path on purpose (`self.cb = () { self.cb(1) }`, or `compose(f, g)` in a loop) still nests copies; that is growth the program asked for, not a memory error, and is accepted.
8a. **Captures are paths, not whole variables** (the spec review found that whole-variable captures deep-copy all of `self`, nest closures inside `self.cb` without bound, and race with other `par for` iterations). Cost: the checker records paths; semantics are the same as whole-variable capture.
8b. **A closure literal inside a `pure` function or a `par for` body must be pure,** even if it is only stored. Cost: an impure stored closure can't be built there.
8c. **`sort_by` runs each comparison in a scratch arena** (when the key can allocate); `filter`/`map`/`any`/`all` don't. Cost: an arena push and pop per comparison of an allocating key; a per-frame `filter` with an allocating closure grows memory unless wrapped in `arena { }`, as a hand-written loop does.
8d. **No devirtualising this round.** Cost: a named function passed through a function-typed parameter is an indirect call even after inlining.
9. **A stored closure follows the string rules** (current arena, `in x.mem { }`). Cost: as for strings.
10. **`filter` refuses owning items;** `map` allows owning results. Cost: filtering a list of textures needs a `remove` loop.
11. **`sort_by` keys must be pure and are computed per comparison,** not cached. Cost: an expensive key runs about 2·n·log n times; caching number keys is a later optimisation if the measurement shows it matters.
12. **NaN sorts last.** Cost: none known (GDScript's order for NaN is unspecified).
13. **`print` of a function value writes `<function>`.** Cost: no signature in the output.
14. **A field and a method with the same name are refused** (new). Cost: an existing program with both stops compiling; none in the tests is known to.
15. **`if (cond) {` stays a condition;** closures in a head need a call's parentheses. Cost: one special case in the parser.
16. **The sort is a runtime function over bytes with a generated comparison,** not generated KIR. Cost: an indirect call per comparison.

## As built

Built 2026-10-18 on `main` in ten tasks (commits d66824b..40b86cc, in `git log d66824b..HEAD`). The last full run was after the `sort` task: `cargo test --workspace --no-fail-fast` passed 834 tests and clippy was clean in `kestrel/` and `kestrel/godot`. The fix rounds after it (the map-loop refusal and the earlier ones in the list tasks) were checked with focused test targets only, because the user was playing a game; **a full workspace run, workspace clippy and the Godot clippy over everything since `dc98b3d` are still owed.** The kernels in "Tests" (the `filter`, `map` and `sort_by` measurements) are not built or measured yet, so "runs like a hand-written loop" is checked only by KIR tests that show the closure inlined into the loop, not by timing. Everything else in the design was built. What differs from the design, or was added to it:

**Function types and values**
- **`KTy::Fn` is a unit type.** The signature is not in the KIR type; it is on each indirect call (`KSig`), so `list[(int)]` and `list[(float)]` share a KIR type. A helper keyed by KIR type that needs a signature must key on `KSig` too (the list-method loops do).
- **A named function as a value gets a generated thunk** (`__kestrel_fn_<n>`, one per function) that takes the environment word first and ignores it. `FnRef` holds the thunk's index.
- **A function-typed constant is called by its bare name** (`RULE(3)`). The design's call forms implied it; it needed a `call_const` record next to `call_local`.
- **The "field and method with the same name" error is for function-typed fields only.** Other clashes keep their older message ("already has a field named"), which an existing test relies on.
- **Messages for what isn't a value** name the real thing: `` `(e) { e.len2() }` `` for a method (also for enum methods and `vec2` methods), a wrapper written out for built-ins (`min` gets `(a: int, b: int) { min(a, b) }`; `print` gets one too, where it used to be an unknown name), "native functions can't be used as values yet".
- **`print` of a struct holding a function** writes `<function>` for the field, as designed; the code path is `print_aggregate`.
- **An array literal of function values takes its type from the first element.** with a `pure` function first and a non-pure one second (`[cube, square]`), the second is refused, while `[square, cube]` works. Give the list an annotation.
- Calling a function field of a `match` binding (`r.go(5)`), `m[k](x)` and `xs.pop()(x)` (optionals) are not supported.

**Closure literals**
- **A closure parameter can't share a name with a visible local** (ruling, Task 4): the no-shadowing rule has no exception, so `(e) { ... }` inside a `for e in ...` loop is refused. Rename the parameter.
- **A closure's locals belong to the closure.** They are listed in `ClosureInfo.locals`, not in the function's locals, so a literal in a field default or a constant needs no function around it. `LocalInfo.owner` is `None` there. There are no made-up function ids.
- **The frame keeps the outer loop state with a floor** instead of resetting it; `break`, `continue` and `remove` refuse to cross the floor. Behaviour is as the design says. A later loop check that ignores the floor would leak across the boundary.
- **The result type comes from the checked branches,** not from the syntax of the tail (a review fix; the first build guessed from syntax and refused `(x: int) { if x < 0 { return 0 } else { x } }`). A trailing `if` or `match` whose finishing branches all give nothing gives nothing; a mix of a value and nothing is "this branch has no value". Branches that leave (`return`) don't count.
- **`+` on strings works inside a closure written in a constant or field default** (the design's "constants can't join strings" rule is skipped inside a closure body, which runs at call time).
- **A closure body is never `main`:** it runs in whatever arena is current when it is called.
- **Constants are checked in dependency order through closure bodies,** so `let A = (x: int) -> int { B(x) }` before `let B = ...` works. A constant closure that calls itself gets two messages.
- **Purity messages** are the checker's own wording for each site ("this closure calls `print`, which a `pure` function can't do", "... which a `par for` body can't do", "this closure isn't pure (it calls `print`), but `sort_by` needs a pure one").
- The parser finds a closure's end by scanning for the matching `)` once and remembering every bracket it closes, so nested parentheses are scanned once.

**Captures**
- **Capture shape.** A capture records the root as written (a loop item stays `b`), the field path, and the source root and path with views expanded (`w.bricks[..]`). Exclusivity uses the expanded form; lowering reads through the view.
- **A call of a captured function field extends the path:** `() { b.on(1) }` captures `b.on`, not `b`.
- **Rule 7b is narrower than the first reading** (review fix, with a ruling): it refuses a captured struct or enum value that holds a function value anywhere in its fields (including inside a field's list). A captured function value, and a list, array, map or optional of function values, are allowed. A closure that is the direct argument of a call is exempt from 7b, but if it is captured by an enclosing closure that copies, that closure is checked.
- **Owning captures** say "a closure can't capture `p`: it owns an Arena" (no backticks around `Arena`; the other messages have them), or "a GPU object".
- **A move of a captured owning value reports two errors:** the captured-change message and the owning message.
- **No `EnvAlloc` node:** a heap `MakeFn` carries its copy function, layout and values, and codegen allocates. The environment starts with a 16-byte header (copy function, size).
- **Copying a function value** deep-copies its environment through that copy function; a `compose` chain N deep copies N levels on the native stack, and each `h = compose(h, inc)` copies the whole chain. This is the growth ruling 7b accepts.
- **`captures` are `RefParam`s inside the closure's code,** bound to their address in the environment. `promote` never keeps them in registers (a possible later optimisation).

**Stack environments and exclusivity**
- **The checker records which literals get stack environments** (`Tables::stack_closures`), before checking the argument. `filter`, `map`, `any`, `all` and `sort_by` use the same record. Natives' arguments never get one.
- **KIR checks the invariant** that a stack environment appears only as a call's argument (`check_stack_envs`, run on `lower()`'s output in debug builds, and over every program test through `compile_unoptimized`). `owned` turns a stack `MakeFn` into a heap one, and the argument-spilling paths leave it in place.
- **Owning values passed to a non-`var` parameter count as moved into the call** for the exclusivity rule (conservative: Kestrel passes them by pointer). Method receivers don't, or `self.each((e) { self.x })` on a struct that owns an arena would be refused.
- **The sibling-arena rule includes `in x.mem { }` owners:** a local declared inside `in w.mem { }` counts `w` as one of its places. It is mostly redundant (freeing an arena that is on the arena stack aborts at run time) but turns that into a compile error.
- **One error per literal,** at its first conflicting capture. `remove` inside an argument counts as a change of the whole list.
- **The design's `handlers[i](var handlers)` can't be written** (the type would contain itself). The error test uses `struct Bus { handlers: list[(var Bus)] }`, with `bus.handlers[i](var bus)` and `for hd in bus.handlers { hd(var bus) }`.
- **A called value that isn't read from a place is copied first** (review fix). `(if c { w.cb } else { h })(var w)` read its environment from `w.mem` after the call freed it; found under `KESTREL_POISON`. KIR now deep-copies such a callee (`own_callee`) before the call, which allocates once for an unusual call shape. Plain places keep the checker's place rule, with no copy. Inside `in w.mem { (if c { w.cb } else { h })(var w) }` the copy lands in `w.mem`, which the call replaces; the runtime aborts with "an arena was freed while allocating into it" (safe, not a read of freed memory).

**List methods**
- **One loop per literal.** `filter`, `map`, `any` and `all` are generated KIR loops, keyed by the literal's code (or by method, list type and function type for a function value). The loop takes the function value as an argument in both cases, so the stack `MakeFn` is a direct call argument; `inline_small` then inlines the closure into the loop, and the loop into the caller when it fits.
- **A discarded `xs.map(f)`, `filter`, `any` or `all` gets the existing "this only reads the list, so the line does nothing".**
- **`map` with a literal that has written parameter types** over the wrong item type is "`map` needs a function that takes `int`, found `pure (float) -> float`"; the literal is not coerced.
- **A native's error line stays the native's** (review fix). The list-method loops call user code that may store its own `SetLine`, so `may_set_line` treated them as line-free and an error raised by the outer native named a line inside the closure. A loop whose callee isn't `pure` is now treated as running user code, and the outer call evaluates its arguments first. An indirect call still counts as possibly setting a line, even of a `pure` type (conservative: it only causes a spill).
- **`sort` compares with a three-way function,** not a less-than (the design said less-than): it returns before, equal or after, so the key runs exactly twice per comparison. The runtime treats anything above "equal" as "after". `kestrel_rt_sort` sorts an index array with Rust's stable `sort_by` and permutes the items by bytes. Strings need one new runtime function, `kestrel_rt_str_cmp` (UTF-8 byte order is code point order). The design is edited to say this.
- **The scratch arena for a `sort_by` key is decided after lowering** (`strip_idle_scratch`): if the key and what it calls build no memory, the push and pop are removed. A function-value key always keeps it. A native or an indirect call counts as building memory.
- **A `sort_by` key read from a place is refused when it could be freed:** `hs.sort_by(hs[0].key)` is "can't call `hs[0].key` while this call changes `hs`, which may hold its memory". The key's purity is checked for a non-literal key everywhere (not only in `pure` code): "`noisy` isn't `pure`, but `sort_by` needs a pure function".
- **Map loops now count as loops over the lists that hold them** (review fix). `for k, v in hs[0].m { hs.sort_by(...) }` was accepted and gave wrong pairs. A loop over a map that is a place joins the overlapping-loop refusal for every operation that moves or removes items: `sort`, `sort_by`, `remove_at`, `insert`, `pop` and `clear` (not `add`). The wording for the others is "can't change how many items `hs` has while looping over it; change `hs` after the loop". This also refuses a few older programs that did this and printed wrong data or failed at run time. A loop over a map held in a local is unaffected.
- **`sort` of a one-item or empty list, and of arrays, works;** arrays accept `sort` and `sort_by` though not `remove_at` and the rest ("arrays have a fixed length").

**`KESTREL_POISON`** (Task 10, added after Task 2 to guard the memory work)
- With `KESTREL_POISON=1`, a freed arena block is filled with `0xDB` and left allocated: it isn't reused or returned to the allocator, so a read of freed memory shows garbage instead of plausible old data. It is read once at `kestrel_rt_start` (JIT and exe) and is off by default. Poisoned blocks are leaked on purpose, so memory grows with arena churn; the flag is for tests.
- The program-test harness (`hermetic`) sets it, so every program in `tests/programs` runs poisoned in the JIT and as an exe; a program's `// env: KESTREL_POISON=0` overrides it. `KESTREL_ARENA_REPORT` is the same with it on or off.
- Poisoning found nothing in the old suite (zero findings). In the closure work it caught the non-place callee read above, and mutations that were reverted (no deep copy of captures, no `fixup` for function values, no copy-function fixups) were each caught by a poisoned program test.
- It catches reads of freed blocks that change output, not writes into freed memory. The Godot add-on doesn't set it, and the `codegen` tests that call thunks in-process run without it. `par for`'s job-failure path leaks its thread arenas unpoisoned.

**An inliner bug found on the way (ebb47a5).** `g(x: int) -> int { if x > 0 { return 1 } else { 7 } }` called from `main` crashed codegen ("arguments of return must match function signature"). It predates this round (same crash at d66824b). `inlinable` split off a function's final `return` and looked for other returns only in the statements before it, so a `return` inside the final `return`'s own value (a tail `if` or `match`) was copied into the caller. It now counts every `return` in the whole body, nested ones included, and the function is inlinable only if there is exactly one and it is last. Test: `tests/programs/inline_tail_return.kes` and a KIR test.

**Controller rulings** (beyond the user's picks): calling a function-typed constant by name (Task 3); the field and method clash message for function-typed fields only (Task 2); closure parameters follow the no-shadowing rule (Task 4); the frame keeps the loop state with a floor (Task 4); rule 7b narrowed to structs and enums that hold a function value (Task 5); a non-place callee is deep-copied in KIR (Task 6); map loops join the overlapping-loop refusal for every item-moving list operation (Task 8); the poison-mode task added to this round and run right after Task 2.
