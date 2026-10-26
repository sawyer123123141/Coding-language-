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
for the parity ones: 10 to 11 ms against 14 to 15 ms; see "Startup" below), and the range is min to max of the 11. The one-thread rows run with
`KESTREL_THREADS=1`. The script runs at idle priority.

**Machine and load.** Windows 11, 16 logical cores. The machine was in use (the user gaming): CPU
sampled for 1 s before each round, without this run, was a mean of 38% and a max of 44% (the first
run, below, 37% and 49%). Idle priority means the other load wins, so the ranges are wide.

**Two runs.** The script was run twice. The first run (an earlier shape of `parity.kes`, Finding 1)
gave world medians in ms, checks on / `--release`: frozen 580 / 500, PARITY_ITEMS 851 / 850,
PARITY_GET 905 / 865, PARITY_REUSE 542 / 527, par for 1 thread 578 vs 578 / 515 vs 533, all threads
98 vs 102 / 89 vs 94 (frozen vs parity). The table below is the **second run**, of the code as
committed. The frozen rows moved by 1.4% (checks on) and 3% (`--release`) between the runs, so
about 3% is the noise level of a median here.

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

Against the frozen copy, the gap of the world time (startup subtracted) and, in brackets, of the
whole run (startup not subtracted, so it includes parity's ~4 ms slower start):

| Variant | checks on | --release |
|---|---:|---:|
| PARITY_ITEMS | +0.5% (+7 ms, +1.2%) | +3.1% (+20 ms, +3.8%) |
| PARITY_GET | +8.7% (+55 ms, +9.2%) | +5.8% (+34 ms, +6.5%) |
| PARITY_REUSE | +0.3% (+6 ms, +1.0%) | +2.1% (+15 ms, +2.9%) |
| par for, 1 thread | +1.4% (+12 ms, +2.0%) | +4.2% (+26 ms, +4.9%) |
| par for, all threads | -4% (0 ms, 0%) | +5.5% (+9 ms, +8.9%) |

The rows near or over 5% are the `par for` ones in `--release` (1 thread +4.2%, all threads +5.5%;
the all-threads ranges overlap, 84 to 115 against 82 to 105 ms, and a 9 ms gap on a 100 ms run is
inside the noise of a shared machine) and `PARITY_GET`, which is the generic method on purpose.
The `par for` rows run the `PARITY_ITEMS` work, so their gap is the same few percent, seen through
a noisier measurement; they were not given a separate verdict.

### Startup

The parity exes start about 4 ms slower than the frozen ones (10.9 against 15.2 ms for the two
empty programs, median of 21 in a separate probe). The cause is `blocks.kes`, which `parity.kes`
imports for `SIZE`, `PAD` and `LAYERS`: it has `use { Color from draw }`, and that links the
drawing library into the exe. A two-line program that only imports `Color from draw` is 665,600
bytes and starts in 14.8 ms, against 162,816 bytes and 10.4 ms for one that does not
(`rel_parity_empty.exe` is 785,920 bytes against 224,256 for the frozen empty program). So the 4 ms
is the engine's `draw` import, not generics; the table subtracts each family's own startup. Taking
the whole-run gap instead adds about 4 ms (0.8%) to each parity row.

**Verdict: `PARITY_ITEMS` --release is 3.1% slower than the frozen copy (+3.8% of the whole
run), inside the 5% target. That depends on the sampling and the meshing sharing one function,
`mesh_items`, as the frozen `mesh` does with `solids`: split into two, the same code ran +70% slower
because of the compiler's inlining budget, not because of generics (Finding 1).**
The ranges overlap (524 to 580 against 511 to 528 ms: one slow round in 11 in the parity row), so
the gap is real but small. `PARITY_REUSE` (+2.1%) is 5 ms faster than `PARITY_ITEMS` in release:
making the grid per chunk (39,304 `add`s into a list, 256 times) costs about 1% in the full
program, and the rest of the gap (+2.1%) is probably the sampling and meshing code, where the
length of a `list` is unknown to the prover (below; not isolated). `PARITY_GET` is 2.6% (release) slower than the `items`
line: the generic method call costs a few percent at most.

### What the checks report says

Commands (in `kestrel/`; `run_parity.ps1` runs them at its end and prints the totals):

```
target/release/kestrel.exe check bench/voxels3d/main.kes --report [--release]
target/release/kestrel.exe check voxel/parity_main.kes --report [--release]
```

Totals the reports give (`bench/engine/out/report_*.txt`):

| | checks | removed | left |
|---|---:|---:|---:|
| frozen `main.kes`, `--release` | 8 | 7 | 1 |
| `parity_main.kes`, `--release` | 19 | 4 | 15 |
| frozen `main.kes`, checks on | 339 | 318 | 21 |
| `parity_main.kes`, checks on | 1,209 | 1,067 | 142 |

The totals are not comparable (the parity entry also compiles the library's `generate`, noise and
the other variants, which the frozen program has no counterpart for), so the functions that do the
work:

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

   **Is it general?** Yes, checked on the frozen program itself (copies of `voxels3d.kes` and
   `main.kes` in `bench/engine/out/split*` and `unsplit*`, `--release`, the frozen folder
   untouched). The split copy has `chunk_faces` call `solids` and pass the array to `mesh(solid,
   cx, cy, cz)`, instead of `mesh` calling `solids`.
   - One `bench(1337, 8)` in `main`: split 627 ms (615 to 639) against unsplit 615 ms (614 to 622),
     5 rounds in turn, +2%; the report has no inlining message. A single split does not hurt.
   - `main` calling `bench` for three seeds: split **1,980 ms (1,918 to 2,132) against unsplit
     1,570 ms (1,551 to 1,669), +26%**, and the split report has 16 messages such as "`hash` isn't
     inlined: the inlining budget of `main` ran out" and "`lattice` isn't inlined: the inlining
     budget of `main` ran out", while the unsplit one has none.
   So the budget is a general property of the compiler: a program that inlines the same
   sampling into several places in one root function (a world with more than one chunk caller, as
   an engine has) loses the noise inlining when the sampling is its own function. It is not
   specific to generics or to `Grid`. The parity entry hit it with three variants in one `main`.
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
