# Engine benchmarks

Numbers for the voxel engine (`kestrel/voxel`), one section per measurement. Each says what was
measured, on what machine and under what load, with medians and ranges.

## Parity

Question (E19, E36, E42): does generic library code cost anything against the hand-written
frozen voxel world? The same 8 x 4 x 8 chunks (463,749 faces, seed 1337) are built by the frozen
`bench/voxels3d/main.kes` and by `kestrel/voxel/parity.kes`, which does the same work but keeps a
chunk's solid flags in a library `Grid[u8]` (a `list[u8]` inside, made by `grid[u8]`) instead of an
`array[u8]`. Everything else is the frozen code: the same noise (`gen.density`, bit-identical), the
same index maths and the same `Mesh.quad`, with each chunk's mesh in its own `arena { }`.

- `PARITY_ITEMS`: a fresh `Grid[u8]` per chunk, the mesher reads `g.items[i]` with the frozen
  code's own index arithmetic. This is the verdict line.
- `PARITY_GET`: the same through `g.get(x, y, z)`: what the generic method costs.
- `PARITY_REUSE`: one grid made once and `fill(0)`ed per chunk outside the arena.

**How it was measured.** `bench/engine/run_parity.ps1` (checks on and `--release` builds of the
frozen `main.kes`, `main_par.kes`, `main0.kes` and the five parity entry files; compiler
`target/release/kestrel.exe` at HEAD 473556b plus this task's files). All 32 exes run in turn,
11 rounds, so a busy spell hits every variant alike; the table is the median of 11 with the matching
empty program's median startup subtracted (frozen `main0.kes` for the frozen ones, `parity_empty`
for the parity ones: 10 to 11 ms against 14 to 15 ms, the parity exes link `draw` through
`blocks.kes`), and the range is min to max of the 11. The one-thread rows run with
`KESTREL_THREADS=1`. The script runs at idle priority.

**Machine and load.** Windows 11, 16 logical cores. The machine was in use (the user gaming): CPU
sampled for 1 s before each round, without this run, was a mean of 38% and a max of 44% (the first
run, below, 37% and 49%). Idle priority means the other load wins, so the ranges are wide; the
medians repeated between the two runs to about 2%.

| Variant | Mode | Whole run (median) | Startup | World (median) | World range (min to max) |
|---|---|---:|---:|---:|---:|
| frozen main.kes | checks on | 598 ms | 10 ms | 588 ms | 578 to 601 ms |
| PARITY_ITEMS | checks on | 605 ms | 14 ms | 591 ms | 579 to 615 ms |
| PARITY_GET | checks on | 653 ms | 14 ms | 639 ms | 631 to 658 ms |
| PARITY_REUSE | checks on | 604 ms | 14 ms | 590 ms | 572 to 603 ms |
| frozen par for, 1 thread | checks on | 595 ms | 10 ms | 585 ms | 574 to 601 ms |
| parity par for, 1 thread | checks on | 607 ms | 14 ms | 593 ms | 584 to 603 ms |
| frozen par for, all threads | checks on | 111 ms | 10 ms | 101 ms | 86 to 112 ms |
| parity par for, all threads | checks on | 111 ms | 14 ms | 97 ms | 92 to 127 ms |
| frozen main.kes | --release | 525 ms | 10 ms | 515 ms | 511 to 528 ms |
| PARITY_ITEMS | --release | 545 ms | 14 ms | 531 ms | 524 to 580 ms |
| PARITY_GET | --release | 559 ms | 14 ms | 545 ms | 534 to 554 ms |
| PARITY_REUSE | --release | 540 ms | 14 ms | 526 ms | 516 to 547 ms |
| frozen par for, 1 thread | --release | 528 ms | 10 ms | 518 ms | 509 to 527 ms |
| parity par for, 1 thread | --release | 554 ms | 14 ms | 540 ms | 527 to 566 ms |
| frozen par for, all threads | --release | 101 ms | 10 ms | 91 ms | 82 to 105 ms |
| parity par for, all threads | --release | 110 ms | 14 ms | 96 ms | 84 to 115 ms |

Against the frozen copy (median of the world time):

| Variant | checks on | --release |
|---|---:|---:|
| PARITY_ITEMS | +0.5% | +3.1% |
| PARITY_GET | +8.7% | +5.8% |
| PARITY_REUSE | +0.3% | +2.1% |
| par for, 1 thread | +1.4% | +4.2% |
| par for, all threads | -4% (noise) | +5.5% (ranges overlap, 84 to 115 against 82 to 105 ms) |

**Verdict: `PARITY_ITEMS` --release is 3.1% slower than the frozen copy, inside the 5% target.**
The ranges overlap (524 to 580 against 511 to 528 ms: one slow round in 11 in the parity row), so
the gap is real but small. `PARITY_REUSE` (+2.1%) is 5 ms faster than `PARITY_ITEMS` in release:
making the grid per chunk (39,304 `add`s into a list, 256 times) costs about 1% in the full
program, and the rest of the gap (+2.1%) is in the sampling and meshing code, where the length of
a `list` is unknown to the prover (below). `PARITY_GET` is 2.6% (release) slower than the `items`
line: the generic method call costs a few percent at most.

### What the checks report says

`kestrel check --report --release` of the frozen `main.kes` against `parity_main.kes`
(`bench/engine/out/report_*.txt`). The totals are not comparable (the parity entry also compiles
the library's `generate` and the other variants), so the functions that do the work:

| | frozen `mesh` (solids inlined) | parity `mesh_items` (sample inlined) | parity `mesh_get` |
|---|---:|---:|---:|
| `--release`: checks left | 1 | 4 | 8 |
| checks on: left (removed) | 20 (317) | 23 (309) | 75 (271) |

The frozen `mesh` has one bounds check left in release (the `out[i]` write in `solids`); `mesh_items`
has four: the same write (`g.items[i]` in `sample`, "`i` can be any `int`") and three reads in the
mesher (`g.items[i]`, `g.items[i + 1]` and `g.items[i + PAD * PAD]`, "`i` is 1191 to 38,112; nothing
shows it is below `g.items.len`"). The frozen mesher's reads are all proven, because its array's
length (`PAD * PAD * PAD`) is known in the same function; a `list`'s length is not known to the
prover, so `Grid[u8]` loses those proofs. That is probably most of the 2 to 3% (the checks are
cheap and predicted); it was not isolated. `mesh_get` has seven more (every `Grid.get` index, "`(...)` can be any `int`"). Adding
`@assume`s for the length would need a value the grid doesn't expose; see findings.

### Findings

1. **The shape of the code changed the speed by 60%, with no change in what it computes.** The
   first version of `parity.kes` had the sampling (`sample`) as its own function, called with the
   mesher from three variants in one chunk loop. `PARITY_ITEMS` --release then took **850 ms against
   500 ms (+70%)** (first run: GET 865, REUSE 527, frozen 500; checks on 851 against 580). The
   report said why: "`octave` isn't inlined: the inlining budget of `main` ran out" (and, in a
   second shape, "`sample` isn't inlined: the inlining budget of `main` ran out"). The compiler
   inlines small functions (`sample`, `fresh_faces`, `items_world`, `parity_faces`) into `main` and
   the budget is counted per root function, so the noise functions inside `sample` were left as
   calls. The fix, with no new syntax: make the function that is not inlined (it is over 256 nodes)
   the one that holds the sampling and the meshing together, as the frozen `mesh` does with
   `solids`: `mesh_items(var g, ...)` samples into `g` and then meshes. Then `octave` is inlined
   there, with that function's own budget. This is about library code in general: a helper that
   one function inlines can use up the budget its callees needed. That is a compiler issue worth a
   look (the controller decides): the budget could be per inlined callee, or a function over the
   size limit could not be charged to its caller. Nothing was changed in the compiler here.
2. A probe of `grid[u8](34, 34, 34, 0)` 256 times in an arena: 35 ms whole run (median of 7; min 33,
   max 173 with load), against 29 ms for the same loop written inline and 12 ms for
   `array[u8](39304)` (startup is about 11 ms of each). So making the grids with `add` costs about
   23 ms over `array` for all 256 chunks in isolation (about 4% of the world), 5 ms of it showing
   in the full program (ITEMS against REUSE); either way it is not what slowed the first version.
   A `grid` made by `add` in a loop is slower than `array[T](n)` because generic code cannot call
   `array[T](n)` (see `grid.kes`); a way to make a list of a given length would remove it.
3. `par` is a keyword: `let par = ...` is refused with "expected a variable name, found `par`".
4. The build writes nothing beside its source (checked `git status` of `bench/voxels3d` after the
   builds: clean); the exes are in `bench/engine/out/` (git-ignored).
