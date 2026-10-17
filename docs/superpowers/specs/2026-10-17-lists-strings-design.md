# List and string basics

Status: designed 2026-10-17. The user took every pick in the proposal ("sounds good", 2026-10-17), with strings counted in characters.

## Why

Lists have only `add`, `.len`, `xs[i]` and `for v in xs`. Strings have only `==`, `!=` and interpolation. The user's Breakout test had no way to remove a brick, so it used an `alive` flag. This design adds the everyday operations, named after GDScript where a GDScript name fits.

## Lists

`list[T]` gets these. Arrays (`[T]`) get only the read-only ones (`has`, `find`).

| Kestrel | GDScript | Meaning |
|---|---|---|
| `xs.remove_at(i)` | `remove_at` | Removes item `i` and keeps the order of the rest. An owning item (one holding an `Arena` or a GPU object) is dropped. |
| `xs.pop()` | `pop_back` | Returns the last item as a `T?` (`none` when the list is empty) and removes it. An owning item moves out; any other item is copied out, like `xs[i]`. |
| `xs.insert(i, v)` | `insert` | Puts `v` at index `i` and shifts the rest up. `i` may equal `xs.len`, which adds at the end. |
| `xs.clear()` | `clear` | Removes every item and drops owning ones. The capacity is kept. |
| `xs.has(v)` | `has` | `true` when an item `== v`. |
| `xs.find(v)` | `find` | The index of the first item `== v`, as an `int?` (GDScript returns -1). |

- `has` and `find` need item types that `==` compares: numbers, bools and strings. Otherwise: "`has` needs items that `==` can compare (numbers, bools and strings); `list[vec2]` holds `vec2`".
- An index outside the list, negative included, panics with the existing message ("index 5 out of bounds for length 2 on line N of f.kes"). For `insert`, `i == xs.len` is in bounds.
- `xs.insert(i, v)` evaluates `v` before shifting, so `xs.insert(0, xs[2])` inserts the old `xs[2]`.
- A popped item that isn't owning (a string, a list, a struct of those, a number) is a read: it is copied into the current arena, as `xs[i]` is, so it lives as long as the current arena, not the list. The list's memory may belong to an owner (`level.names`, built in `level.mem`) that is replaced or cleared while the item lives.
- An owning item (one holding an `Arena` or a GPU object) moves out of the list's memory, so its value has the list's lifetime. A kept `pop` of one is refused when the list is inside a value that holds an `Arena` (a field of such a struct, or reached through an element of a list of them), because the item's memory may be in that arena: "can't take an item out of `level.items` with `pop`: it may point into `level`'s arena", with the help "read what you need with `level.items[i]`, then use `remove_at`". A plain local or parameter list of owning items (`var ps: list[Pool]`) still moves.
- A `pop` whose value isn't kept drops the item at once.
- `remove_at`, `pop`, `insert` and `clear` change their receiver, so they need a `var` root, as `add` does, and they follow `add`'s rules (`par for` writes, `match` guards, exclusivity).
- A discarded `xs.has(v)` or `xs.find(v)` says the line does nothing, like a map read.
- **In a loop over the same list** (or one that overlaps it: the same variable, a field path that contains the other, or a loop item that is part of it, like `for row in grid` then `row.clear()` inside `for x in grid[0]`), `remove_at`, `pop`, `insert` and `clear` are refused: "can't change how many items `xs` has while looping over it; use `remove b` to remove the loop's item, or change `xs` after the loop". The loop reads the length once, so a shorter list would run past its end. `add` stays allowed: the loop doesn't see the added items.
- On an array: "arrays have a fixed length; use a `list[int]` if items come and go". `add` keeps its existing message.

### Removing the loop's item: `remove b`

```
for b in bricks {
    if hits(ball, b) {
        score += 10
        remove b
    }
}
```

`remove b` removes the current item of a `for b in xs` loop over a list. The loop compacts the list as it goes, in one pass: items keep their order, none is skipped, and removing k items from n costs O(n), not O(k·n). This is the case GDScript gets wrong when you remove while iterating.

- **Syntax.** `remove` is not a keyword: `remove` followed by a name, on a statement of its own, is this statement. `remove(x)`, `remove = 1` and a variable called `remove` keep working, and `m.remove(k)` is unchanged.
- **What it names.** The item of an enclosing `for ... in` loop over a `list`. Otherwise:
  - a name that isn't a loop item: "`remove` takes the item of a `for ... in` loop over a list; `x` isn't one"
  - a loop over an array: the array message above
  - a loop over a map: "`remove` works in loops over a list; for a map, use `m.remove(k)`"
  - a loop over a string: "`remove` works in loops over a list; strings can't change, so build a new one"
  - inside a `par for` (the loop itself or one around the statement): "`remove` can't be used in a `par for`: its iterations run at the same time"
  - a list that can't change: "can't remove from `xs`: it was declared with `let`" (with the same variants `add` has for parameters and `self`)
  - a loop over a temporary (`for b in make_bricks()`): "`remove` needs a loop over a list in a variable; store the list in a `var` first"
  - an item that a `match` arm's binding is reading: the existing "can't change `b` here: this `match` arm's `x` reads from it"
  - another loop around it that loops over the same list (or an overlapping one, as above): "can't remove from `xs` here: an outer loop is also looping over it"
- **After `remove b`, `b` is gone.** The checker treats it like a move of `b`, so:
  - a later use is "`b` was removed earlier", or "`b` may have been removed earlier" when only some paths removed it;
  - a second `remove b` is an error the same way;
  - assigning to `b` after removing it is refused too ("`b` was removed earlier"), unlike a moved `var`, which an assignment gives back: the removed slot is gone;
  - in a loop nested inside the `for b` loop, `remove b` must be followed by leaving that inner loop (`break`), or be on a path that leaves it, exactly like a move. The message is the loop-move message with "removed" for "moved": "`b` is removed inside a loop, so the next pass would remove it again; leave the loop with `break` right after removing it".
  - `remove b` itself doesn't jump: the rest of the body still runs, so `remove b` and then `continue` is the usual way to skip the rest.
- **The list during the loop.** While a loop that contains `remove` runs, its items move. So the body may not use the list except through the item: a use of the loop's list path (`xs`, `xs[j]`, `xs.len`, `xs.add(..)`, passing `xs`, or the whole root variable when the list is a field, like passing `world` while looping over `world.bricks`) is refused: "this loop removes items from `xs`, so its items move while it runs; use the loop's item (`b`), or use `xs` after the loop". Disjoint fields of the same root are fine (`world.score += 10` while looping over `world.bricks`).
- **Memory safety at run time.** Any other code that reaches the list during the loop through a `var` parameter or a global can't be seen by the checker. That code reads zeroed or shifted items but never freed memory: a removed item is dropped and its slot zeroed, a kept item is moved down and its old slot zeroed, and the final step tolerates a list whose length changed during the loop.
- **`for i, b in xs`** with `remove`: `i` counts the items as they were when the loop started.
- **Every way out of the loop finishes it.** At the end, and on `break`, `return` and `continue`, the current item is kept or removed, and on leaving the loop the unvisited tail is moved down and the length set.

## Strings

Strings are UTF-8. **Lengths and indexes count characters** (Unicode code points, as GDScript's `length()` does), not bytes: `"héllo".len` is 5.

| Kestrel | GDScript | Result |
|---|---|---|
| `s.len` | `length()` | `int`, the number of characters |
| `s[i]` | `s[i]` | the character at `i`, as a one-character `string` |
| `for c in s`, `for i, c in s` | `for c in s` | each character as a `string` (`i` is its character index) |
| `a + b`, `s += t` | same | a new string |
| `parts.join(", ")` | `", ".join(parts)` | one string, from a `[string]` or `list[string]` |
| `s.split(",")` | `split` | `list[string]` |
| `s.has("x")` | `contains`, `in` | `bool` |
| `s.starts_with("x")`, `s.ends_with("x")` | `begins_with`, `ends_with` | `bool` |
| `s.find("x")` | `find` | `int?`, the character index of the first match (GDScript returns -1) |
| `s.slice(from, to)` | `substr(from, len)` | the characters from `from` up to, not including, `to` |
| `s.upper()`, `s.lower()` | `to_upper`, `to_lower` | a new string (Unicode case rules) |
| `s.trim()` | `strip_edges` | `s` without whitespace at either end |
| `s.to_int()`, `s.to_float()` | `to_int`, `to_float` | `int?` / `float?`: `none` when the text isn't a number or doesn't fit (GDScript returns 0) |

- **Every string these return is new,** built in the current arena like interpolation: `s[i]`, `slice`, `trim`, `+`, `join`, `split` (its list and each piece), `upper`, `lower`, and each character of `for c in s`. Storing one into a longer-lived place needs `in x.mem { }`, as now. A loop like `for c in s` in a frame loop allocates a few bytes per character, as interpolation does; wrap the frame's body in `arena { }` to free them each frame.
- **`for c in s` walks `s` as it was when the loop started,** even if the body assigns a new string to `s`.
- **Empty inputs:** `"".split(",")` is `[""]` (one empty piece, as in GDScript); `s.find("")` is `0`; `s.has("")`, `s.starts_with("")` and `s.ends_with("")` are `true`; `join` of an empty list is `""`; `"".trim()` is `""`.
- **Cost of counting characters.** `.len`, `s[i]`, `slice` and `find` walk the string from the start (`s[i]` in a `for i` loop is O(n²) on long text). `for c in s` is the linear way through.
- **Errors at run time** (with "on line N of f.kes", like other panics):
  - `s[i]` out of range: "index 5 out of bounds for length 5"
  - `slice` with `from > to`, or either outside `0..len` (negative included): "slice 2 to 9 is out of bounds for a string of 5 characters"
  - `split("")`: "`split` needs a separator that isn't empty; to go through each character, use `for c in s`"
- **Errors at check time:**
  - `s[i] = x`: "strings can't be changed in place; build a new one with `+` or `\"{...}\"`"
  - `+` with a string and a non-string: "`+` needs two numbers or two strings, found string and int", with help "put a number in a string with `\"{n}\"`". `+=` on a string takes a string.
  - `join` on a list of non-strings: "`join` needs a list of strings, found `list[int]`"
  - an unknown string method: "string has no method `x`", with help "strings have `has`, `find`, `starts_with`, `ends_with`, `slice`, `split`, `upper`, `lower`, `trim`, `to_int`, `to_float` and `.len`"
  - an unknown string field: "string has no field `x`", with help "its length is `.len`"
  - `par for c in s`: the existing "`par for` loops over an array or a list"
  - a discarded string method call that only reads: "this only reads the string, so the line does nothing"
- `to_int` and `to_float` trim whitespace (Unicode whitespace, as `trim` does), then accept an optional sign and the digits (`to_float` also a fraction and an exponent). Anything else, including `inf` and `nan`, is `none`.
- Strings still compare only with `==` and `!=`.

## Drawing: `draw.fill`

`draw.fill(r: Rect, color: Color)` draws a filled `Rect`. It is written in Kestrel in `draw.kes` (`rect(r.pos, r.size, color)`), so it needs no new native. Its headless line, and an error it raises (no window open), say `draw.rect`.

## Not in this design

- Sorting, reversing, slicing lists, and `keep_if`/`map`/`filter`: they wait for closures, the next design.
- String comparison with `<`, string repetition, formatting numbers with a precision, a character type.
- Negative indexes from the end (GDScript allows `s.substr(-3)`).
- Removing from a map while looping over it: already allowed, with `m.remove(k)`.

## Rulings made on the user's behalf

What each costs if wrong:
1. **Characters are code points, not grapheme clusters.** An emoji built from several code points counts as several. Cost: such text splits oddly; graphemes would need Unicode tables in the runtime.
2. **`s[i]`, `slice`, `trim` and the characters of `for c in s` are copies** in the current arena, not views into `s`. The design first made them views; the spec review found six ways a view could outlive its bytes (returns from owning roots, `arena { }` blocks, loops whose string is freed by the body, later `var` arguments). Cost: a small allocation per read, and memory growth in a frame loop without `arena { }`. Views can come back later as an optimisation with a stricter region rule.
3. **`split` copies its pieces** into the current arena instead of pointing into `s`. Cost: an allocation per piece. Views inside a list would need the list's region tied to `s`'s.
4. **Length changes in a loop over the same list are refused** for the new methods. `add` stays allowed, as today. Cost: a program that wants to `insert` during a loop must use a `while` loop with an index.
5. **A loop that uses `remove` can't otherwise use its list.** Cost: reading `xs.len` inside such a loop needs a counter of its own.
6. **`remove b` doesn't jump;** the body keeps running with `b` gone. Cost: none; `continue` is one word.
7. **`draw.fill` is Kestrel code calling `draw.rect`,** so headless lines and its errors say `draw.rect`. Cost: a slightly confusing error when no window is open.
8. **Unicode case rules for `upper` and `lower`** (`"ß".upper()` is `"SS"`). Cost: a string's length can change.
9. **`to_int` and `to_float` reject `inf`, `nan`, hex and `_` separators.** Cost: none for normal input.
10. **`remove` and length changes are refused when any enclosing loop iterates an overlapping list,** not only the innermost one. Cost: nested loops over the same list must collect indexes and remove after.
11. **`"".split(",")` is `[""]`.** Cost: a program that expects `[]` must check for an empty string first.
12. **Assigning to a removed loop item is an error.** Cost: none; there is no slot to put it in.

## As built

Built 2026-10-17 in five tasks on `main` (commits 2281e7a, 58d5295, 3f81f0b, 2bd84b9, 9a7094d, 025677b and the last one that adds `draw.fill` and these docs). `cargo test --workspace --no-fail-fast` passes 681 tests and clippy is clean in `kestrel/` and `kestrel/godot`. The Godot add-on was rebuilt and its headless test passes: 31 ok, 0 failures. The program tests (`tests/programs/`, run as JIT and as exe) and error tests (`tests/errors/`) cover each operation, the panics, owning items, non-ASCII text and empty values. Everything in the design was built. What differs from it, or was added to it:

**Lists**
- **The overlap rule is one-directional.** `remove_at`, `pop`, `insert` and `clear` are refused only when the list they change is the looped list or contains it (the changed path is the start of the looped path; an index matches any index). `for row in grid { row.clear() }` and `for e in enemies { e.targets.clear() }` are fine, because they never change the length of the list being looped. `row.clear()` inside `for x in grid[0]` is refused, as the design says. A `match` binding or `let ... else` view counts as the place it reads, and a loop item counts as its list plus an index.
- **The refusal names the list as written up to its first index** (`xs`, `w.items`, `grid` for `grid[0]`) and the innermost overlapping loop's item in "use `remove x`".
- **Arrays** get `has` and `find`; `remove_at`, `pop`, `insert` and `clear` say "arrays have a fixed length; use a `list[T]` if items come and go". `add` keeps its old message. Unknown list methods get a help line listing the methods, `join` included.
- **KIR.** The list operations are `ListOp { RemoveAt, Insert, Clear, Compact }` and `Stmt::SetLen`. `pop` is built from existing nodes plus `SetLen`, and `has` and `find` call one generated search function per item type. Every one of them is reported as a write of its list, pinned in `scalarize.rs`, and handled in `visit_mut`, `rename` and `inline`.
- **Vacated slots are zeroed.** `clear` and `pop` zero what they vacate (as `remove_at` and `remove b` do), so no freed arena stays reachable from a spare slot.
- **`pop` copies an item that isn't owning** (the final review's fix). As first built, `pop` moved every item out and gave it the list's lifetime. That was unsound when the list's memory belonged to an owner that was then replaced or cleared (`level.names` built in `level.mem`, or `xs[3].tags` with `xs: list[Thing]`): the popped string read a later level's bytes. Now a non-owning item is copied into the current arena (`fixup`, as an indexed read does) and its slot zeroed, and its region is the current arena. An owning item still moves, and a kept `pop` of one is refused when any place the list is inside of (following a loop's item or a view to the place it reads) has an `Arena` field. A value that holds only GPU objects owns no arena memory, so an element of a list of owning items counts as an owner only when it has an `Arena` field. A discarded `pop` moves the item out and drops it at once, not at the block's end.
- **`KESTREL_ARENA_COUNT=1`** prints `arenas alive at exit: N` to stderr. It was added to prove that owning items are dropped exactly once. It counts user arenas only: the program arena, the `par for` workers' arenas and the arena for heap constants are left out (that one is built with `StaticArena`, a new KIR expression and runtime function).

**`remove b`**
- **Extra refusals** the design doesn't list:
  - `remove b` inside `in b.mem { }` ("can't remove `b` here: this `in` block is putting memory in `b`'s arena");
  - using a view of `b` (an inner loop's item over `b.parts`, a `match` binding) after `remove b` ("`h` reads from `b`, which was removed earlier");
  - `remove b` while an inner loop is removing from a list inside `b` ("the loop over `b.hits` is removing items from it; leave that loop first").
- **Uses of the list are refused in both directions** in a removing loop: `xs`, `xs.len`, `xs[0]` and `w` (when looping over `w.bricks`), not only a change that covers the looped list. Uses rooted at the item or its views are exempt.
- **Wording** where the design gave only a stem: "`b` was removed earlier, so it can't be used here" (and "may have been removed earlier"), "... so it can't be removed again", "can't remove from parameter `xs`: it isn't `var`", "can't remove from `self`: the method takes `self`, not `var self`".
- **`Tables::removing_loops` and `Tables::removes`** are the checker's result for lowering; `removes` is keyed by the statement's span.
- **One test program changed.** The design's `drop_two_stop_at_three` used `x` after a conditional `remove x`, which the design's own rule refuses, so its two `if`s are swapped (same output).
- **Not allowed:** `remove` in a `par for` over a chunk's own list would be safe but is refused (a follow-up).

**Strings**
- **String `+` is refused in constants and field defaults** ("a constant or field default can't join strings with `+`; write the text as one string"), as interpolation is, because the result is built in an arena at each use. String methods there were already refused (methods on non-enums aren't constant).
- **`+` chains build one string:** `a + b + c` and `s += t` lower to one allocation.
- **`s[i]`, `slice` and `s.len`'s index arguments** accept any integer type; a float says "an index must be an integer, found float". A negative index says "index -1 out of bounds for length N".
- **Messages for loops and indexing name strings now:** "... needs an array, list, map or string" and "... only arrays, lists, maps and strings can be indexed".
- **`to_float` accepts `.5` and `5.`** (at least one digit in the mantissa). `1e-999` (an `f32` underflow) gives 0 rather than `none`: "doesn't fit" means overflow.
- **`find`, `starts_with` and `ends_with` search bytes,** which is valid because UTF-8 is self-synchronising; the result is converted to a character index.

**Drawing**
- **`draw.fill`** is Kestrel in `draw.kes`, as designed. Its parameter is called `r`, and this showed a limit that was already there: a parameter of a function in `draw.kes` can't have the same name as a global of the user's program ("`r` is already defined; Kestrel doesn't allow shadowing"), because the shadowing check sees both. `color` has had the same effect since the window was built. The voxel demo's fly test had a helper called `r`, which now fails with `use draw`; the helper is renamed `round3` in `cli/tests/voxels_demo.rs`. A user program with a global `r` that uses `draw` would now fail the same way; see the follow-ups.

**Controller rulings** (Tasks 1-4): the overlap rule above; `ListOp` and `Stmt::SetLen` as above; `KESTREL_ARENA_COUNT` counts user arenas only; `clear` and `pop` zero what they vacate; the extra `remove b` refusals above; string `+` refused in constants and defaults; loop and index messages name strings; `to_float` accepts `.5` and `5.`.
