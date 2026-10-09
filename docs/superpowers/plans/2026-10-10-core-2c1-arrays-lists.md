# Core Rebuild 2c-i: Arrays, lists and `for in`

**Goal:** `[T]` arrays and `list[T]` lists with literals, `array[T](n)`, `list[T]()`, bounds-checked indexing, `.len`, `.add`, `for v in xs` / `for i, v in xs`, deep-copy value semantics and printing, through JIT and exe. Every allocation goes to the program arena. Scoped arenas, moves/drops and the escape check are 2c-ii.

**Spec:** syntax spec (Types, Expressions, Control flow), core spec §2 (layout table, value semantics, indexing, arenas runtime).

**Execution note:** written for and executed by the controller directly (subagents were rate-limited), TDD per step, with a review pass afterwards.

## Design

- **Syntax:**
  - `[a, b, c]` and `[]` are array literals.
  - Postfix `xs[i]` chains with `.field` and `.method()`.
  - `array[T](n)` and `list[T]()` are type calls: an identifier `array`/`list`/`map` directly followed by `[`.
  - `for v in xs { }` and `for i, v in xs { }`.
  - `array`, `list` and `map` become built-in names, so they can't name a variable or function.
- **Types:**
  - `Ty::Array(ElemId)` and `Ty::List(ElemId)`. `Tables::elems` interns element types, so `Ty` stays `Copy`.
  - `KTy::Array(ElemIdx)` (16 bytes: `{ptr, len: i64}`) and `KTy::List(ElemIdx)` (32 bytes: `{ptr, len, cap, arena}`). Both are aggregates, and `Module::elems` mirrors the element types.
  - Element types can be anything, including types that contain themselves through a collection (`struct Node { kids: [Node] }`): the pointer breaks the size cycle.
- **Literals:**
  - The element type comes from the expected type, else from the first element. Later elements coerce to it.
  - `[]` needs an expected type. With a list expected, a literal builds a list whose `cap` equals its length.
- **`array[T](n)`:** `n` is any integer, and the memory is zero-filled. Zero is a valid value of every type: 0, false, the first variant, an empty array or list, `""`. Field defaults are *not* applied.
- **Indexing:**
  - Any integer index is widened to i64.
  - It is always bounds-checked: `index 5 out of bounds for length 3 on line N`, exit 101.
  - `xs[i]` is a place, so it can be assigned, passed as `var`, used as a receiver, and projected (`xs[i].hp -= 1`).
- **`.len`** is `int`. Lengths are stored as i64, and reading one that exceeds `int` panics (narrowing check).
  - Ruling: `int` is used, not `i64` — `for i from 0 to xs.len` then `xs[i] = i` must work without conversions — cost if wrong: a later `len64`.
- **`.add(v)`** works only on lists, and the receiver must be changeable (like `var self`). `v` is evaluated (and copied if needed) before the list grows. Growth doubles the capacity (minimum 4) in the list's own arena; a null arena (a zero-filled list) means the program arena (changed in 2c-ii: the current one could be a temporary that dies first).
- **`for i, v in xs`:**
  - `i: int`. `v` aliases `xs[i]` (`LocalInfo::alias` to the root), and can be changed when the root can.
  - A non-place `xs` is stored in a temporary first.
  - The length is read once before the first iteration. `v` is re-indexed (bounds-checked) on each use, so changing `xs` in the body is safe.
- **Value semantics (copies):**
  - Every place read stored somewhere else deep-copies the heap parts of its value into the current arena. That covers let, assignment, constructor and variant fields, literal elements, `.add` arguments, and returning a parameter.
  - Fresh values (calls, literals, `array[T](n)`) are not copied.
  - Returning a local, or the tail of a local, moves it instead of copying, because the local dies.
  - Read-only parameters are passed by pointer and not copied.
  - Strings are immutable and shared, not copied.
  - Last-use moves for other cases come with 2c-ii's move analysis.
- **KIR helpers:** functions generated during lowering, memoized by type and appended after the user functions.
  - `copy[T]` for each array or list type with heap parts: `Dup` its buffer, then fix up each element's heap parts in a loop.
  - `print[T]` for each array or list type that is printed: `[1, 2, 3]`, with strings quoted.
  - Struct and enum fix-ups are generated inline. Collections always go through a helper call, which is why recursive types terminate.
  - Drops in 2c-ii will use the same mechanism.
- **KIR additions:**
  - `Proj::Index(Expr)` (the line comes from the expression).
  - Expressions: `ExprKind::{ArrayNew(elem, len), ArrayLit(elems), ListNew, Len(Place)}`.
  - Statements: `Stmt::{Dup(Place), ListAdd(Place, Expr), Write(Vec<PrintArg>)}` (`Write` prints with no separators and no newline).
  - `Module::print_fns: HashMap<KTy, FuncIdx>`.
- **Runtime:**
  - A chunked bump arena (64 KB zeroed blocks; a big request gets its own block) and a thread-local current-arena stack, with the program arena at the bottom.
  - Functions: `kestrel_rt_alloc(size, align) -> ptr`, `kestrel_rt_dup(ptr, bytes, align) -> ptr`, `kestrel_rt_list_grow(list, elem_size, elem_align)`, `kestrel_rt_current_arena() -> ptr` and `kestrel_rt_panic_index(i, len, line) -> !`.
- **Codegen:**
  - `place_addr` walks projections and emits Index loads and bounds checks.
  - Array literals allocate and store each element.
  - `ListAdd` has an inline fast path (`len < cap`) and a cold `list_grow` call.
  - `Dup` reallocates and copies the buffer. For lists it also sets `cap = len` and the arena to the current one.
  - Printing an array or list calls its `print[T]` helper with the value's address.

## Steps

1. **Syntax:** array literals, index postfix, type calls and `for in`. Parser tests.
2. **Checker:** element interning, `resolve_type`, literals, indexing places, `.len`/`.add`, type calls, `for in` with alias bindings, built-in names, exclusivity through `Index`. Checker tests and error tests.
3. **Runtime:** the arena, the current-arena stack, alloc/dup/grow, and the index panic. Rust unit tests.
4. **KIR:** types, places, new expressions and statements, `for in` lowering (to `ForRange` over an alias place), copy points, and the copy/print helpers. KIR tests.
5. **Codegen:** layouts, indexed places with bounds checks, literals, `ArrayNew`/`ListNew`/`Len`/`ListAdd`/`Dup`/`Write`, printing via helpers, runtime declarations and JIT symbols. JIT tests.
6. **End to end:**
   - Program tests: `arrays.kes`, `lists.kes`, `array_copies.kes`, `index_oob.kes`, and a recursive `Node` tree.
   - Error tests.
   - A benchmark (a sieve plus a list of structs) against Rust, recorded in `kestrel/bench/2a/results.md`.
