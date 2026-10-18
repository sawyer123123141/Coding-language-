# Kestrel Syntax Redesign

Date: 2026-10-09. Status: approved.

Sub-project 1 of 5 in the Kestrel redesign. Defines what Kestrel source
looks like. Semantics beyond what syntax needs (type checker, IR,
codegen) belong to sub-project 2.

## Context

Decisions from the redesign brainstorm that shape this spec:

- **Target:** games, especially voxel/simulation games with heavy
  number crunching. The language thesis: purity proofs give safe,
  automatic parallelism (`par for`) and SIMD.
- **Memory model:** plain values + arenas. No garbage collector, no
  borrow checker.
- **No classes / no inheritance.** Structs and enums with methods,
  `match` for per-kind behaviour. Interfaces may come later.
- **One implementation.** `kestrel.js` and the separate
  `jit_codegen.rs` are deleted in sub-project 2; one Rust compiler with
  one Cranelift codegen serves JIT (dev) and AOT (exe).
- **Feel:** distinct from Rust; readable by someone coming from
  GDScript.

Sub-project order: 1 syntax (this) -> 2 core rebuild -> 3 parallelism
v2 (chunked `par for` + SIMD) -> 4 game shell (C FFI, window, input)
-> 5 release speed (LLVM backend, precompiled runtime).

## Example

```
use { noise2 from math }

let SIZE = 32

enum Block {
    Air
    Dirt
    Stone { hardness: u8 }

    pure is_solid(self) -> bool {
        match self {
            Air  { false }
            else { true }
        }
    }
}

struct Chunk {
    x: int
    z: int
    mem: Arena = Arena()
    heights: [int] = []

    generate(var self, seed: int) {
        in self.mem {
            self.heights = array[int](SIZE * SIZE)
        }
        for i from 0 to SIZE * SIZE {
            let wx = self.x * SIZE + i % SIZE
            let wz = self.z * SIZE + i / SIZE
            self.heights[i] = height(wx, wz, seed)
        }
    }
}

pure height(x: int, z: int, seed: int) -> int {
    let n = noise2(vec2(x, z) * 0.01, seed)
    if n > 0.5 { int(n * 64) } else { int(n * 32) }
}

pure block_at(y: int, ground: int) -> Block {
    if y > ground          { Block.Air }
    else if y > ground - 3 { Block.Dirt }
    else                   { Block.Stone(hardness: 3) }
}

main() {
    var world = list[Chunk]()
    for cx from 0 to 16 {
        for cz from 0 to 16 {
            world.add(Chunk(x: cx, z: cz))
        }
    }

    par for chunk in world {
        chunk.generate(seed: 1337)
    }

    arena {
        var tris = list[vec3]()
        for chunk in world { mesh(chunk, var tris) }
        print("triangles: {tris.len / 3}")
    }
}
```

## Lexical rules

- **Comments:** `//` to end of line. No block comments.
- **Statement ends:** a newline ends a statement. No semicolons
  required; `;` is allowed as a separator between statements on one
  line.
- **Line continuation:** a newline does *not* end a statement when
  inside an unclosed `(` or `[`, when the line ends with a binary
  operator or `,`, or when the next line starts with `else` or `.`.
- **Strings:** `"..."`, interpolation with `{expr}`, escapes `\n \t \"
  \\ \{ \}`.
- **Numbers:** `42`, `3.14`, `1_000_000`, `0xFF`. An integer literal is
  `int` unless context needs another integer type; a literal with `.`
  is `float` unless context needs `f64`.

## Declarations

### Functions

```
[pub] [pure] name(params) [-> Type] { body }
```

- No `fn` keyword. Only declarations appear at top level and in
  struct/enum bodies, so a bare `name(` there is unambiguous.
- `pure` marks a function the compiler verifies has no effects (no I/O,
  no writes to anything but its own locals and its `var` parameters,
  calls only pure functions). `var` parameters, including `var self`,
  are allowed (decided 2026-10-09), so `par for` bodies can call
  `pure build(var self)` on their item.
- Parameters: `name: Type`. **Read-only by default.**
- `var name: Type` parameter: the function may change the caller's
  value, and the change is visible to the caller (by reference). A
  `var` parameter has no other meaning — there are no "mutable local
  copy" parameters.
- **Call sites mark `var` arguments:** `mesh(chunk, var tris)`, so a
  reader sees what a call changes. Passing a `var` argument without the
  marker (or marking a read-only one) is a compile error. The receiver
  of a method call is exempt: `chunk.generate()`, not
  `var chunk.generate()`.
- Program entry point: `main()`.

### Native functions

```
[pub] [pure] native name(params) [-> Type]
```

- `native` declares a function whose body the runtime supplies, so it has no block (`pub native circle(center: vec2, radius: float, color: Color)`). It is a keyword, and it comes after `pub` and `pure`: `pub pure native`.
- Only Kestrel's built-in modules can declare natives (decided 2026-10-10). A native in a user file is an error: "only Kestrel's built-in modules can declare native functions". A native can't be a method.
- A native's parameters and result are limited to what Rust can read from memory. Details: `2026-10-10-window-design.md` ("Built-in modules and `native`").
- A native that isn't `pure` has effects, like `print`, so `pure` functions and `par for` bodies can't call it.

### Built-in modules

`window`, `draw` and `input` are modules that ship inside the compiler. `use draw` loads the embedded file, and a `draw.kes` next to the program is an error. They are ordinary Kestrel (structs, enums, constants, functions) plus `native` functions, so `use { Color, RED from draw }` and `draw.circle(...)` work like any module. They open a window, draw 2D shapes and text, and read the keyboard and mouse. They are only available in the standalone `kestrel` command, not in the Godot add-on. The full API and the design are in `2026-10-10-window-design.md`.

### Methods

Declared inside a `struct` or `enum` body. First parameter is `self`
(read-only) or `var self` (changes the original). Called with dot
syntax: `chunk.generate(seed: 1337)`.

### Structs

```
[pub] struct Name {
    field: Type [= default]
    ...
    methods...
}
```

Fields one per line, no commas. A field with a default may be omitted
at construction.

### Enums

```
[pub] enum Name {
    Variant
    Variant { field: Type, ... }
    ...
    methods...
}
```

Variants without data, or with named fields. Construct with the type
prefix: `Block.Air`, `Block.Stone(hardness: 3)`.

### Variables and constants

- `let x = value` — cannot change after creation.
- `var x = value` — can change.
- Optional type: `let x: float = 1`.
- **Block scoped:** visible from declaration to the end of the
  enclosing `{ }`.
- **No shadowing:** declaring a name already visible in an enclosing
  scope is an error. Visible means in the same file: the file's own items,
  its imports and the enclosing locals. Another file's items aren't
  visible, so a local may share their names ("`r` is already defined in
  this file; Kestrel doesn't allow shadowing" is only for a clash in one file).
- Top-level `let` is a module constant. Top-level `var` is a global;
  touching it is an effect, so `pure` functions can't.

### Imports and visibility

```
use geom                        // then geom.dist2(a, b), geom.Shape.Box(w: 2, h: 3)
use { Point, dist2 from geom }  // then Point(1, 2), dist2(a, b)
use {
    noise2, lerp from math
    colors
}
```

- **Modules.** A module is one file, named after it, found next to the file that uses it. A file is loaded once; files may use each other.
- **`pub`** marks a function, struct, enum or constant as importable. Anything else stays private to its file.
- **A module sees only its own items and its imports.** The program's own file is no exception: a module that uses `bump` from it without importing it gets "unknown function `bump`: it's in main.kes, which `util` can't see".
- **Names must be unique.** An imported name can't collide with the file's own items, another import, or a local variable.
- **Printing** shows a type's own name (`Point`), not `geom.Point`.
- **Types from another module** must be imported by name; `geom.Point` doesn't work in a type position yet.
- **Runtime errors** name the line and the file it is in: "index 5 out of bounds for length 2 on line 24 of geom.kes" (implemented 2026-10-12).

(Implemented 2026-10-10.)

## Types

| Spelling | Meaning |
|---|---|
| `int` | 32-bit signed integer |
| `float` | 32-bit float |
| `bool` | `true` / `false` |
| `i8 i16 i64 u8 u16 u32 u64` | sized integers |
| `f64` | 64-bit float |
| `string` | text, UTF-8, a heap value like an array: copying a string copies its bytes into the current arena (literals stay in read-only data and belong to no arena). `s.len` and `s[i]` count characters, not bytes. `+`, `join`, `split`, `slice`, `find`, `upper`, `trim`, `to_int` and the rest: see "Lists and strings" |
| `vec2 vec3 vec4` | float vectors, SIMD-backed |
| `ivec2 ivec3 ivec4` | int vectors (voxel coordinates) |
| `[T]` | array of `T`, fixed length once created (fast, arena/SIMD friendly) |
| `list[T]` | growable list: `add`, `remove_at`, `pop`, `insert`, `clear`, `has`, `find`, `join`, `.len`; `remove b` in a `for b in xs` loop. See "Lists and strings" |
| `map[K, V]` | hash map: `m[k] = v`; `m[k]` is a `V?` (so `let v = m[k] else { ... }`); `m.has(k)`, `m.remove(k)`, `m.len`, `for k, v in m` (decided 2026-10-10). Literal `{"a": 1, "b": 2}`, empty `{}`; `m.get(k, default)`; `for k in m` gives keys (decided 2026-10-09). Insertion order. Details: `2026-10-09-map-design.md` |
| `(A, B) -> R` | a function value: `(A, B)` gives nothing, `pure (A) -> R` is pure, `var A` marks a parameter changed in place. See "Closures and function values" |
| `T?` | a `T` or `none` |
| `Arena` | a memory bucket |

- **Type parameters use square brackets:** `list[vec3]`, `map[string,
  int]`.
- **Creating generic values:** `list[vec3]()`, `array[int](n)`, or
  annotate and use a literal: `var tris: list[vec3] = []`.
- **Vectors:** `vec3(1, 2, 3)`; `vec2()` and `vec3()` with no arguments are zero vectors (like Godot's `Vector3()`), and giving only some components is an error; operators `+ - * /` component-wise and
  with scalars, swizzles `v.xz`, `v.zyx`.
- **Conversions:** integer -> float and narrower -> wider integer are
  implicit. Float -> integer and wider -> narrower require an explicit
  call: `int(x)`, `u8(n)`.
- **Overflow:** debug builds trap on integer overflow; release builds
  wrap.

## Expressions

- **Operators**, highest to lowest precedence: unary `-`, `not` and
  `~`; `* / %`; `+ -`; `<< >>`; `&`; `^`; `|`; `== != < > <= >=`;
  `and`; `or`. Comparisons don't chain. Logic is words only: `&&`,
  `||` and `!` don't exist (`!=` stays). Bit operators are symbols, as
  in GDScript, and work on integers (decided 2026-10-10): `& | ^` and
  `~x`, shifts `<<` and `>>` (arithmetic for signed types). A shift by
  a negative amount or by the type's width or more panics when checks
  are on. Compound forms: `&= |= ^= <<= >>=`.
- **Calls:** `f(a, b)`. Any argument may be labelled with its
  parameter name: `f(a, seed: 1337)`. Labelled arguments come after
  positional ones.
- **Construction:** `Chunk(x: 1, z: 2)` or positional in field order
  `vec3(1, 2, 3)`. Labels are required once any field is skipped.
- **Blocks are expressions:** the last expression in a block is its
  value. `return` is only for early exit.
- **`if` is an expression:** `let n = if a { 1 } else { 2 }`.
- **Indexing:** `a[i]`. **Field access:** `p.x`.
- **Array literal:** `[1, 2, 3]`. Empty: `[]`.
- **Interpolation:** `"hp = {hp}"` works anywhere, not only in `print`
  (`let label = "hp = {hp}"`). It formats as `print` does, and the string
  is built in the current arena. Because strings are heap values, storing
  a non-literal string into a longer-lived place needs `in x.mem { }`, the
  same rule arrays have.

## Built-in functions

Built into the compiler, with no import, like `print` and `int()`. They
can take several number types, which user functions can't. Their names
are reserved: a function, type, constant, variable or parameter with one of
them is an error ("`min` is a built-in name; pick another").

- **Math:** `sqrt sin cos tan asin acos atan atan2 exp log pow floor ceil
  round abs sign min max clamp lerp`, and the constants `PI` and `TAU`.
  All are `pure`.
- **Vector methods** (`vec2`, `vec3`): `v.length()`, `v.normalized()`,
  `a.dot(b)`, `a.distance_to(b)`. These names are not reserved.
- **Random numbers:** `randf()`, `randf_range(lo, hi)`,
  `randi_range(lo, hi)`, `rand_seed(n)`. Not `pure`: each call changes the
  generator.

The full table of argument and result types is in
`2026-10-10-window-design.md` ("Global math and random numbers").

## Statements

- Assignment: `x = v`, `p.x = v`, `a[i] = v`, compound `+= -= *= /=
  %=`. Target must be a `var`, a `var` parameter, or reached through
  `var self`.
- `print(...)` is an ordinary (effectful) function, usable anywhere.
- **No-effect statements are errors.** An expression statement whose
  value is unused and that has no effect (`+bonus`, `x == 1`) is a
  compile error: "this line has no effect; did you mean to continue
  the line above?" This closes the line-continuation trap where a line
  starting with an operator silently becomes its own statement.

## Control flow

```
if cond { ... } else if cond { ... } else { ... }
while cond { ... }
for i from 0 to n { ... }      // n exclusive, step +1
for v in xs { ... }            // each element
for i, v in xs { ... }         // index and element
par for v in xs { ... }        // parallel, see below
break
continue
return [value]
```

No parentheses around conditions.

### `match`

```
match value {
    Variant          { ... }
    Variant binding  { ... }   // binding names the variant's fields: binding.hp
    else             { ... }   // catch-all
}
```

- `match` is an expression.
- Every variant must be handled or `else` present — adding a variant
  makes the compiler list every `match` that misses it.
- Inside `match`, variants are written bare (`Air`), since the type is
  known.
- A binding refers to the payload in place; it is changeable when the
  matched value is (`var self`, a `var`).

### `par for`

```
par for chunk in world {
    chunk.generate(seed: 1337)
}
```

Runs iterations in parallel across CPU cores. The body may only:

- call `pure` functions,
- call `var self` methods on the loop's own item, when those methods
  change nothing but that item,
- change the loop's own item and its own locals.

Anything else is a compile error naming the offending line, e.g.
"line 3 writes to `world.score`, which is shared." Work splitting
(chunking) and SIMD are sub-project 3. Details and the remaining
rules: `2026-10-09-par-for-design.md`.

## Lists and strings

Design and rulings: `2026-10-17-lists-strings-design.md`. Names follow GDScript where a GDScript name fits.

**List methods.** `has` and `find` also work on arrays; the rest need a `list[T]` held in a `var`.

| Kestrel | Meaning |
|---|---|
| `xs.add(v)` | add at the end |
| `xs.remove_at(i)` | remove item `i`; the rest keep their order |
| `xs.pop()` | remove the last item and return it as a `T?` (`none` when the list is empty); GDScript's `pop_back` |
| `xs.insert(i, v)` | put `v` at `i`, shifting the rest up; `i == xs.len` adds at the end |
| `xs.clear()` | remove every item |
| `xs.has(v)` | `true` when an item `== v` (numbers, bools and strings) |
| `xs.find(v)` | the index of the first item `== v`, as an `int?` (GDScript returns -1) |
| `parts.join(sep)` | one string from a list or array of strings |

`filter`, `map`, `any`, `all`, `sort` and `sort_by` are under "Closures and function values".

An index outside `0..xs.len` panics. `remove_at`, `pop`, `insert` and `clear` are refused inside a `for` loop over the same list, because the loop read the length once. `add` is allowed there; the loop doesn't see the new items.

**`remove b`** removes the current item of a `for b in xs` loop over a list. The loop compacts the list in one pass, so items keep their order and none is skipped (in GDScript, `xs.erase(b)` inside `for b in xs` skips the next item).

```
for b in bricks {
    if hits(ball, b) {
        remove b
    }
}
```

- `remove` is not a keyword: `remove` followed by a name, as a statement of its own (`if hit { remove b; continue }` too), is this statement.
- `b` is gone after `remove b`, so using it later is an error. The rest of the body still runs, so follow it with `continue` to skip that.
- While a loop that uses `remove` runs, its body can't use the list except through `b` (not `xs`, `xs.len` or `xs[j]`), because the items are moving. Other fields of the same variable are fine.
- Not allowed in `par for`, in a loop over an array, map, string or a temporary, or on a list declared with `let`.

**Strings** are UTF-8. Lengths and indexes count characters (code points, as in GDScript), not bytes: `"héllo".len` is 5.

| Kestrel | Result |
|---|---|
| `s.len` | `int`, the number of characters |
| `s[i]` | the character at `i`, as a one-character `string` |
| `for c in s`, `for i, c in s` | each character as a `string`, and its index |
| `a + b`, `s += t` | a new string (both sides must be strings; use `"{n}"` for numbers) |
| `s.split(sep)` | `list[string]`; `"".split(",")` is `[""]`; `sep` can't be empty |
| `s.has(t)`, `s.starts_with(t)`, `s.ends_with(t)` | `bool` |
| `s.find(t)` | the character index of the first match, as an `int?` |
| `s.slice(from, to)` | the characters from `from` up to, not including, `to` (GDScript's `substr` takes a length instead) |
| `s.upper()`, `s.lower()`, `s.trim()` | a new string (`trim` is `strip_edges`) |
| `s.to_int()`, `s.to_float()` | `int?` / `float?`, `none` when the text isn't a number (GDScript returns 0) |

- Strings can't be changed in place: `s[i] = x` is an error. Build a new string.
- Every string these produce is new and lives in the current arena, like interpolation. Storing one in a longer-lived place needs `in x.mem { }`. In a frame loop, wrap the body in `arena { }` to free them each frame.
- `.len`, `s[i]`, `slice` and `find` walk the string from its start, so `s[i]` inside a loop over `i` is slow on long text. `for c in s` is the linear way.
- A bad `s[i]` or `slice` panics with the usual line and file.

```
var words: list[string] = []
for part in "go, stop ,  jump".split(",") {
    words.add(part.trim().upper())
}
print(words.join("-"))      // GO-STOP-JUMP
```

## Closures and function values

Design and rulings: `2026-10-18-closures-design.md` (its "As built" section lists what differs). A closure is a function with its name cut off, as GDScript's `func(e): ...` is `func name(e): ...` without the name.

```
square(x: int) -> int { x * x }                    // named function
let sq = (x: int) -> int { x * x }                 // the same, as a closure
let alive = enemies.filter((e) { e.hp > 0 })       // e's type comes from context
let hurt = (amount: int) { print("ouch {amount}") }
hurt(3)
```

**Literal.** `(params) [-> Type] { body }` is an expression. A parameter is `name: Type` or `var name: Type`; the type can be left out where a function type is expected (a list method's argument, a function-typed parameter, an annotated `let`, a field, a default, a `return`). The body is a block whose last expression is the result, and `return v` leaves the closure. The result type is inferred from the body unless written with `->`. A closure is `pure` when its body is. A closure parameter can't share a name with a local that is already visible, like any other local.

**Parentheses.** In expression position, a `(` whose `)` is followed by `{` or `->` starts a closure. In the head of an `if`, `while`, `for`, `match` or `in`, `(cond) {` stays a parenthesised condition, so `if (ready) { go() }` still works. A closure in a head goes inside a call's parentheses: `if xs.any((e) { e.dead }) { ... }`.

**Function types.** `(A, B) -> R`, `(A, B)` (gives nothing), `()`, `pure (A) -> R`, and `var A` for a parameter changed in place. They work in every type position: `list[(int) -> int]`, fields, parameters, results. `T?` of a function type is refused; give a callback a do-nothing default instead (`on_hit: (int) = (n) { }`). A function value of a `pure` type goes where a non-pure type is expected, not the other way round.

**Named functions are values:** `let f = square`, `xs.sort_by(score)`, `util.score`. Methods, built-in functions (`min`, `sqrt`, `print`) and natives (`draw.rect`) are not; wrap them in a closure. Function values can't be compared with `==` or used as map keys, and `print` writes `<function>`.

**Calling.** `f(x)` for a local, parameter or constant of function type, `self.on_hit(3)` for a field (a struct can't have a function field and a method with the same name), and `handlers[i](3)` or `make_adder(2)(5)` for any other expression. Arguments are positional (no labels), and a `var` parameter is marked as usual: `f(var p, 0.5)`.

```
struct Button {
    label: string
    on_click: (int) = (n) { }

    press(self) { self.on_click(1) }
}

make_adder(k: int) -> (int) -> int {
    (x) { x + k }
}

main() {
    let add2 = make_adder(2)
    print(add2(5))              // 7
    let b = Button(label: "ok", on_click: (n) { print("clicked {n}") })
    b.press()
}
```

**Captures.** A closure captures the locals and parameters it names, **by value**, as GDScript does: it sees what they held when it was made. What is captured is a path (`self.name`, `w.score`), not the whole variable. A closure can't change what it captured (assigning, `+=`, `var` arguments, changing methods and moves are errors; use a `for` loop to accumulate), and it can't capture a value that owns an `Arena` or a GPU object. A closure that may be stored can't capture a whole struct that holds a function value (`self.cb = () { self.fire() }`, or `make_timer(3, () { self.fire() })`, since a function may store its argument); copy what it needs into a local first. A literal passed to `filter`, `map`, `any`, `all` or `sort_by` is never stored, so it may. A function value, or a list or map of them, can be captured. `break`, `continue` and `remove` can't cross a closure's boundary.

**Memory.** A stored closure with captures lives in the arena that was current when it was made, like a string, so storing it somewhere longer-lived needs `in x.mem { }`. A closure without captures, and a named function, allocate nothing and can go anywhere. A closure literal written directly as an argument to a call of a user function, a method or a function value allocates nothing either, so the call must not change anything the closure reads: "`xs` is changed by this call, so the closure passed to it can't read it".

**`pure` and `par for`.** A `pure` function or a `par for` body may call a function value only if its type is `pure`, and a closure literal written there must be pure.

### List methods that take functions

`filter`, `map`, `any`, `all`, `sort` and `sort_by` work on lists and arrays (`sort` and `sort_by` need a `var` list).

| Kestrel | Meaning |
|---|---|
| `xs.filter(f)` | a new list of the items for which `f(item)` is `true` |
| `xs.map(f)` | a new list of `f(item)` for each item |
| `xs.any(f)` | `true` when `f` is `true` for some item; stops at the first |
| `xs.all(f)` | `true` when `f` is `true` for every item; stops at the first `false` |
| `xs.sort()` | sorts in place, smallest first; items are numbers, strings or bools |
| `xs.sort_by(key)` | sorts in place by `key(item)`, a number, string or bool; `key` must be `pure` |

```
let alive = enemies.filter((e) { e.hp > 0 })
let spots = enemies.map((e) { e.pos })
if enemies.any((e) { e.boss }) { alarm() }
enemies.sort_by((e) { e.pos.distance_to(player) })
```

- `f` takes the item read-only. Any function value of the right type works, not only a literal.
- `filter`, `map`, `any` and `all` call `f` once per item, first to last. The `sort_by` key runs twice per comparison, so its order is not promised, and it runs in a scratch arena freed after each comparison.
- Sorting is stable. Strings compare by code point, `false` comes before `true`, and a float NaN sorts last. For largest first, negate a number key.
- `filter` copies the kept items, so it refuses items that own memory (use a `for` loop with `remove`). `map` results may own memory.
- `sort` and `sort_by` are refused inside a loop over the same list (or over a map held in it), and a key can't read the list it sorts. `remove_at`, `insert`, `pop` and `clear` are refused inside a loop over a map held in the list as well. So is a call given the list, or what holds it, as `var`.
- Not built: `reduce`, a comparator sort, `reverse`, `min_by` and `max_by`.

## Memory

```
arena {                      // temporary
    var tris = list[vec3]()
    ...
}                            // everything made inside is freed here

in chunk.mem {               // owned: lives as long as chunk.mem
    chunk.voxels = array[u8](32768)
}
```

- Allocations go to the **current arena**: the innermost `arena { }`
  or `in x { }` block, including inside functions called from it.
- Outside any block, allocations go to the **program arena**, freed at
  exit.
- **Escape check:** a value allocated in an arena may not be stored
  anywhere that outlives that arena. Error names both lifetimes and
  suggests a copy: `chunk.mesh = tris.copy_to(chunk.mem)`.

## Optional values

```
let hp = scores["ana"] else { return }
```

- `T?` holds a `T` or `none`.
- A map lookup is the usual source of one: `scores["ana"]` on a `map[string, int]` is an `int?`, `none` when the key is missing.
- `let x = expr else { ... }` unwraps; the `else` block must exit
  (`return`, `break`, `continue`).
- Using a `T?` as a `T` without unwrapping is a compile error.

## Grammar sketch

```
file      := (use | decl)*
decl      := 'pub'? (func | struct | enum | 'let' binding | 'var' binding)
func      := 'pure'? 'native'? IDENT '(' params? ')' ('->' type)? block?
params    := param (',' param)*
param     := 'var'? (IDENT ':' type | 'self')
struct    := 'struct' IDENT '{' (field | func)* '}'
field     := IDENT ':' type ('=' expr)?
enum      := 'enum' IDENT '{' (variant | func)* '}'
variant   := IDENT ('{' IDENT ':' type (',' IDENT ':' type)* '}')?
type      := IDENT ('[' type (',' type)* ']')? '?'? | '[' type ']' '?'?
           | 'pure'? '(' ('var'? type (',' 'var'? type)*)? ')' ('->' type)?
block     := '{' stmt* '}'
stmt      := 'let' binding | 'var' binding | assign | if | while | for
           | 'par' for | match | arena | in | 'break' | 'continue'
           | 'return' expr? | 'remove' IDENT | expr
closure   := '(' (cparam (',' cparam)*)? ')' ('->' type)? block
cparam    := 'var'? IDENT (':' type)?
binding   := IDENT (':' type)? '=' expr ('else' block)?
for       := 'for' IDENT ('from' expr 'to' expr | (',' IDENT)? 'in' expr) block
arena     := 'arena' block
in        := 'in' expr block
match     := 'match' expr '{' (pattern block)* '}'
pattern   := IDENT IDENT? | 'else'
```

A `native` function has no block, and every other function has one. `native` is only allowed at the top level of a built-in module.

## Out of scope (later specs)

User-defined generic types/functions, interfaces, error types beyond `?`, fixed-size arrays,
C FFI declarations, GPU/shader code. Also not yet: optional function types, methods and
built-ins as values, `reduce`, `reverse`, a comparator sort.

## Resolved in review

- Call-site `var` marker: yes (receiver exempt).
- Logic operators: `and` / `or` / `not` only.
- Index + value loop `for i, v in xs`: included.
