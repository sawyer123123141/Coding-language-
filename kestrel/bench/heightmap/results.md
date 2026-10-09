# Heightmap milestone benchmark

`heightmap.kes` is the core spec's milestone program:
- 16×16 chunks of 32×32 columns, each chunk with its own `Arena`
- three-octave value noise for the ground
- a `Block` enum matched per block
- a checksum over all 16.7M blocks
- a temporary `arena { }` that gathers every column height

`heightmap.rs` is the same arithmetic in plain Rust, and the outputs must match.

## 2026-10-10, before inlining

| Variant | Time |
|---|---:|
| Kestrel exe (checked) | 134 ms |
| Kestrel exe (`--release`) | 117 ms |
| `kestrel run` (JIT, incl. compile) | 142 ms |
| `kestrel check` (front end only) | 18 ms |
| Rust -O3 (overflow checks on) | 35 ms |
| Rust -O3 | 29 ms |

Output: `columns: 262144 / tallest: 57 / checksum: 205794432`

Kestrel is **about 4x** Rust (checked vs checked). The milestone target is 1.5x.

Where the time goes (release exe, median of 5):

| Variant | Time |
|---|---:|
| Full program | 127 ms |
| Checksum hand-inlined (no `block_at`/`weight` calls) | 90 ms |
| Checksum removed | 47 ms |

- **Calls: about 37 ms.** 33M calls to `block_at` (which returns an enum through memory) and `weight`. Rust inlines both. A KIR inliner for small functions is the fix.
- **The checksum loop itself: about 43 ms, even inlined.** That is roughly 8 cycles per block, with overflow-checked multiply and add, an if-chain, and `% 7` per block. Rust's whole program takes 29 ms, likely because the loop is vectorized. Closing this needs sub-project 5 (a stronger backend).
- **`kestrel run` meets the "< 50 ms including compile" goal only for smaller worlds.** Compile plus startup is about 15–20 ms (`kestrel check` is 18 ms), and the rest is run time.

## After the KIR inliner, 2026-10-10

Small non-recursive functions (at most 64 KIR nodes, with no `return` except a final one) are now inlined, including `var self` methods.

| Variant | Time |
|---|---:|
| Kestrel exe (checked) | 102 ms |
| Kestrel exe (`--release`) | 88 ms |
| `kestrel run` (JIT, incl. compile) | 112 ms |
| `kestrel check` | 17 ms |
| Rust -O3 (overflow checks on) | 31 ms |
| Rust -O3 | 28 ms |

Kestrel is now **about 3.2x** Rust (it was 4x).

What's left is backend quality:
- `block_at`'s enum still makes a round trip through a stack slot.
- Field loads aren't forwarded from stores.
- Nothing is vectorized.

Those belong to sub-project 5, as the milestone allows ("if missed, record the gap; it does not block the rest of sub-project 2").

The inliner also moved `particles` (2a suite) from 1.26x to **1.05x** Rust: its `step` method is now inlined.
