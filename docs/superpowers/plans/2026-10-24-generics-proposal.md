# Proposal: generics, your own `[T]` types and functions (2026-10-24)

**Status: approved 2026-10-24** ("review it again make sure its good then thats fine"): the user took picks 1-5 and the default answers to questions A-C, after a second review. Next: the spec, then the plan. Nothing is built yet. The user rated expressiveness 6/10 on 2026-10-24 and agreed the plan: generics first (this proposal), the language server in parallel, then the generics build. It is written in GDScript terms. Each part shows a pick, what was rejected, and what the user has to decide. The syntax is the user's call; the rulings (R1-R8) near the end are Claude's (architecture) and can be overruled. Reviewed twice by Opus subagents against the parser, checker and lowering (first: 2 Critical, 6 Important, 4 Minor; second: 1 Critical, 6 Important, 7 Minor; all folded in below).

## What generics are, briefly

Kestrel already has types that take another type: `list[int]`, `list[Enemy]`, `map[string, int]`, `int?`. They are built into the compiler. Generics let you write your own:

```
struct Stack[T] {
    items: list[T] = []

    push(var self, v: T) { self.items.add(v) }

    pop(var self) -> T? { self.items.pop() }
}

var ints = Stack[int]()
ints.push(3)
var wave = Stack[Enemy]()
wave.push(Enemy(name: "orc", hp: 9, pos: vec2(0, 0)))
```

`T` is a placeholder: a stand-in for whatever type you use it with. In Godot terms, GDScript 4 lets you write `Array[int]` and `Dictionary[String, int]`, but you can't write your own class that takes a type like that; you fall back to an untyped `Array` and lose the type checks. C# can (`class Stack<T>`). Kestrel has no `Variant`-style "any" type to fall back on, so today the only way is to copy the code per type.

**It costs nothing at run time.** The compiler writes a separate copy for each type you use (`Stack[int]` and `Stack[Enemy]` become two ordinary structs with their own methods), so it runs exactly like the code you would have copied by hand.

## The pick

### Generic structs

As above: `struct Name[T] { ... }`, several placeholders separated by commas (`struct Pair[A, B]`). Methods use the struct's placeholders. Using one is `Stack[int]`, in any type position: a local, a parameter, a field, a list (`list[Stack[int]]`), a map value. A generic struct may implement interfaces (`struct Stack[T]: Named`), for every `T`.

### Generic functions

```
pure best[T](xs: list[T], score: pure (T) -> float) -> T? {
    var found: T? = none
    var top = 0.0
    var first = true
    for x in xs {
        let s = score(x)
        if first or s > top {
            found = x
            top = s
            first = false
        }
    }
    found
}

let target = best(foes, (e) { -e.pos.distance_to(me) })   // foes: list[Enemy], so T is Enemy
let tallest = best(heights, (h) { float(h) })               // heights: list[int], so T is int
```

You almost never write the type: the compiler works it out from the arguments. You can write it when you want to (`best[Enemy](foes, ...)`).

### Rules on what `T` can do

Inside generic code the compiler doesn't know what `T` is, so by default a `T` can only be stored, copied, passed and returned. To do more, the placeholder says what it needs:

```
pure total_hp[T: Damageable](xs: list[T]) -> int {
    var sum = 0
    for x in xs {
        sum += x.hp()          // allowed: every T is Damageable
    }
    sum
}

total_hp(goblins)              // list[Goblin]: fine, Goblin is Damageable
total_hp(rocks)                // error: "`Rock` isn't `Damageable`, so it can't be `T` in `total_hp`"
```

This is the fast twin of an interface value. `list[Damageable]` holds any mix of types and each `hp()` is an indirect call through a table (4.28 ns an item in `bench/kernels`, quiet rerun); `total_hp(goblins)` is compiled for `Goblin` alone, so `hp()` is a direct call that can be inlined (0.43 ns an item for the same loop over a `list[Goblin]`). Several rules: `[T: Damageable + Named]`.

Besides interfaces, three **built-in rules** cover what only the compiler's own types can do (question B), each matching what Kestrel accepts today:
- `T: equatable`: `==` and `!=` on `T`, and `has` / `find` on a `list[T]` or `[T]`. Today that is the numbers, `bool`, `string` and the vectors (structs and enums can't be compared with `==` yet).
- `T: ordered`: `<`, `<=`, `>`, `>=` on `T`, `sort()` on a `list[T]` or `[T]`, and `T` as a `sort_by` key. Today: the numbers (floats included), `bool` and `string`.
- `T: key`: `T` as a map key (`map[T, V]`). Today: `int`, `i64`, `u8`, `bool`, `string`.

```
pure count_by[T, K: key](xs: list[T], kind: pure (T) -> K) -> map[K, int] {
    var counts: map[K, int] = map[K, int]()
    for x in xs {
        let k = kind(x)
        let n = counts[k] else {
            counts[k] = 1
            continue
        }
        counts[k] = n + 1
    }
    counts
}

let per_team = count_by(units, (u) { u.team })    // team: string, so K is string
```

### Generic enums

```
enum Result[T, E] {
    Ok { value: T }
    Err { error: E }
}

load_level(name: string) -> Result[Level, string] { ... }

match load_level("cave") {
    Ok r  { play(r.value) }
    Err e { print("can't load: {e.error}") }
}
```

This is what makes error values with reasons possible without building them into the compiler: `Result` can be an ordinary enum in a small standard module.

### Across modules

`pub struct Stack[T]` and `pub best[T]` can be imported and used from other files, like any `pub` item. This is what makes libraries possible, so it is in the first round.

## The picks for the user to confirm

1. **Square brackets: `Stack[T]`, `best[T](...)`, `Stack[int]()`.** They match `list[int]`, `map[K, V]` and `array[int](n)`, so the built-in types and yours look the same. **The catch:** `best[Enemy](foes)` looks exactly like indexing followed by a call (`handlers[i](3)` is valid Kestrel today), and `Result[int, string].Ok(...)` like an index with a comma. Today the parser only knows `list`, `array` and `map` by name. The fix: the parser keeps "brackets after a name" undecided and the checker settles it by what the name is (a generic function or type versus a value). That works because Kestrel never lets a local hide a function or type of the same name. Since type arguments are rarely written (pick 2), it seldom comes up. (Rust needed its `::<>` "turbofish" for this clash.) *Rejected:* angle brackets `Stack<T>` (C#, Rust, C++): familiar, but `<` and `>` are also comparisons, which makes parsing guess (`a < b > c`), and it would look different from `list[int]`.
2. **Type arguments are worked out wherever the values or the expected type decide them; you write them only when nothing does.** `best(foes, ...)` needs nothing; `let s: Stack[int] = Stack()` needs nothing; a bare `Stack()` with nothing to go on is an error that says to write `Stack[int]()`. Plain values decide first, closure literals after (so `best(foes, (e) { ... })` knows `e` is an `Enemy`), and a closure's result can decide too (`make(() { 3 })` gives `int`); a placeholder given an `int` and a `float` becomes `float`, as mixed numbers already do, except through a `var` argument or inside a `list[T]`, which must match exactly; an empty `[]` decides nothing. Each failure has its own message (`` can't tell what `T` is in `best`: write `best[int](...)` ``). *Rejected:* always writing them (`best[Enemy](foes, ...)`): safe but noisy, and GDScript users are used to types being inferred.
3. **What `T` may do comes only from rules written on it: interfaces, plus `equatable` and `key`.** Every requirement is visible in the first line of the generic code. *Rejected:* letting generic code do anything and checking each copy separately (C++ templates, Zig): no rules to write, but a mistake shows up deep inside the generic code, only when someone uses it with the wrong type, and the generic code itself is never checked on its own.
4. **Several rules with `+`: `[T: Damageable + Named]`.** *Rejected:* a comma (`[T: Damageable, Named]` reads as a second placeholder called `Named`) and `and` (it is the logic operator).
5. **Generic enums are in the first round, so `Result[T, E]` and similar can be written.** *Rejected:* structs and functions only: smaller, but leaves error values with reasons blocked.

## Questions for the user

- **A. Maths on `T`.** Should there be a built-in `number` rule, so `pure sum[T: number](xs: list[T]) -> T` works for `int`, `float` and the vectors? The pick is **no, not in the first round**: a key function (`score: pure (T) -> float`) covers most game uses. Say if you want it.
- **B. The built-in rules.** `equatable`, `ordered` and `key` (above; the second review split the first draft's two rules into three, because `sort()` takes floats and map keys don't). They are words only after `:` in a placeholder list, so `key` stays usable as a variable name. Alternatives: `comparable` / `hashable` (Swift's words), or no built-in rules at all (no `==`, `<`, map keys or `sort()` on `T` in the first round).
- **C. Placeholder names.** `T`, `E`, `K`, `V` by convention, but any capitalised name works (`[Item]`, `[Key, Value]`). A placeholder can't have the name of a type that already exists (Kestrel never lets one name hide another): `struct Bag[Item]` is an error when a struct `Item` exists. Fine as is?

## Rulings (architecture; the user can overrule any)

- **R1. Generic code is checked once, with `T` unknown, and copied per type when compiled.** Type errors in generic code show up where it's written, whether or not anyone uses it yet; a type that breaks a rule is an error where it is used. This fits the checker, which keeps one type per expression; checking every copy separately would mean keeping one per copy.
- **R2. Every copy is collected before lowering.** At the end of checking, one pass walks from the non-generic code and finds every copy the program uses (each `Stack[int]`, `best[Enemy]`, and the copies those make), gives each its own struct, enum, function and signature ids, and stops with an error on copies that would never end (`f[T]` calling `f[list[T]]`, or a type like `Tree[T] { kids: list[Tree[list[T]]] }`). The walk starts from every non-generic function and also from constants, globals, field defaults and `@assume`s. The review found two things that need this: lowering assumes each function is lowered once under the checker's id, and whether a struct "owns" memory is decided once, before function bodies are checked, so a copy first seen inside a body would otherwise be treated as plain (a double-free risk).
- **R3. Which types can be `T` in the first round:** any type except a type that owns an `Arena` or a GPU object ("`Sprite` owns a GPU object, so it can't be `T` yet"), an optional (`int?`; `Stack[int?]` would make `int??`), a function type, and a struct or enum that holds a function (the closure rules refuse capturing those, and `x == none` on an optional of them). Interface types are allowed (`Stack[Damageable]`), and an interface satisfies its own rule (`Damageable` can be `T` in `total_hp[T: Damageable]`; that copy makes indirect calls). Owning types need generic code checked as if every `T` might move (as Rust does), a second set of rules. *Cost:* a generic pool of textures, a stack of optionals, a generic queue of callbacks, and a stack of structs with a callback field wait for a later round.
- **R4. Inside generic code `T`, and any type containing it, gets the strictest answer to every question the checker asks of a type** (about fifteen places: passed by pointer, holds heap memory (so a copy may allocate, and arena escape rules apply as for a `string`), holds an interface, breaks a zero fill), and is never owning. Since R3 keeps functions out of `T`, "may hold a function" is answered no. Lowering follows the checker's recorded decisions (views, moves, conversions) in every copy, so those decisions must be right for any `T`. The second review found this was the one place generic code could become unsafe: a permissive answer would let a `string` copy outlive its arena. Consequences: `array[T](n)` and `array[Box[T]](n)` (zero-filled) aren't allowed in generic code, use a `list[T]`. A `T: Damageable` value can be passed where a `Damageable` is expected. In `pure` code and `par for`, calls through an interface rule follow the interface rule (only `pure` methods), which is stricter than the same code written for one type.
- **R5. Contracts and the report work per copy.** This is the one place an error can appear far from where the generic was written (a library's copy breaks a contract written in the program). `@prove` and `kestrel check --report` run on compiled code, so they judge each copy: `@prove(no_alloc)` may hold for `Stack[int]` and fail for `Stack[string]` (copying a string allocates). The error names the copy and the line that made it (`` `Stack[string].push` allocates: copying a `string` (made by `Stack[string]` on line 12) ``). A generic with no uses has no copies, so its contract has nothing to check yet; the report says so ("`best`: not checked, no copies").
- **R6. One copy per generic and set of types in the whole program,** whichever modules use it; `print` and `"{x}"` show a copy's type as `Stack[int]`.
- **R7. Rules on a struct's placeholders are checked wherever the struct is named** (`Grid[X]`, and `Grid[T]` inside other generic code, where `T` must carry the same rule).
- **R8. Methods use their struct's placeholders only;** a method with placeholders of its own (`map[U]` on a `Stack[T]`) waits; generic functions cover that case. Interfaces don't take placeholders yet (`interface Container[T]`), and a generic function is a function value only once its types are fixed (`let f = best[Enemy]` works, `let f = best` doesn't).

## What it would buy, honestly

- **Reusable code and, for the first time, libraries:** a pathfinding module over any node type, an object pool, a spatial grid, a priority queue, `Result`. This is the main gain; nothing is faster at run time than the hand-copied version.
- **A fast alternative to interface values** where all items have one type: a direct call instead of an indirect one.
- **Costs:** compile time and executable size grow with each type a generic is used with (usually small). The checker and lowering grow by a fair amount (placeholders, rules, working out types, the collection pass, and lowering each function once per copy: lowering reads the checker's types and decisions in about 48 places across 16 files), and error messages need care so they stay plain sentences. **Expected size: 10-12 tasks**, the largest round so far. The first estimate (about five) and the second (7-9) were too low.

## Not in this round

The `number` rule (question A, unless the user wants it); owning types, optionals and function types as `T` (R3); placeholders on methods and interfaces (R8); default types (`[T = int]`); constant placeholders (`Grid[W, H]` with numbers); `is` type tests; specialising a generic for one type; Godot exports of generics (a `pub` generic has no concrete type for GDScript to call; a non-generic wrapper is the way).

## For the spec to settle

- **How a generic type is represented while checking** (`Stack[T]` and `Stack[int]` as `Ty`s) so that every type question (`has_heap`, owning, zero fill, names, map types) sees through it; types that contain themselves through a generic (`struct Bad { w: Wrap[Bad] }` is an error, as a struct containing itself is today; `enum L[T] { Cons { tail: L[T]? } }` likewise); interface tables per copy and interface, including ones that appear only inside copies. Built in task 2, not later.
- **Brackets after a field access** (`geom.best[Enemy](foes)`, `geom.Stack[int]()`, `self.handlers[i](3)`), and bracket contents that are types but not expressions (`[int]`, `(int) -> int`): parsed both ways, settled by what the name is, across modules too.
- `match` on a generic enum inside generic code, field defaults and labelled construction of generic structs, `T` in closures and `par for`, and the exact messages for each rule.

## How it would be built (sketch, for the plan)

1. **Parser:** placeholders and rules on `struct`, `enum` and functions; type arguments in type positions; a "brackets after a name" node whose contents are parsed both as types and as an index, for the checker to settle (pick 1).
2. **Checker, declarations:** a placeholder type (`Ty::Param`) with its rules; `Stack[int]` in a type position.
3. **Checker, bodies:** generic bodies checked once against their rules (method calls on `T` through its interfaces, `equatable`/`key` operations, the cautious rules of R4); settling brackets after a name; working out type arguments (values first, closures after).
4. **The collection pass (R2):** every copy found to a fixed point and given its own ids, owning-ness taken from the generic definition, a limit on copies that never end.
5. **Lowering (KIR):** lowering every function once per copy, with one place that answers "the type of this expression" and "what this call resolves to" under the copy's substitution (today many places use the checker's ids directly); calls on `T` become direct calls in each copy. Range analysis, the inliner, contracts and codegen then see ordinary functions.
6. **Names and contracts:** `Stack[int].push` in errors, the report and `@prove`, with the line that made the copy.
7. **Across modules:** importing `pub` generics; copies made in the importing program.
8. **Docs and tests:** the syntax spec's "Generics" section, the README, program and error tests for each rule, and the final review.
