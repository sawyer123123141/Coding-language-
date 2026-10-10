# `map[K, V]` design

Status: decided. The user chose the syntax (the Types table entry of 2026-10-10, plus the `{}` literal and `get` below); the rest are Claude's rulings, recorded here.

## What it looks like

```
var stock: map[string, int] = {}
let prices = {"sword": 30, "bow": 12}
stock["sword"] = 3
stock["sword"] = stock.get("sword", 0) + 1
let n = stock["bow"] else { return }      // m[k] is an int?
if stock.has("bow") { stock.remove("bow") }
print(stock.len, prices)                  // 1 {"sword": 30, "bow": 12}
for name, price in prices { print("{name}: {price}") }
for name in prices { print(name) }        // keys only, like GDScript
```

| Form | Meaning |
|---|---|
| `map[K, V]()` | an empty map |
| `{}` | an empty map; the type comes from context (`var m: map[K, V] = {}`, a field type, a parameter) |
| `{k: v, ...}` | a map with these entries, in this order; without context, types come from the first entry |
| `m[k]` | `V?`: the value, or `none` when `k` is missing |
| `m[k] = v` | adds `k`, or replaces its value |
| `m.get(k, d)` | the value, or `d` when `k` is missing (`d` is always evaluated, like any argument) |
| `m.has(k)` | `bool` |
| `m.remove(k)` | removes `k`; `true` when it was there |
| `m.len` | the number of keys |
| `for k, v in m` | each key and value, in insertion order; `v` changes the map's value when `m` can change |
| `for k in m` | each key |

## Rules

- **Key types** are the types with `==`: integers (`int`, `i64`, `u8`), `bool` and `string`. This adds `==` and `!=` for strings. Enums, structs and vectors become key types when they get `==`. Floats never do (`0.1 + 0.2` vs `0.3`, and NaN). Error: "map keys must be integers, bools or strings; `float` keys would be unreliable".
- **Value types** are anything that doesn't own an arena. A struct with an `Arena` field can't go in a map yet: "maps can't hold values that own an arena yet (`Chunk` has one); keep them in a list and map keys to indexes".
- **Order** is insertion order, like GDScript's `Dictionary`. Replacing a value keeps its place; removing a key and adding it again puts it last. `print` shows `{"sword": 30, "bow": 12}`, and `{}` when empty.
- **Values are copied in and out**, like list elements: `m[k]` and `m.get` give a copy (a deep copy when `V` holds a list), and a whole map copies deeply on `let m2 = m`. To change a value in place, loop with `for k, v in m`, or read, change and write back.
- **`m[k]` can't be changed in place**, since it may be missing. `m[k] += 1` and `m[k].hp -= 1` are errors that say to use `get` or read-change-write.
- **Adding a key while looping over that map panics** at the change: "can't add a key to a map while looping over it (line 12); collect the keys in a list and change the map after the loop". Adding may move the entries; nothing else does. Removing keys is fine, including the current one (`for k, v in m { if v == 0 { m.remove(k) } }`), and later removed entries are skipped. Replacing a value, `has`, `get` and reading are fine too. The loop locks the map when it starts and unlocks it on every way out. (Ruled 2026-10-09 while implementing: the first draft also forbade removing, but removal only marks an entry dead, so forbidding it bought nothing.)
- **Memory:** a map's memory lives in the arena that was current when it was made, like a list, and grows there. The escape check applies to what is stored in it. Removed entries are reclaimed in place, so a long-lived map with churn doesn't grow.
- **`pure`** functions may make and change local maps.
- **Not yet:** `==` on maps, maps across the Godot bridge (a clear load error), owning values, struct and enum keys.

## How it works

### Memory layout

The map value is a 64-byte header; the entries and the index table live in its arena.

| Offset | Field | |
|---|---|---|
| 0 | `entries` | pointer to `cap` entries |
| 8 | `count` | live keys; at the same offset as a list's length, so `Len` works unchanged |
| 16 | `used` | entries used, including removed ones |
| 24 | `cap` | entries allocated |
| 32 | `slots` | pointer to the index table (`i32` per slot) |
| 40 | `slot_count` | a power of two, at least twice `cap`; 0 before the first insert |
| 48 | `locks` | loops running over the map |
| 56 | `arena` | where the map's memory lives (null: the program arena) |

An entry is `hash: u32` at 0, `live: u8` at 4, the key at 8, then the value at `val_off = round_up(8 + key_size, align(V))`. Entry alignment is `max(8, align(V))` and the size rounds up to it. Keys are 1, 4, 8 or 16 bytes (a string is pointer and length).

The index table uses open addressing with linear probing: -1 is empty, -2 a removed key, and anything else an entry index. With at most `cap` entries in `2 × cap` slots, a probe always ends.

Inserting into a full `entries` array either compacts it in place (when at least half the used entries are removed) or doubles `cap` into new arrays in the map's arena. Either way the table is rebuilt. Compaction never allocates, which is what keeps churn from growing memory. It only happens on inserting a new key, which can't happen during a loop over the map.

### Runtime (`runtime/src/map.rs`)

Each call takes the key's address and kind (0: 1 byte, 1: 4 bytes, 2: 8 bytes, 3: string) plus the entry size and alignment.

- `kestrel_rt_map_find(map, key, kind, entry_size) -> *mut u8`: the entry, or null.
- `kestrel_rt_map_insert(map, key, kind, entry_size, entry_align, line) -> *mut u8`: the entry for `key`, adding a zeroed one when missing. It panics when the map is locked and the key is new.
- `kestrel_rt_map_remove(map, key, kind, entry_size) -> i8`: 1 when removed. It only marks the entry dead, so it works while the map is locked.
- `kestrel_rt_map_dup(map, entry_size, entry_align)`: replaces the map's arrays with a compact copy in the current arena, and resets `locks` (a copy isn't being looped over).
- `kestrel_rt_str_eq(a_ptr, a_len, b_ptr, b_len) -> i8`.

Hashes: integers are multiplied by a 64-bit odd constant and the high half is kept; strings use FNV-1a. Neither needs to resist attacks.

### Check

- `Ty::Map(ElemId, ElemId)`. `resolve_type` and `check_type_call` accept `map[K, V]`, and both validate key and value types.
- New AST node `ExprKind::MapLit(Vec<(Expr, Expr)>)`, parsed when `{` starts an expression (newlines inside are skipped).
- `m[k]` reads as `V?` (`check_index`). As an assignment target it is a map set (`check_assign`). Compound assignment, and field or index changes through it, are errors.
- `get`, `has` and `remove` are collection methods. `remove` changes the receiver, like `add` (`Res::MapRemove`, with `changed_inside` updated); so does `m[k] = v`.
- `for k, v in m` / `for k in m`: `k` is a read-only copy, `v` a view like a list element.
- `==` / `!=` on two strings give `bool`.

### KIR

- `KTy::Map(ElemIdx, ElemIdx)`, 64 bytes, align 8.
- New expressions:
  - `MapNew`, `MapLit(Vec<(Expr, Expr)>)`
  - `MapGet(Place, key)`: an optional
  - `MapHas(Place, key)`, `MapRemove(Place, key)`
  - `MapUsed(Place)`, `MapLive(Place, at)`: false when `at` is past `used`
  - `MapLock(Place)`: increments `locks`, returns the old value
- New statements: `MapSet(Place, key, value)` and `MapUnlock(Place, saved)`.
- New projections: `Proj::MapKey { at }` and `Proj::MapValue { at, checked }`. A checked value panics when `at` is past `used`; that happens only when the body replaced the whole map with a smaller one.
- `m.get(k, d)` lowers to a `MapGet` into a temporary and a tag test.
- A for-in over a map lowers like one over a list:
  ```
  saved = MapLock(m)
  for i from 0 to MapUsed(m) {
      if MapLive(m, i) { k = m.MapKey(i); bind v = m.MapValue(i); body }
  }
  MapUnlock(m, saved)
  ```
  The unlock is the cleanup of a scope around the loop, so `break` (which lands after the loop) and `return` (which runs every cleanup) both unlock. When the body may replace the map, `v` re-indexes on every use with a checked `MapValue`, as lists do.
- Copies: `Dup` on a map calls `map_dup`; when `V` holds heap memory, a fixup loop deep-copies each value. Printing gets a generated print function. A map is never owning (v1), so it has no drop.

### Codegen

`func/maps.rs` (new) lowers the map expressions and statements. Keys are evaluated into a stack slot, and its address goes to the runtime. `MapGet` copies the value bytes into the optional's payload. String `==` evaluates both strings into slots and calls `str_eq`. The exhaustive matches (`cl_type`, `bits`, `print_scalar`, `export_ty`) get map arms; `export_ty` returns "a map".

## Testing

- `tests/programs/maps.kes` covers each operation, order, overwrite and re-add order, `get`, `remove`'s result, `print` (nested in lists and structs, string keys quoted), copies being independent, maps in struct fields, `pure` helpers, `arena { }`, and growth past several thousand keys with churn.
- `tests/programs/map_loop_add.kes` checks the loop-lock panic and its line; `map_loops.kes` checks removing during a loop and replacing values.
- `tests/errors/maps.kes` covers key and value type errors, `{}` without a type, `m[k] += 1`, `m[k].x = 1`, `var` misuse, the escape check, and string `==` type errors.
- Unit tests: parser (map literals, multi-line), checker, KIR for-in shape, and runtime tests in `map.rs` (find, insert, remove, compaction keeping order, growth, string keys).
- A small benchmark against C# `Dictionary` (int keys and string keys) records results honestly.
