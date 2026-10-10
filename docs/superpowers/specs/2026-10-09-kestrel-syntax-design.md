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
  scope is an error.
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
- **Names must be unique.** An imported name can't collide with the file's own items, another import, or a local variable.
- **Printing** shows a type's own name (`Point`), not `geom.Point`.
- **Types from another module** must be imported by name; `geom.Point` doesn't work in a type position yet.
- **Runtime errors** give the line in their own file, but not the file name yet.

(Implemented 2026-10-10.)

## Types

| Spelling | Meaning |
|---|---|
| `int` | 32-bit signed integer |
| `float` | 32-bit float |
| `bool` | `true` / `false` |
| `i8 i16 i64 u8 u16 u32 u64` | sized integers |
| `f64` | 64-bit float |
| `string` | text, a heap value like an array: copying a string copies its bytes into the current arena (literals stay in read-only data and belong to no arena) |
| `vec2 vec3 vec4` | float vectors, SIMD-backed |
| `ivec2 ivec3 ivec4` | int vectors (voxel coordinates) |
| `[T]` | array of `T`, fixed length once created (fast, arena/SIMD friendly) |
| `list[T]` | growable list |
| `map[K, V]` | hash map: `m[k] = v`; `m[k]` is a `V?` (so `let v = m[k] else { ... }`); `m.has(k)`, `m.remove(k)`, `m.len`, `for k, v in m` (decided 2026-10-10). Literal `{"a": 1, "b": 2}`, empty `{}`; `m.get(k, default)`; `for k in m` gives keys (decided 2026-10-09). Insertion order. Details: `2026-10-09-map-design.md` |
| `T?` | a `T` or `none` |
| `Arena` | a memory bucket |

- **Type parameters use square brackets:** `list[vec3]`, `map[string,
  int]`.
- **Creating generic values:** `list[vec3]()`, `array[int](n)`, or
  annotate and use a literal: `var tris: list[vec3] = []`.
- **Vectors:** `vec3(1, 2, 3)`, operators `+ - * /` component-wise and
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
let tex = load("grass.png") else { return }
```

- `T?` holds a `T` or `none`.
- `let x = expr else { ... }` unwraps; the `else` block must exit
  (`return`, `break`, `continue`).
- Using a `T?` as a `T` without unwrapping is a compile error.

## Grammar sketch

```
file      := (use | decl)*
decl      := 'pub'? (func | struct | enum | 'let' binding | 'var' binding)
func      := 'pure'? IDENT '(' params? ')' ('->' type)? block
params    := param (',' param)*
param     := 'var'? (IDENT ':' type | 'self')
struct    := 'struct' IDENT '{' (field | func)* '}'
field     := IDENT ':' type ('=' expr)?
enum      := 'enum' IDENT '{' (variant | func)* '}'
variant   := IDENT ('{' IDENT ':' type (',' IDENT ':' type)* '}')?
type      := IDENT ('[' type (',' type)* ']')? '?'? | '[' type ']' '?'?
block     := '{' stmt* '}'
stmt      := 'let' binding | 'var' binding | assign | if | while | for
           | 'par' for | match | arena | in | 'break' | 'continue'
           | 'return' expr? | expr
binding   := IDENT (':' type)? '=' expr ('else' block)?
for       := 'for' IDENT ('from' expr 'to' expr | (',' IDENT)? 'in' expr) block
arena     := 'arena' block
in        := 'in' expr block
match     := 'match' expr '{' (pattern block)* '}'
pattern   := IDENT IDENT? | 'else'
```

## Out of scope (later specs)

User-defined generic types/functions, interfaces, closures/lambdas,
string operations, error types beyond `?`, fixed-size arrays,
C FFI declarations, GPU/shader code.

## Resolved in review

- Call-site `var` marker: yes (receiver exempt).
- Logic operators: `and` / `or` / `not` only.
- Index + value loop `for i, v in xs`: included.
