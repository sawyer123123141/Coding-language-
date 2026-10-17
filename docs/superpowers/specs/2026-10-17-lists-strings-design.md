# List and string basics

Status: designed 2026-10-17. The user took every pick in the proposal ("sounds good", 2026-10-17), with strings counted in characters.

## Why

Lists have only `add`, `.len`, `xs[i]` and `for v in xs`. Strings have only `==`, `!=` and interpolation. The user's Breakout test had no way to remove a brick, so it used an `alive` flag. This design adds the everyday operations, named after GDScript where a GDScript name fits.

## Lists

`list[T]` gets these. Arrays (`[T]`) get only the read-only ones (`has`, `find`).

| Kestrel | GDScript | Meaning |
|---|---|---|
| `xs.remove_at(i)` | `remove_at` | Removes item `i` and keeps the order of the rest. An owning item (one holding an `Arena` or a GPU object) is dropped. |
| `xs.pop()` | `pop_back` | Returns the last item as a `T?` (`none` when the list is empty) and removes it. An owning item moves out. |
| `xs.insert(i, v)` | `insert` | Puts `v` at index `i` and shifts the rest up. `i` may equal `xs.len`, which adds at the end. |
| `xs.clear()` | `clear` | Removes every item and drops owning ones. The capacity is kept. |
| `xs.has(v)` | `has` | `true` when an item `== v`. |
| `xs.find(v)` | `find` | The index of the first item `== v`, as an `int?` (GDScript returns -1). |

- `has` and `find` need item types that `==` compares: numbers, bools and strings. Otherwise: "`has` needs items that `==` can compare (numbers, bools and strings); `list[vec2]` holds `vec2`".
- An index outside the list, negative included, panics with the existing message ("index 5 out of bounds for length 2 on line N of f.kes"). For `insert`, `i == xs.len` is in bounds.
- `xs.insert(i, v)` evaluates `v` before shifting, so `xs.insert(0, xs[2])` inserts the old `xs[2]`.
- `pop` moves the item out of the list's memory, so its value has the list's lifetime, not the current arena's (it can't be stored somewhere that outlives the list). A `pop` whose value isn't kept drops it.
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
