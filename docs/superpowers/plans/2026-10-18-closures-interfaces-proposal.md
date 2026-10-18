# Proposal (2026-10-18): closures, interfaces, list methods, `@`

Presented to the user on 2026-10-18; waiting for their picks. Syntax is the user's call. Everything under "Rulings" is Claude's, recorded here and to be copied into the spec with its cost. After the picks: spec (plus the `@` spec), Opus review, plan, subagent-driven build on `main`.

## 1. Closures

**Pick:** a closure is a function with its name cut off. GDScript's `func(e): ...` is `func name(e): ...` without the name; Kestrel functions have no keyword, so:

```
square(x: int) -> int { x * x }                    // named function
(x: int) -> int { x * x }                          // the same, as a closure
let alive = enemies.filter((e) { e.hp > 0 })       // e's type comes from context
enemies.sort_by((e) { e.pos.distance_to(player) })
let hurt = (amount: int) { print("ouch {amount}") }
hurt(3)
```

- Parameter types may be left out when the context gives them (a list method, a typed parameter, a typed `let`); otherwise they are required, as for functions. The return type is inferred.
- Function types: `(Enemy) -> bool`, `(int)` (takes an int, gives nothing), `()`, `pure (vec2) -> float`.
- Calling: `f(x)` and `self.on_hit(3)`, not GDScript's `.call()`.
- Named functions are values too: `xs.sort_by(score)`, `let f = score`. Methods are not (v1).

**Rejected:**
- `e => e.hp > 0` (JS/C#): shorter, but a second way to spell a function.
- `|e| e.hp > 0` (Rust): Rust-identical; `|` is bit-or and `||` is already an error.
- `e -> e.hp > 0`: `->` already means "returns".
- An implicit `it` (`enemies.filter(it.hp > 0)`): shortest, but a magic name, one parameter only, and it hides where the closure starts.
- A `fn`/`func` keyword: Kestrel declares functions without one.

**Rulings:**
- Captures are by value, as in GDScript. A closure can't change a captured variable (the error says to return a value or use a `for` loop), which closes GDScript's silent "changed the copy" gotcha.
- A closure passed straight to a call reads captured variables in place (nothing is copied; it looks the same because they can't change during the call). It is copied only when stored (`let`, field, list, `return`).
- A stored closure is a heap value like a string: built in the current arena, `in x.mem { }` to store it somewhere longer-lived. Its environment carries its own copy function.
- A closure can't capture an owning value (`Arena`, GPU texture or mesh) in v1.
- `return` inside a closure returns from the closure; `break`/`continue` can't cross it.
- A closure's purity is inferred from its body. A `pure` function or a `par for` body may call a function value only when its type is `pure`; a literal closure is checked where it is written.
- No `==` on function values, no function-valued map keys; a `pub` function with a function type in its signature compiles but isn't runnable from Godot.
- Calls through a function value are positional (function types have no parameter names).
- Parsing: `(...)` followed by `{` or `->` is a closure, except in an `if`/`while`/`for`/`match` head, where it stays a condition and its block (Rust's struct-literal rule). A closure in a head goes inside a call's parentheses anyway.
- Implementation: a function value is `{code, env}` (16 bytes) called with Cranelift `call_indirect`, the shape `par for` already uses. A literal closure passed to a list method becomes a KIR function called directly from a helper generated for that call site, so the inliner removes the call.

## 2. Interfaces

**Pick:**

```
interface Damageable {
    take_damage(var self, amount: int)
    pure hp(self) -> int
}

struct Goblin: Damageable {
    hp: int = 10
    take_damage(var self, amount: int) { self.hp -= amount }
    pure hp(self) -> int { self.hp }
}

var foes: list[Damageable] = []
foes.add(Goblin())
foes.add(Slime(size: 3))
for f in foes { f.take_damage(2) }
```

- `struct Goblin: Damageable, Drawable` reads like `x: T`: "Goblin is a Damageable". Enums too (`enum Barrel: Damageable`). Every method must match exactly (`pure`, `var self`, types).
- Declared only on the struct itself (another module can't add one later).
- Methods only: no fields, no default bodies. "No inheritance" stays true.

**Rejected:**
- Structural, Go-style (no `: Damageable`; anything with the methods fits): less typing, but a misspelt method silently stops matching and the error shows up far away.
- `struct Goblin is Damageable` / `implements`: a new keyword, and `is` is kept for type tests (open question).
- `trait` with `impl Damageable for Goblin { }` blocks: Rust-identical, and it splits a struct's methods.
- Generic templates (`total[T: Damageable](xs: list[T])`): a copy per type, per-instance checker tables, and goblins and slimes can't share one list, the main reason to want interfaces.

**Rulings:**
- Dynamic dispatch: an interface value is data plus a method table. Passing a `Goblin` to `d: Damageable` costs a pointer and a table (no copy). Storing one (list, field, `let`) copies it into the current arena, like a string.
- When the concrete type is known after inlining, the call becomes a direct call.
- `print(f)` prints the concrete value. No `==` on interface values.
- A struct that owns an `Arena` or GPU object may implement an interface and be passed as one, but can't be stored as one (v1).
- The method table carries a type id, so a type test can be added later cheaply.

## 3. List methods

**Pick** (GDScript's names where it has them):

```
let alive = enemies.filter((e) { e.hp > 0 })         // new list
let spots = enemies.map((e) { e.pos })               // list[vec2]
if enemies.any((e) { e.boss }) { alarm() }
if enemies.all((e) { e.hp <= 0 }) { win() }
scores.sort()                                        // numbers and strings
enemies.sort_by((e) { e.pos.distance_to(player) })   // in place, by key
```

**Rejected:**
- GDScript's `sort_custom((a, b) { a.hp < b.hp })` comparator: a key can't be written inconsistently (a bad comparator scrambles a sort silently) and is shorter. Could come later.
- `keep_if` in place: `remove b` inside a `for` already removes in place.
- `reduce`: a `for` loop reads clearer.

**Rulings:**
- `filter` and `map` return a new list in the current arena, on lists and arrays. They refuse owning items (they would copy them).
- `sort` and `sort_by` work in place on a `var` list or array, stable, smallest first (negate a number key for largest first). Keys are numbers, strings or bools.
- The `sort_by` key must be `pure` (it runs many times). `filter`/`map`/`any`/`all` closures may print and run once per item, in order; `any`/`all` stop early.
- `sort`/`sort_by` are refused inside a loop over the same list, and a key closure can't capture the list it sorts.
- Literal closures are specialised at the call site and inlined: no call overhead.

## 4. `@` annotations (spec now, build after range analysis)

**Pick:** GDScript-style `@word(args)` on the line before, like `@export_range(0, 100)`:

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

- `@prove(...)` fails the build when Kestrel's own KIR analysis can't show it, with the call chain in the error. Contracts: `no_alloc`, `no_checks`, `no_panic`, `inline`.
- `@assume(cond)`: checked when checks are on, trusted under `--release`, a warning when it removes nothing.
- `kestrel check --report` comes first: every remaining check or allocation with its reason.
- An interface method's `@prove` binds every implementation (checked there), so calls through the interface keep it. A call through a plain function value counts as "may do anything" unless the closure is known at the call site (the list methods). Contracts on function types: later.

**Rejected:**
- Flat `@no_checks`: reads like a switch that turns checks off (what `--release` does), the opposite of a proof.
- `@optimize`, `@prefer(speed)`: vague, nothing to check.
- Keywords (`prove no_alloc step(...)`): `@` keeps proofs apart from structure (`pure`, `pub`).

## Open question

Type tests on interface values, GDScript's `is`: `if f is Goblin g { g.enrage() }`, and in `match` (`match f { Goblin g { ... } else { ... } }`). Claude's lean: later. Enums already give closed sets with `match`, and the method table carries a type id, so adding it later is cheap.

## Build order (after the picks)

Function types and named functions as values, then closures and captures, then the list methods, then interfaces, then devirtualising. Several checker files are near 400 lines (`tables.rs`, `collect.rs`, `body/mod.rs`, `body/call.rs`) and get split first.
