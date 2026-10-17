# List and String Basics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add list methods (`remove_at pop insert clear has find`), the `remove b` statement, string length, indexing, character loops and `+`, string methods, and `draw.fill`.

**Architecture:** Each feature runs through `syntax → check → kir → codegen`. New `kestrel_rt_*` functions go in `runtime/src/list.rs` and `runtime/src/string.rs`. The templates:
- **List methods:** the map methods (`check/src/body/maps.rs`, `kir/src/lower/maps.rs`, `MapOp`, `codegen/src/func/maps.rs`).
- **`remove b`:** the map-loop lock (a `Scope` whose exits emit a statement; `kir/src/lower/scopes.rs` `locked`) and the move checker (`check/src/body/moves.rs`).
- **Strings:** `Format`'s `str_begin`/`str_end` pair and `map_get_into`'s "write an aggregate result into `dest`".

**Tech Stack:** Rust, Cranelift. Tests are Rust unit tests plus `.kes` program and error tests.

**Spec:** `docs/superpowers/specs/2026-10-17-lists-strings-design.md`. Every message and behaviour below comes from it. The spec wins any conflict.

## Global Constraints

- **Messages:** plain sentences that say what to do, worded exactly as in the spec.
- **File sizes:** keep source files under about 400 lines, split by responsibility.
  - Tight already: `check/src/body/mod.rs` (397), `check/src/body/call.rs` (391), `check/src/collect.rs` (389), `kir/src/ir.rs` (405), `kir/src/lower/helpers.rs` (375), `check/src/tables.rs` (370).
  - New checker code goes in new files: `check/src/body/lists.rs`, `check/src/body/remove.rs`, `check/src/body/strings.rs`.
  - New KIR code: `kir/src/lower/lists.rs`, `kir/src/lower/remove.rs`, `kir/src/lower/strings.rs`.
  - New op enums go in `kir/src/ops.rs`. Each adds only a one-line variant to `ir.rs`.
- **Op enums live in the checker.** `kir` depends on `check`, not the other way round, so the checker's method enums go in `check/src/tables.rs` (`ListMethod`, `StrMethod`). Lowering maps them to KIR's `ListOp`/`StrOp`.
- **Every new KIR statement or expression that writes a list** (remove, pop, insert, clear, set-length, compact) must:
  - be reported as a write of that place in `kir/src/visit.rs`, as `MapOp::Remove` is at `visit.rs:116-121`, so `may_move` re-indexes loop items;
  - be pinned in `scalarize.rs`;
  - be handled in `visit_mut.rs`, `rename.rs` and `inline.rs`.
- **Runtime functions** need four things:
  - a field in `RtFuncs` and a `decl` in `codegen/src/rt.rs`;
  - an entry in `jit_symbols()` (the Godot add-on uses the same table);
  - `#[no_mangle] pub extern "C"` in the runtime crate.

  The exe links by name.
- **Runtime panics** take `(line: i32, file: *const FileName)`, as `kestrel_rt_panic_index(index, len, line, file)` does (`runtime/src/panic.rs:80`). Codegen passes `loc_args(line)` (`codegen/src/func/collections.rs:46`). Every runtime function below that can panic takes `line, file` in that position, and its message ends "on line N of f.kes" through the existing path.
- **Test program facts:**
  - **No string literals inside `{...}`.** The lexer doesn't allow them. Use `print(a, b)`, which separates with a space, or locals.
  - **`var xs = [1, 2]` is an array.** A list needs `var xs: list[int] = [...]`.
  - **Lists of strings print quoted:** `["b", "c"]`. Optionals print as the value or `none`.
  - **Expect lines are trimmed,** so a trailing space never matches.
  - **Running the tests:**
    - program tests: `cargo test -p kestrel-cli --test programs programs_`
    - error tests: `cargo test -p kestrel-cli --test errors error_programs`
    - Name filters on file names match nothing.
- **Off-limits:** never touch `kestrel/test.kes` or `voxels.exe`. Never `git add -A`, `git stash` or `git clean`.
- **Window programs** run only with `// env: KESTREL_HEADLESS=<n>` (tests) or `KESTREL_HIDDEN=1`.
- **Linker:** never call a bare `link.exe`.
- **Escapes:** shell heredocs mangle `\n`, so write files that contain escapes with file tools.
- **Before each commit,** run in `kestrel/`:
  - `cargo test --workspace --no-fail-fast` (`relocated` can flake with "file in use"; re-run it alone);
  - `cargo clippy --workspace --all-targets -- -D warnings`.
- **Commits:** messages are plain sentences like the repo's history, ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Work on `main`; the controller pushes.

## Review Focus

1. **Owning items** (a struct holding an `Arena`) through `remove_at`, `pop`, `clear` and `remove b`: each item must be dropped exactly once, and `pop` moves out. Task 1 adds a live-arena count at exit to prove it; Tasks 1 and 2 use it.
2. **`remove b` exits:** `break`, `continue` and `return` inside nested `if`/`match` and inside an inner loop must all finish the compaction. An inner loop's `break` must not finish it. (Task 2.)
3. **New strings stored past their arena:** the result of `s[i]`, `+` or `s += t` built inside `arena { }` and stored outside must be refused, as interpolation is. (Tasks 3 and 4.)
4. **Non-ASCII text** (`"héllo"`, emoji) through `len`, `s[i]`, `slice`, `find`, `split` and `for c in s`: indexes count characters. (Tasks 3 and 4.)
5. **Empty and zero values:**
   - an empty list: `pop` gives `none`; `clear` and `remove` work;
   - an empty string: `len` is 0; `split`, `trim` and `find("")` behave as the spec says;
   - a zero-filled string (`array[string](1)[0]`, null `ptr`).

   (Tasks 1, 3 and 4.)

---

### Task 1: List methods

**Files:**
- Create:
  - `kestrel/check/src/body/lists.rs`: `list_method` for `add remove_at pop insert clear has find`, moving `add` out of `collections.rs`; the loop-overlap check.
  - `kestrel/kir/src/lower/lists.rs`.
  - `kestrel/codegen/src/func/lists.rs`.
  - `kestrel/runtime/src/list.rs`.
- Modify:
  - `check/src/body/collections.rs`: `collection_method` dispatches to `lists.rs`; `check_for_in` pushes and pops the loop stack below.
  - `check/src/tables.rs`: `ListMethod`, and `Res::List(ListMethod)` alongside `ListAdd`.
  - `check/src/body/nested.rs:16-19`: list writes change their receiver.
  - `check/src/body/call.rs` `check_discarded_call`: a discarded `has`/`find` says "this only reads the list, so the line does nothing".
  - `check/src/body/regions.rs` `region()`: `pop`'s region is its list's, as described below.
  - `kir/src/ops.rs` (`ListOp`) and `kir/src/ir.rs`.
  - `kir/src/visit.rs`, `visit_mut.rs`, `rename.rs`, `inline.rs` and `scalarize.rs`, per the Global Constraints.
  - `kir/src/lower/mod.rs:249-292` (discarded calls) and `lower/expr.rs:105-121` (value calls).
  - `kir/src/lower/temps.rs:22-31` (`builds_new`).
  - `codegen/src/func/mod.rs`, `codegen/src/rt.rs`, `runtime/src/lib.rs`.
  - `runtime/src/start.rs` and `runtime/src/arena.rs` for the live-arena count.
- Test:
  - `check/tests/collections.rs`, `kir/tests/collections.rs`;
  - `tests/programs/list_methods.kes`, `tests/programs/list_owning_remove.kes`, `tests/errors/list_methods.kes` (all new);
  - unit tests in `runtime/src/list.rs`.

**Interfaces:**
- Produces (runtime):
  - **`kestrel_rt_list_remove(list: *mut List, i: i64, elem_size: i64, line: i32, file: *const FileName)`.** Panics out of range (negative included) with the existing index message. Memmoves the tail down, zeroes the vacated last slot, then `len -= 1`. KIR drops the item before calling it.
  - **`kestrel_rt_list_insert(list: *mut List, i: i64, elem_size: i64, align: i64, line: i32, file: *const FileName) -> *mut u8`.** Panics when `i < 0 || i > len`. Grows by calling the existing `kestrel_rt_list_grow` when full, memmoves up, sets `len += 1`, and returns the slot to store into.
- Produces (runtime test hook): with `KESTREL_ARENA_COUNT=1` set, the runtime writes `arenas alive at exit: N` to stderr when the program ends. N counts `Arena` values created minus freed. Tests use it with `// env:` and `// expect-stderr:`.
- Produces (checker): a stack `for_in_lists: Vec<LoopList>` pushed and popped in `check_for_in` for loops over a list or an array. Each entry holds the loop item's `LocalId`, the iterated expression's id, the alias-resolved root `LocalId`, and its field path.
  - `fn overlapping_loop(&self, place: &Expr) -> Option<usize>` (in `lists.rs`) returns the innermost entry whose list overlaps `place`.
  - **Overlap rules:**
    - Roots are compared after following `locals[..].alias`, so a loop item counts as part of its list.
    - Two places overlap when one field path is a prefix of the other.
    - An index projection counts as covering the whole list.
  - Task 2 uses this.
- Produces (KIR): `ListOp { RemoveAt, Pop, Insert, Clear, Find }` (`has` is `Find` plus a tag test), and `Stmt::SetLen(Place, Expr)` if no statement can write a list length today.

Steps:

- [ ] **Step 1: Write the failing program test** `kestrel/tests/programs/list_methods.kes`:

```
// expect: [1, 3]
// expect: 3 [1]
// expect: none
// expect: [0, 1, 2]
// expect: [0, 1, 2, 9]
// expect: []
// expect: true false
// expect: 1 none
// expect: true 0
// expect: ["b", "c"] 2
// expect: [3, 1, 2, 3]
// expect: [1, 2, 3, 1, 2, 3]
main() {
    var xs: list[int] = [1, 2, 3]
    xs.remove_at(1)
    print(xs)
    let p = xs.pop()
    print(p, xs)
    var e: list[int] = []
    print(e.pop())
    var ys: list[int] = [0, 2]
    ys.insert(1, 1)
    print(ys)
    ys.insert(3, 9)
    print(ys)
    ys.clear()
    print(ys)
    let names = ["ana", "bo"]
    print(names.has("bo"), names.has("cy"))
    print(names.find("bo"), names.find("cy"))
    let arr = [4, 5]
    print(arr.has(4), arr.find(4))
    var ss: list[string] = ["a", "b", "c"]
    ss.remove_at(0)
    print(ss, ss.len)
    var zs: list[int] = [1, 2, 3]
    zs.insert(0, zs[2])
    print(zs)
    var grow: list[int] = [1, 2, 3]
    for g in grow { grow.add(g) }
    print(grow)
}
```

- [ ] **Step 2: Write the owning-items test** `tests/programs/list_owning_remove.kes`.
  - Setup: `// env: KESTREL_ARENA_COUNT=1`, `// expect-stderr: arenas alive at exit: 0`, and `struct Pool { mem: Arena = Arena(); n: int }`.
  - The program builds `var ps: list[Pool]` with `n` 1 to 5.
  - It calls `ps.remove_at(0)`, then `let last = ps.pop() else { return }` and `print(last.n)`, then a discarded `ps.pop()` statement, then `let maybe = ps.pop()` (never unwrapped, so it is dropped at the end of the block), then prints the `n`s left, then `ps.clear()`.
  - Expect lines are the printed `n`s.
  - Add KIR tests in `kir/tests/collections.rs`:
    - `remove_at` on `list[Pool]` lowers to one element drop before the runtime remove;
    - `pop` lowers to a move, with no drop inside the list op.
  - Add a runtime test that the count goes up on `arena_new` and down on free.

- [ ] **Step 3: Write the failing error test** `kestrel/tests/errors/list_methods.kes`. One case each:
  - `remove_at` on a `let` list: the `add`-style "can't call `remove_at` on `xs`: it changes `xs`, which was declared with `let`".
  - `pop` on an array: "arrays have a fixed length; use a `list[int]` if items come and go".
  - `has` on `list[vec2]`: "`has` needs items that `==` can compare (numbers, bools and strings); `list[vec2]` holds `vec2`".
  - A discarded `xs.find(1)`: "this only reads the list, so the line does nothing".
  - `xs.remove_at(0)` inside `for x in xs`: "can't change how many items `xs` has while looping over it; use `remove x` to remove the loop's item, or change `xs` after the loop".
  - `w.items.clear()` inside `for x in w.items`, and `w.items.pop()` inside `for x in w.items`: the same message.
  - `row.clear()` inside `for x in grid[0]` inside `for row in grid`: the same message, naming `grid`.
  - `xs.insert(1.5, 2)`: the index-type message `check_index` uses.
  - A pop that escapes. `var keep: string? = none` outside, then:

    ```
    arena {
        var xs: list[string] = []
        xs.add("{n}")
        keep = xs.pop()
    }
    ```

    Expect the existing arena message ("this memory is freed at the end of the `arena { }` block, but `keep` lives longer").

- [ ] **Step 4: Run them and see them fail** ("has no method `remove_at`"), using the test commands in the Global Constraints.

- [ ] **Step 5: Implement.**
  - **Checker:**
    - Argument checks work as for `add`: `check_element` or `check_value` with the item type. An index is `int`. `insert` uses `check_store` like `add`.
    - Results: `pop` is `Ty::Optional(elem)`, `find` is `Ty::Optional(int)`, `has` is `Bool`, and the rest are `Unit`.
    - Writes go through `receiver_root`. The loop refusal uses `overlapping_loop(receiver)`.
    - `region()` for `xs.pop()` is the region of `xs`'s root, computed like a moved local (`regions.rs:148`) and never `current()`.
  - **KIR:**
    - **`remove_at`:** put the index in a temp. Drop the checked `xs[i]` with `drop_stmts` when the item type is owning. Then call the runtime remove.
    - **`pop`:** put the length in a temp `n`. If `n == 0` the result is `NoneLit`. Otherwise it is `Some(Move(xs[n - 1]))` with a checked index, and the length is set to `n - 1` after the move (`SetLen`). In `builds_new` (`temps.rs`), `Pop` is a new value, so a printed or otherwise unheld result is held and dropped. A discarded `pop` statement lowers to the same and drops it (no `unreachable!`).
    - **`insert`:** first evaluate the value into a temp (with `add`'s copy rules, `owned`), then call the runtime insert, then store into the returned slot. The order matters: `xs.insert(0, xs[2])`.
    - **`clear`:** call the generated `drop_fn` for owning items, then `SetLen 0`.
    - **`has` and `find`:** a generated helper per item type with a loop and `==`, like `copy_fn` and `drop_fn` in `lower/helpers.rs`, or a new `lower/list_helpers.rs` if `helpers.rs` would pass 400 lines. Strings compare with `str_eq`.

- [ ] **Step 6: Run the tests until they pass,** then run the full suite and clippy.

- [ ] **Step 7: Commit:** "Add remove_at, pop, insert, clear, has and find to lists".

### Task 2: `remove b` in a loop over a list

**Files:**
- Create: `kestrel/check/src/body/remove.rs`, `kestrel/kir/src/lower/remove.rs`.
- Modify:
  - `syntax/src/ast.rs`: `Stmt::Remove { name: String, name_span: Span, span: Span }` and `span()`.
  - `syntax/src/parser/stmt.rs`.
  - `syntax/src/sexpr.rs`, `syntax/src/link.rs`: every exhaustive `Stmt` match.
  - `check/src/body/mod.rs` `check_stmt`; `check/src/body/nested.rs` `changed_in_block`, where `remove b` changes the root of `b`'s alias.
  - `check/src/body/moves.rs`: removal reuses moves, with a "removed" kind.
  - `check/src/body/place.rs`: assigning to a removed item is refused.
  - `check/src/body/collections.rs` `check_for_in`.
  - `check/src/tables.rs`: what the checker records for KIR, below.
  - `kir/src/lower/mod.rs` `stmt()`, `kir/src/lower/collections.rs` `for_in`/`list_loop`, `kir/src/lower/scopes.rs`.
  - `runtime/src/list.rs`, `codegen/src/rt.rs`.
- Test: `syntax/tests/parser.rs`, `check/tests/remove.rs` (new), `kir/tests/collections.rs`, `tests/programs/remove_in_loop.kes` (new), `tests/errors/remove_in_loop.kes` (new).

**Interfaces:**
- Consumes: Task 1's `for_in_lists`, `overlapping_loop` and `KESTREL_ARENA_COUNT`.
- Produces (runtime): **`kestrel_rt_list_compact(list: *mut List, kept: i64, next: i64, elem_size: i64)`.**
  - If `next <= len`: memmove items `[next, len)` to `kept`, zero the vacated slots `[kept + (len - next), len)`, and set `len = kept + (len - next)`.
  - Otherwise: set `len = min(len, kept)`.
  - It never touches memory outside `[0, len)` of the list as it is at the call.
- Produces (tables): `Tables::removing_loops: HashSet<ExprId>` (the iterated expression of each loop whose body removes its item) and `Tables::removes: HashMap<StmtSpan or id, LocalId>` (each `remove` statement's item). Pick the key type KIR can find.
- Parser: `remove` followed by an identifier token on the same line, at the start of a statement, is `Stmt::Remove`. Everything else starting with `remove` parses as today.

Steps:

- [ ] **Step 1: Write the failing program test** `tests/programs/remove_in_loop.kes`. Use `var x: list[int] = [...]` lists:

```
// expect: [1, 3, 5, 7, 9]
// expect: [] [4, 5]
// expect: [1, 3, 4, 5]
// expect: [1, 3, 4]
// expect: [1, 3]
// expect: World(bricks: [1, 2], score: 7)
// expect: [10, 12]
// expect: ["ann", "cy"]
// expect: 4
struct World {
    bricks: list[int]
    score: int = 0
}

drop_two_stop_at_three(var xs: list[int]) {
    for x in xs {
        if x == 2 { remove x }
        if x == 3 { return }
    }
}

remove(n: int) -> int { n + 1 }

main() {
    var a: list[int] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    for x in a { if x % 2 == 0 { remove x } }
    print(a)
    var all: list[int] = [1, 2]
    for x in all { remove x }
    var kept: list[int] = [4, 5]
    for x in kept { if x > 9 { remove x } }
    print(all, kept)
    var b: list[int] = [1, 2, 3, 4, 5]
    for x in b {
        if x == 2 { remove x; continue }
        if x == 3 { break }
    }
    print(b)
    var c: list[int] = [1, 2, 3, 4]
    drop_two_stop_at_three(var c)
    print(c)
    var bricks: list[int] = [1, 2, 3, 4]
    let balls = [2, 4, 9]
    for br in bricks {
        for ball in balls {
            if ball == br { remove br; break }
        }
    }
    print(bricks)
    var w = World(bricks: [1, 2, 3, 4])
    for br in w.bricks { if br > 2 { w.score += br; remove br } }
    print(w)
    var d: list[int] = [10, 11, 12, 13]
    for i, x in d { if i == 1 or i == 3 { remove x } }
    print(d)
    var names: list[string] = ["ann", "bob", "cy"]
    for n in names { if n == "bob" { remove n } }
    print(names)
    print(remove(3))
}
```

  If a struct field `bricks: list[int]` won't take a list literal in construction, build `w` another way. Don't change the loops.

  Also add `tests/programs/remove_owning.kes`:
  - `// env: KESTREL_ARENA_COUNT=1` and `// expect-stderr: arenas alive at exit: 0`;
  - a `list[Pool]` (see Task 1) with removes in an `if`, in a `match` arm on an enum field of `Pool`, and before `continue` and `break`;
  - print the `n`s left.

  Add KIR tests:
  - `remove b` on an owning item lowers to exactly one drop;
  - a removing loop has no element `Bind` (it re-indexes), and `promote` doesn't promote its item (check the optimised KIR).

- [ ] **Step 2: Write the failing error test** `tests/errors/remove_in_loop.kes`. One case per spec message:
  - not a loop item;
  - a loop over an array;
  - a loop over a map;
  - in a `par for`;
  - a loop over a `let` list;
  - a loop over a temporary (`for b in make()`);
  - an outer loop over the same list (`for a in xs { for b in xs { remove b } }`);
  - a use after `remove b` ("`b` was removed earlier");
  - "may have been removed earlier";
  - a second `remove b`;
  - `b = 5` after `remove b`;
  - `remove b` in an inner `while` without `break` (the loop message with "removed");
  - `remove b` inside `match b { ... }` whose arm binds a field of `b` (the existing match-guard message);
  - uses of the list in the body: `xs.len`, `xs[0]`, and passing `w` when looping over `w.bricks`. All three are refused. `w.score += 1` in the same loop is allowed.

  The string case ("`remove` works in loops over a list; strings can't change, so build a new one") can't be written until Task 3 makes strings loopable. Add it as a `check/tests/remove.rs` unit test that's ignored with the reason, and Task 3 turns it on.

- [ ] **Step 3: Add parser tests** in `syntax/tests/parser.rs`:
  - `remove b` gives `Stmt::Remove`;
  - `remove(b)`, `remove = 1`, `let remove = 2` and `x.remove(k)` parse as before.

- [ ] **Step 4: Run them and see them fail.**

- [ ] **Step 5: Implement the checker.**
  - **Marking removing loops.** `check_for_in` marks a loop as removing when its body contains `remove <its item>` anywhere, including nested blocks and loops. A syntactic pre-scan is fine. Record it in `Tables::removing_loops`.
  - **Uses of the list.** A removing loop refuses, in its body, uses of places that overlap its list (Task 1's rule). It allows disjoint fields.
  - **Resolving `remove v`.** It finds `v`'s `for_in_lists` entry. Then:
    - It refuses the cases the spec lists: not an item; array, map or string; `par`; a temporary; an outer loop over an overlapping list; a list that can't change, checked like `receiver_root`.
    - It calls `check_not_matched` on `v`'s root.
    - It records a move of `v` with a "removed" kind. Messages say "removed": "was removed earlier", "may have been removed earlier", and the loop message.
  - **Back edges.** The item is declared at the outer loop depth (`collections.rs:199`, before `loop_body`). So, beside `loop_moves`, keep `loop_items: Vec<Option<LocalId>>`. At the back edge of the item's own loop, end of body or `continue` (`moves.rs` `back_edge`), skip that loop's own item and clear it from `moved`. An inner loop's back edge still reports it.
  - **Assignment.** Assigning to a removed item is refused ("`b` was removed earlier"). It doesn't re-initialise it as it does a moved `var`.

- [ ] **Step 6: Implement KIR and runtime.** For a loop in `removing_loops`:
  - **Before the loop:**
    - `w = 0` (kept count) and `next = 0`;
    - wrap the loop in a non-loop scope, like `locked` in `scopes.rs:45-50`, whose exit cleanup (taken only by `return`) emits `list_compact(xs, w, next)`.
  - **The loop scope's cleanup,** emitted at the body's normal end and by `continue`, `break` and `return` (`jump`/`exit_cleanup`), is the keep step:
    - if not `removed`: when `w != i`, move `xs[i]` to `xs[w]` and zero `xs[i]`; then `w += 1`;
    - then `next = i + 1`.
  - **Each iteration** starts with `removed = false`.
  - **`remove v`:** drop the item when it's owning (`drop_stmts` on the checked `xs[i]`), zero the slot, and set `removed = true`.
  - **After the `ForRange`,** where the normal end and `break` both land: `list_compact(xs, w, next)`.
  - **The loop item** is always re-indexed (skip the `Bind`), and the loop must not be promoted.
  - **Runtime:** `kestrel_rt_list_compact` with unit tests, including a list that grew and one that shrank before the call.

- [ ] **Step 7: Run the tests until they pass,** then run the full suite and clippy.

- [ ] **Step 8: Commit:** "Remove the current item of a list loop with `remove b`".

### Task 3: String length, indexing, character loops and `+`

**Files:**
- Create: `kestrel/check/src/body/strings.rs`, `kestrel/kir/src/lower/strings.rs`, `kestrel/codegen/src/func/strings.rs`, `kestrel/runtime/src/string.rs`.
- Modify:
  - `check/src/body/structs.rs`:
    - `check_field`: `.len` on `Str` is `Int`; other fields say "string has no field `x`" with help "its length is `.len`";
    - `check_method_call`: dispatches `Str` receivers to `strings.rs` (Task 4 fills the methods; this task's unknown-method message and help are the spec's).
  - `check/src/body/collections.rs`: `check_index` and `check_for_in` for `Str`; `par for` over a string keeps "`par for` loops over an array or a list".
  - `check/src/body/expr.rs`: `+` of two strings is `Str`; a mixed `+` gets the spec's message and help.
  - `check/src/body/place.rs`:
    - `s += t` on a string;
    - `s[i] = x` refused;
    - `place_root` treats a string's `.len` as no place, as it does arrays, lists and maps (`place.rs:29`), so `s.len = 1` is an error.
  - `check/src/body/regions.rs`: `region()` of `s[i]`, `a + b` and every Task 3/4 string result is `current()`. A string `+=` is checked as storing a `current()` value (`place.rs:131` calls `check_store` with the value's own region, which would be wrong).
  - `kir/src/lower/call.rs:25-29`: `place()` doesn't turn `s[i]` into a place (exclude `Str`, as maps are).
  - `kir/src/lower/mod.rs:220`: compound assignment of a `Str` lowers to an `Assign` of `Format[s, t]`, not `vec_combine`.
  - `kir/src/lower/expr.rs`, `codegen/src/rt.rs`, `runtime/src/lib.rs`.
  - `tests/errors/strings.kes:9`: `"a" + "b"` becomes valid, so change it to `"a" + 1` with the new message.
  - Turn on Task 2's ignored string-`remove` test and add its line to `tests/errors/remove_in_loop.kes`.
- Test: `check/tests/strings.rs`, `tests/programs/string_basics.kes`, `tests/programs/string_index_oob.kes`, `tests/errors/string_basics.kes` (new), runtime unit tests.

**Interfaces:**
- Produces (runtime). `KStr` is `#[repr(C)] { ptr: *const u8, len: i64 }`, defined in `runtime/src/string.rs`. A null `ptr` with `len` 0 is the empty string everywhere.
  - `kestrel_rt_str_chars(ptr: *const u8, len: i64) -> i64`: the number of characters.
  - `kestrel_rt_str_at(ptr, len, i: i64, line: i32, file: *const FileName, out: *mut KStr)`: a new one-character string in the current arena. Panics "index {i} out of bounds for length {chars}".
  - `kestrel_rt_str_char_len(ptr, len, pos: i64) -> i64`: the byte length of the character starting at byte `pos`.
  - `kestrel_rt_str_piece(ptr, len, pos: i64, n: i64, out: *mut KStr)`: a new string of bytes `[pos, pos + n)` in the current arena.
- Produces (checker): `StrMethod` in `tables.rs` and a `Res::Str(StrMethod)` (with `Len` and `At`, or another way to mark them), which Task 4 extends.
- Produces (KIR): `StrOp { Chars, At, CharLen, Piece }` in `ops.rs` and `ExprKind::Str(StrOp, Vec<Expr>)`. Task 4 extends both.
- `a + b` lowers to `ExprKind::Format` of the two values, with no new runtime code.

Steps:

- [ ] **Step 1: Write the failing program test** `tests/programs/string_basics.kes`:

```
// expect: 5 5 0 0
// expect: h é o
// expect: h-é-l-l-o-
// expect: 0h,1é,2l,3l,4o,
// expect: héllo world
// expect: ab!
// expect: true
// expect: héllo
main() {
    let s = "héllo"
    let e = ""
    let blank = array[string](1)
    print(s.len, "hello".len, e.len, blank[0].len)
    print(s[0], s[1], s[4])
    var out = ""
    for c in s { out += c + "-" }
    print(out)
    var line = ""
    for i, c in s { line += "{i}{c}," }
    print(line)
    print(s + " " + "world")
    var t = "a"
    t += "b"
    t = t + "!"
    print(t)
    print(s[1] == "é")
    var u = "héllo"
    var walked = ""
    for c in u { u = "x"; walked += c }
    print(walked)
}
```

  Add `tests/programs/string_index_oob.kes`: `print("héllo"[5])` (or a local), with `// expect-exit: 101` and `// expect-stderr: index 5 out of bounds for length 5 on line N`.

- [ ] **Step 2: Write the failing error test** `tests/errors/string_basics.kes`:
  - `s[0] = "x"`: "strings can't be changed in place; build a new one with `+` or `\"{...}\"`"
  - `s.len = 1`: an error, whichever existing message `place_root` gives a non-place
  - `"a" + 1`: "`+` needs two numbers or two strings, found string and int"
  - `s.size`: "string has no field `size`"
  - `s.reverse()`: "string has no method `reverse`"
  - `par for c in s`: "`par for` loops over an array or a list"
  - `var keep = ""` then `arena { let t = "{n}"; keep = t[0] }`: the existing arena message ("this memory is freed at the end of the `arena { }` block, but `keep` lives longer")
  - `var keep = ""` then `arena { keep += "x" }`: the same message
  - `s[1.5]`: the index-type message

- [ ] **Step 3: Run them and see them fail.**

- [ ] **Step 4: Implement.**
  - **Checker:**
    - `for c in s` declares `c: string`, not changeable. It is a plain local, not a view, since it gets a new string each pass.
  - **KIR:**
    - `.len` of a string is `Str(Chars, [s])`. `ExprKind::Len` on a `Str` gives bytes, so don't use it.
    - `s[i]` is `Str(At, ..)`, written through `lower_into` like `map_get_into`.
    - `for c in s` reads a snapshot of `s`'s header before the loop. Snapshot with `Dup` (a copy into the current arena) when `may_move(body, s's place)`, otherwise a plain load. The loop is a `while` over a byte position `pos`:
      - `n = CharLen(snap, pos)`;
      - `c = Piece(snap, pos, n)`;
      - `i` counts characters;
      - `pos += n` (and `i += 1`) on every path back to the condition, `continue` included. Use the scope mechanism or put the step where `continue` lands.
  - **Runtime:** UTF-8 decoding by the lead byte. The input is valid UTF-8, since strings come only from source text, interpolation and the runtime.

- [ ] **Step 5: Run the tests until they pass,** then run the full suite and clippy.

- [ ] **Step 6: Commit:** "Give strings a length, indexing, character loops and `+`".

### Task 4: String methods and `join`

**Files:**
- Modify:
  - `check/src/body/strings.rs`;
  - `check/src/body/lists.rs`: `join` is a list and array method, since `check_method_call` sends those receivers to `collection_method`;
  - `kir/src/lower/strings.rs`, `kir/src/lower/lists.rs`, `codegen/src/func/strings.rs`;
  - `runtime/src/string.rs` (split it if it passes about 400 lines: `string_find.rs` for searching, `string_make.rs` for building);
  - `codegen/src/rt.rs`;
  - `check/src/body/call.rs` `check_discarded_call`: "this only reads the string, so the line does nothing".
- Test: `check/tests/strings.rs`, `tests/programs/string_methods.kes`, `tests/programs/string_slice_oob.kes`, `tests/programs/string_split_empty.kes`, `tests/errors/string_methods.kes` (new), runtime unit tests.

**Interfaces:**
- Consumes: Task 3's `StrMethod`, `StrOp`, `KStr`, string dispatch and `current()` region rule.
- Produces (runtime). Each string is a `ptr, len` pair, and every name is `kestrel_rt_` plus the name here:
  - `str_find(s, sub) -> i64`: a character index, or -1; `find("")` is 0.
  - `str_starts(s, sub) -> i8` and `str_ends(s, sub) -> i8`. `has` is `find != -1`.
  - `str_slice(s, from: i64, to: i64, line: i32, file, out)`: a new string. Panics "slice {from} to {to} is out of bounds for a string of {chars} characters".
  - `str_trim(s, out)`: a new string, trimming Unicode whitespace.
  - `str_case(s, upper: i8, out)`: a new string, using Rust's Unicode case mapping.
  - `str_join(parts: *const KStr, count: i64, sep, out)`: a new string.
  - `str_split(s, sep, line: i32, file, out: *mut List)`: a new `list[string]`, each piece copied, all in the current arena. Fill the list header the way `finish_list` does. `"".split(",")` is `[""]`. Panics with the spec's message for an empty separator.
  - `str_to_int(s, out: *mut i32) -> i8` and `str_to_float(s, out: *mut f32) -> i8`, 1 when parsed:
    - trim first, then an optional sign and digits;
    - `to_float` also allows a fraction and an exponent;
    - reject `inf`, `nan`, `infinity`, any non-finite result, overflow, hex and `_`.

Steps:

- [ ] **Step 1: Write the failing program test** `tests/programs/string_methods.kes`:

```
// expect: a, b, c|a-b|
// expect: ["x", "y", "", "z"] 4
// expect: true false true true false
// expect: 2 none 1
// expect: ll|é|
// expect: HÉLLO héllo
// expect: [hi]
// expect: 42 -7 none none
// expect: 2.5 none none
// expect: [""] 0 true true
main() {
    let parts = ["a", "b", "c"]
    let l: list[string] = ["a", "b"]
    let empty: list[string] = []
    let j1 = parts.join(", ")
    let j2 = l.join("-")
    let j3 = empty.join("-")
    print("{j1}|{j2}|{j3}")
    let sp = "x,y,,z".split(",")
    print(sp, sp.len)
    let s = "héllo"
    print(s.has("ll"), s.has("z"), s.starts_with("hé"), s.ends_with("lo"), s.ends_with("x"))
    print(s.find("ll"), s.find("z"), s.find("é"))
    let a = s.slice(2, 4)
    let b = s.slice(1, 2)
    let c = s.slice(3, 3)
    print("{a}|{b}|{c}")
    print(s.upper(), "HÉLLO".lower())
    let tr = "  hi \n".trim()
    print("[{tr}]")
    print("42".to_int(), " -7 ".to_int(), "4x".to_int(), "".to_int())
    print("2.5".to_float(), "nan".to_float(), "1e999".to_float())
    print("".split(","), "".find(""), "abc".has(""), "abc".starts_with(""))
}
```

  Add these, each with `// expect-exit: 101`:
  - `tests/programs/string_slice_oob.kes`: `// expect-stderr: slice 2 to 9 is out of bounds for a string of 5 characters on line N`;
  - `tests/programs/string_split_empty.kes`: `// expect-stderr:` with the spec's empty-separator message.

- [ ] **Step 2: Write the failing error test** `tests/errors/string_methods.kes`:
  - `[1, 2].join(",")`: "`join` needs a list of strings, found `[int]`"
  - a discarded `s.has("a")`: "this only reads the string, so the line does nothing"
  - `s.slice(1)`: "`slice` takes two values" (in the style of `args_problem`)
  - `s.find(1)`: the usual type mismatch for a string argument
  - `var keep = ""` then `arena { keep = "a,b".split(",")[0] }`: the arena message

- [ ] **Step 3: Run them and see them fail.**

- [ ] **Step 4: Implement:**
  - **Checker:** argument checks; result types from the spec's table; every result's region is `current()`.
  - **KIR and codegen:** extend `StrOp`. `has`, `find`, `to_int` and `to_float` build their bools or optionals from the runtime result. `split` writes a list header into `dest`.
  - **Runtime:** the functions above, with unit tests for each, including non-ASCII text and empty inputs.

- [ ] **Step 5: Run the tests until they pass,** then run the full suite and clippy.

- [ ] **Step 6: Commit:** "Add string methods: join, split, has, find, slice, case, trim and number parsing".

### Task 5: `draw.fill` and the docs

**Files:**
- Modify:
  - `kestrel/driver/std/draw.kes`: add `pub fill(r: Rect, color: Color) { rect(r.pos, r.size, color) }`.
  - `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md`: the types table's `string` and `list[T]` rows, plus a short "Lists and strings" section listing the operations and `remove b`.
  - `README.md`: in the language tour, one small example with a list and `remove b`, and one with strings.
  - `docs/superpowers/specs/2026-10-10-window-design.md`: add `draw.fill` to the draw API list.
  - `docs/superpowers/specs/2026-10-17-lists-strings-design.md`: add an "As built" section with the test count, anything that differs from the design, and the tasks' own rulings.
  - `docs/superpowers/plans/2026-10-09-core-2a-followups.md`:
    - remove "list and string basics" from the open items;
    - add the deferred minor findings the controller hands you;
    - add a follow-up for string views as an optimisation.
- Test: a new `tests/programs/draw_fill.kes`:
  - `// env: KESTREL_HEADLESS=1`;
  - the usual `window.open`/`window.frame()` loop, with `draw.fill(draw.Rect(pos: vec2(1, 2), size: vec2(3, 4)), draw.WHITE)` inside it;
  - an expect line `draw.rect vec2(x: 1, y: 2) vec2(x: 3, y: 4) Color(r: 1, g: 1, b: 1, a: 1)`.

  Copy the frame-loop shape from `tests/programs/rects.kes`.

**Interfaces:**
- Consumes: everything above.

Steps:

- [ ] **Step 1: Add the failing headless test** and see it fail.
- [ ] **Step 2: Add `fill` to `draw.kes`.** The test passes.
- [ ] **Step 3: Update the docs** listed above. Check every new example in them by running it (`cargo run -q -p kestrel-cli -- run` on a scratch file, headless for window code).
- [ ] **Step 4: Rebuild the Godot add-on** with `kestrel/godot/build.ps1` (Godot must not be running), and run its headless test (see the handoff's "Environment and gotchas"). Report the ok and fail counts. Run clippy in `kestrel/godot`.
- [ ] **Step 5: Run the full suite and clippy, then commit:** "Add draw.fill and document list and string basics".
