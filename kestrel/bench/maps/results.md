# Map benchmark: Kestrel `map` vs C# `Dictionary`

`maps.kes` has two workloads, and `cs/` is the same code in C# (.NET 9, Release, tiered PGO). Outputs match.

- **Integer keys:** 1M keys, 5 rounds. Each round adds every key, reads each back with `get`, removes every other one and tests each with `has`: 20M operations. Each round's map lives in an `arena { }`.
  - *In order:* key `i * 7 + r`.
  - *Scattered:* a one-to-one multiplicative scramble of `i` over a billion values.
- **String keys:** count 10M words drawn pseudo-randomly from 64 (`m[w] = m.get(w, 0) + 1`). That is 20M operations on a small map.

## Results

Median of 7 runs with process startup subtracted, two passes, after the review fixes:

| Workload | Kestrel (checks on) | Kestrel (`--release`) | C# .NET 9 |
|---|---:|---:|---:|
| Integer keys in order | 423–437 ms | 443–613 ms | 168–172 ms |
| Integer keys scattered | 446–583 ms | 395–440 ms | 310–319 ms |
| String keys | 321–330 ms | 339–344 ms | 290–307 ms |

Startup: Kestrel 9 ms, .NET 59–66 ms. Checks barely matter here; the work is in the runtime's hash table, which is always optimized.

**Kestrel loses on all three, by 10% to 2.5x:**
- **String keys: about 10% behind.** Both hash every string on every operation.
- **Scattered integer keys: about 1.3x behind.** Both tables are too big for the cache, so each operation waits on memory.
- **Integer keys in order: 2.5x behind, mostly from where the keys land.**
  - C# hashes an `int` to itself, so in-order keys fill in-order buckets, and the cache and prefetcher see a sequential walk.
  - Kestrel multiplies keys by a constant, which scatters them. That costs a cache miss per operation on a big map, but it keeps strided keys (all multiples of 1,024, say) from piling into one stretch of a power-of-two table.
  - Rust's and Go's maps scatter the same way. Keeping the scatter is the safer default; this result is the price.
- **Every operation is a call into the runtime.** On a map that fits in the cache, a lookup costs about 8 ns more than an empty loop, close to C#. Inlining lookups for integer keys into generated code would cut the call; that's in the follow-ups.

**What the first version taught (before the fixes):**
- The first runtime compared keys as byte slices (a `memcmp` call per compare) and probed twice per insert. Reading integers directly and reusing the first probe's slot took in-order integer keys from 1.8 s to 1.06 s.
- Both of those numbers were inflated: a code review was building and running tests on the same machine at the time. The table above is from a quiet machine, so how much each fix saved on its own isn't known.

**2026-10-10: strings became heap values.** Copying a string place now copies its bytes into the current arena, like an array.
- Kestrel only (C# not re-measured), median of 5 with startup subtracted, `kestrel.exe` built from the commit before and after, two passes each, run back to back. This machine was faster than for the table above, so compare only within this table.

| Workload | Before (checks on / `--release`) | After (checks on / `--release`) |
|---|---:|---:|
| Integer keys in order | 300–326 / 296–312 ms | 280–324 / 296–302 ms |
| Integer keys scattered | 350–358 / 314–328 ms | 325–326 / 321–338 ms |
| String keys | 320–321 / 324–326 ms | 430–437 / 430–435 ms |

- **Integer keys: unchanged.**
- **String keys: about 35% slower.**
  - `let w = words[...]` in the loop now copies the word (a runtime call and a few bytes of arena, 10M times).
  - The key in `counts[w] = ...` isn't copied: the checker checks its arena instead (see `check_map_set`).
- Both builds print the same result.

## Reproduce

```powershell
# from kestrel/bench/maps
./run.ps1
```
