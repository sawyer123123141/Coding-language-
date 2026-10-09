# Core Rebuild 2c-ii: Arenas, moves and the escape check

**Goal:** the `Arena` type, `arena { }` (temporary memory, freed at `}`), `in x { }` (allocate into an arena a value owns), owning types that move instead of copy and free their arenas when dropped, and an escape check that keeps every pointer inside live memory. Everything goes through JIT and exe.

**Spec:** syntax spec (Memory), core spec §2 (Non-copyable types and drops, Arenas runtime) and §3 checks 4 (Moves) and 6 (Arena escape).

**Execution note:** written for and executed by the controller directly (subagents were rate-limited), TDD per step, with a review pass afterwards.

## Design

- **Syntax:** `arena { ... }` and `in <place> { ... }` are statements. `Arena()` creates an arena; it is a built-in call that is allowed in field defaults (`mem: Arena = Arena()`).
- **Types:**
  - `Ty::Arena` / `KTy::Arena` is a pointer to a runtime arena (8 bytes) and a scalar.
  - A type is **owning** if it contains an `Arena` through fields, enum payloads or collection elements. It is computed to a fixpoint per struct and enum, so recursive types terminate.
- **Arena places can't be assigned or passed as `var`.** Values elsewhere in the owner point into the arena, so replacing it would leave them dangling. To start over, replace the whole owner (`chunk = Chunk(...)`).
  - Ruling: this is a ban rather than "free the old arena" — a reset can't be checked safely — cost if wrong: a later `reset` built on whole-owner replacement.
- **Owning values move; copying one is an error.**
  - A move is a read of a whole non-parameter local where it is stored (let, assignment, constructor field, `add`, literal element, return).
  - Moving zeroes the source, so dropping it later does nothing.
  - Moving out of a field, element or parameter is an error ("can't move out of ...; owning values only move as whole variables").
  - Use after move is an error: any mention of the local later in the source, or a move inside a loop when the local is declared outside that loop.
  - This is conservative and flow-insensitive: a move in one `if` branch followed by a use in the other is rejected.
- **Drops:**
  - An owning local is dropped at the end of its block, including on `break`/`continue`/`return` out of it.
  - An owning place is dropped before it's overwritten.
  - Dropping frees each contained arena, inner elements first.
  - Owning temporaries that are never stored (`Chunk(x: 1).x`) leak until exit. Ruling: accepted for now — rare, and arenas allocate nothing until first used.
- **`arena { }`** pushes a fresh arena and frees it at `}` and on every exit edge; locals declared inside are dropped first. **`in x { }`** pushes the arena stored at `x` and pops it without freeing. A zero-filled arena (from `array[Chunk](n)`) is created on first `in`.
- **Escape check.** A value's region says where its memory lives:
  - none (no arena memory: scalars, `[]`, string literals, `Arena()` itself)
  - the function's own context, which is **Program** in `main` and **Caller** elsewhere
  - **Scoped(b)** inside `arena { }` block `b`
  - **Owned(p)** inside `in p.mem { }`, where `p` is the arena's owner as written (a `for ... in` alias stays itself)
- **Region of an expression:**
  - Fresh allocations (non-empty literals, `array[T](n)`, `list[T]()`, copies of places, calls returning heap types) are in the *current* context.
  - A move carries its local's context.
  - Constructors, `if` and `match` take their innermost part.
- **Store rule.** A value stored into a place rooted at a local declared in context `d` must have one of these regions:
  - none
  - Program
  - a Scoped context no deeper than `d`
  - Owned(p) when the target's path starts with `p`, or when the local was declared inside that `in` block
  - Caller when the root isn't a parameter. A Caller value stored into a `var` parameter is the error "this memory belongs to the caller's current arena, which may not live as long as `p`", with help "allocate it with `in p.mem { }`".
- **Return rule.** A returned value's region must be none or the function's own context.
- **Runtime safety net:** freeing an arena that is on the current stack (for example, replacing `chunk` inside `in chunk.mem { }`) panics instead of freeing.
- **KIR:**
  - `KTy::Arena`.
  - Expressions: `ExprKind::{NewArena, Move(Place)}`.
  - Statements: `Stmt::{PushArena(Option<Place>), PopArena { free: bool }, FreeArena(Place)}`.
  - Drop statements are generated inline for structs and enums, and as per-type helpers for collections.
  - The lowerer keeps a scope stack (owning locals, arena pushes, loop markers) and emits cleanups before `break`/`continue`/`return` and at block ends. A return value is computed into a temporary before cleanups run.
- **Runtime:** `kestrel_rt_arena_new`, `kestrel_rt_arena_free` (null-safe, panics if pushed), `kestrel_rt_arena_push` and `kestrel_rt_arena_pop`.

## Steps

1. **Syntax:** `Stmt::Arena { body }` and `Stmt::In { arena, body }`. Parser tests.
2. **Runtime:** arena new/free/push/pop and the in-use check. Unit tests.
3. **Checker:**
   - `Ty::Arena` and `Arena()`.
   - `in` and `arena` statements.
   - Owning fixpoint, the arena-place assignment ban, moves and use-after-move (`Tables::moves`), regions and the store and return rules.
   - Tests for every error.
4. **KIR:** arena statements, moves, drops (helpers), and the scope stack with exit cleanups. KIR tests.
5. **Codegen:** the new statements and expressions, `KTy::Arena`, runtime declarations. JIT tests.
6. **End to end:** program tests (temporary arena in a loop with `break`, `in self.mem`, a list of owning chunks, moves) and error tests.
