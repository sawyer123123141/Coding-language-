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

## Engine

Question (E18b, E32): does the engine hold 60 fps while it streams, and what do generating,
meshing, uploading and editing cost? `kestrel/voxel/bench.kes` runs the play loop of `main.kes`
(stream, mesh and upload, draw, HUD) in a hidden window, driven by a script instead of the
keyboard, with a fixed simulation step of 1/60 s a frame, so every run does the same work.
A frame's **work time** is `window.ticks_usec()` from the frame's start to just before
`window.frame()`; the hidden window doesn't wait for the display (the wall time per frame is
about the work time), so a run takes about 100 s of real time for its 343 simulated seconds.
The phases: `fill` (stand at the start until the queue is empty and every chunk within `VIEW`
is meshed), `fly20` and `fly100` (20 s straight at 20 and 100 blocks a second, at height 100),
`edits` (25 chunk-middle and 25 chunk-corner cells, each broken and put back), `circle`
(300 s on a circle of radius 1,000 blocks at 50 blocks a second). Script details: spec E66.

**How it was measured.** `py measure.py` (in `kestrel/bench/engine`): builds `bench.kes`
`--release` (compiler `target/release/kestrel.exe` at 984481d plus this task's files), runs it
three times with `KESTREL_HIDDEN=1` at idle priority, and gives the median of the three runs'
values with their range (each run's value is itself a median, 99th percentile or worst over
the phase's frames). Raw lines: `out/measure_<n>.txt`; everything parsed: `out/measure.json`.

**Machine and load.** Windows 11, AMD Ryzen 7 5800X (8 cores, 16 threads), 16 GB, NVIDIA
RTX 3070. The user was away; nothing else heavy ran. The whole machine's CPU use, sampled for 2 s
before each run: 5.0%, 18.5% (a file copy of mine, finished before the run's phases) and 11.4%;
during each run, the machine's CPU time minus the benchmark's own: 8.2%, 9.1% and 7.9% (this
includes `measure.py` itself).

### The pass mark (E32): passes with `GEN_BATCH` 2, failed with the spec's 8

| Phase | frames | work median | **work p99** | work worst | generated | meshed |
|---|---:|---:|---:|---:|---:|---:|
| fill | 135 | 11.1 ms (10.4 to 11.3) | 14.1 ms (13.5 to 14.5) | 61 ms (47 to 93) | 269 | 788 |
| fly20 | 1,200 | 0.89 ms (0.88 to 0.92) | **15.0 ms (14.7 to 15.3)** | 16.3 ms (16.0 to 16.4) | 232 | 832 |
| fly100 | 1,200 | 10.3 ms (10.2 to 10.9) | **15.5 ms (15.5 to 16.2)** | 17.8 ms (16.5 to 18.7) | 1,193 | 4,268 |
| edits | 51 | 3.1 ms (3.0 to 3.2) | 7.3 ms (6.1 to 7.4) | 7.3 ms (6.1 to 7.4) | 0 | 450 |
| circle | 18,000 | 0.89 ms (0.85 to 1.46) | **15.8 ms (15.5 to 16.2)** | 20.5 ms (20.0 to 33.6) | 9,035 | 31,388 |

**E32 passes: the 99th-percentile work time is at most 16.7 ms in `fly20`, `fly100` and
`circle` in every run (worst run 15.3, 16.2 and 16.2 ms).** The margin is small (0.5 ms in the
worst run of `fly100` and `circle`), and the worst frames are over: 16.3, 17.8 and 20.5 ms
(medians), once 33.6 ms. The fill's worst frame (47 to 93 ms) is the first upload (the driver's
first `make_mesh`), not streaming. Wall time per frame: median 0.92, 10.4 and 1.68 ms in the
three flights (no vsync in a hidden window).

That is with **`GEN_BATCH` = 2** (spec E65). With the spec's starting value of 8 (the same
script, three runs of the code before the change; load before 6.7 to 9.5%, others during 7.0 to
8.7%), **E32 failed**: p99 **17.5 ms (16.8 to 17.9)** in `fly20`, **19.9 ms (19.6 to 19.9)** in
`fly100` and **19.7 ms (18.2 to 19.8)** in `circle`; worst 18.7, 21.8 and 25.5 ms; the fill took
51 frames (718 to 760 ms of real time). Why, from a copy of the bench timing each part of the
frames over 14 ms (`GEN_BATCH` 8): generating 8 columns took 11.5 to 11.9 ms, copying them into
the pool and queueing their chunks 2.3 ms (`finish_jobs`: the item-by-item copy, 8 x 131,072
items; see the findings), meshing 16 chunks about 1 ms and uploading them 2 to 3 ms, drawing
0.8 to 1.3 ms. One column takes **7.48 ms** to generate on one core (`KESTREL_THREADS=1`: 7.47
to 7.49 ms a column in every phase), so a frame that generates anything costs at least that;
8 at once on 16 hardware threads took 1.55 times one column's time (they share cores and
memory bandwidth). Batch sizes tried (circle cut to 100 s, two runs each; the p99 of each run):

| `GEN_BATCH` / `MESH_BATCH` | fly20 p99 | fly100 p99 | circle p99 | fill frames |
|---|---|---|---|---:|
| 8 / 16 (spec) | 15.6, 17.3 | 20.1, 21.8 | 23.5, 24.3 | 51 |
| 6 / 16 | 17.4, 14.7 | 19.2, 19.4 | 21.4, 19.2 | 53 |
| 4 / 16 | 13.8, 13.2 | 17.9, 18.0 | 18.0, 18.2 | 68 |
| 4 / 8 | 11.7, 12.0 | 16.7, 16.8 | 16.5, 16.7 | 102 |
| 3 / 16 | 16.0, 16.2 | 17.3, 17.3 | 17.2, 17.2 | 90 |
| 3 / 8 | 15.7, 15.2 | 16.4, 16.4 | 16.3, 16.4 | 105 |
| **2 / 16 (picked)** | 14.8, 14.9 | 15.5, 15.6 | 15.4, 15.6 | 135 |
| 2 / 8 | 14.7, 14.8 | 15.5, 15.7 | 15.4, 15.5 | 135 |

2 / 16 is the largest generation batch that passed with room to spare (3 / 8 passed by 0.3 ms);
a smaller mesh batch didn't help at 2. **The cost of the pick: the view fills in 135 frames
instead of 51**, 1.45 to 1.57 s of real time here (2.25 s at 60 fps) instead of 0.72 to 0.76 s,
and generation is capped at 120 columns a simulated second (flying at 100 blocks a second needs
about 60). In `fly100` nearly every frame now generates (median work 10.3 ms instead of 1.0 ms).
The underlying limit is that one column is one job on one core (E35); splitting a column's
generation across cores, or spreading it over frames, would let a frame generate without paying
a whole column (see the findings).

### Generate, mesh and upload

Per-frame medians over the frames that did some. A batch runs on all cores, so these are the
batch's time shared out, not one item's time on one core (those are below):

| | fill | fly20 | fly100 | circle |
|---|---:|---:|---:|---:|
| generate, ms per column | 3.90 (3.84 to 3.96) | 4.03 (4.01 to 4.29) | 5.24 (4.71 to 5.36) | 5.85 (4.97 to 5.86) |
| mesh, ms per chunk | 0.127 (0.112 to 0.132) | 0.085 (0.084 to 0.096) | 0.086 (0.084 to 0.098) | 0.090 (0.090 to 0.091) |
| upload, ms per chunk | 0.190 (0.156 to 0.191) | 0.143 (0.141 to 0.161) | 0.147 (0.145 to 0.164) | 0.100 (0.096 to 0.154) |

On one core (`KESTREL_THREADS=1`, `GEN_BATCH` 8, one run): **7.48 ms a column** (131,072
blocks, about 57 ns a block, three octaves of 3D noise each) and **0.25 ms a chunk** to mesh.
Upload (`make_mesh`) is on the main thread either way: 0.10 to 0.19 ms a chunk. A chunk has
1,866 faces on average (1,470,349 faces in the 788 chunks of the filled view, from the headless
smoke run): 7,464 vertices and 11,196 indices.

**Columns per simulated second:** 11.6 at 20 blocks a second, 59.7 at 100, 30.1 on the circle
at 50 (the same in every run: the script fixes the work).

**Time to fill the view from a standing start:** 135 frames, **1,538 ms of real time (1,455
to 1,568)** in a hidden window that doesn't wait for vsync (with `GEN_BATCH` 8: 51 frames, 718
to 760 ms). At 60 fps that is 2.25 s (0.85 s with 8).

**Edit latency** (from `edit_at` to the upload of every chunk it dirtied, in the same frame;
50 breaks and 50 places of each kind; medians): **chunk middle 0.71 ms (0.70 to 0.77), 1 chunk
meshed again; chunk corner 1.82 ms (1.69 to 1.86), 8 chunks**. Every edit showed in the frame it
was made.

## Memory

Question (E31, E40): does the engine stop allocating once warmed up?

**The program arena is the same at both flight lengths (E31 passes).** `measure.py` also builds
`bench.kes` with the circle cut to 100 s (`out/bench_c100.exe`) and runs both with
`KESTREL_ARENA_REPORT=1` (the runtime reports the program arena only at exit): **60,739,584
bytes after a 100 s circle and 60,739,584 bytes after the 300 s one** (load before 6.3 and
6.2%, others during 6.5 and 6.7%), and the same in the three timed runs. 58 MB of it is the
pool's 441 column grids (E9); the rest is the world's and the renderer's lists, the two
generation jobs' grids, the bench's sample buffers and the map. A first measurement differed
(62,083,072 against 63,066,112 bytes): the bench sized its sample buffers from the circle's
length, so the two builds had different buffers; they are now sized for 18,000 frames in both
(E66). Before that fix a headless run of the 300 s build already read 63,066,112 bytes at 2,600,
6,000, 12,000 and 18,000 frames, so nothing grew during the flight.

**The working set rises less than 5% over the circle's last 200 s (E31 passes).** Sampled every
250 ms; "circle 100" is the last sample before the `mark 100` line (MB):

| | start | after fill | circle 0 | circle 100 | circle 200 | circle 300 | last 200 s: min / max | rise |
|---|---:|---:|---:|---:|---:|---:|---|---:|
| run 1 | 12.9 | 454.0 | 500.0 | 538.8 | 547.6 | 548.9 | 524.1 / 550.4 | 2.16% |
| run 2 | 1.8 | 470.1 | 501.7 | 529.1 | 537.5 | 539.7 | 514.6 / 540.8 | 2.21% |
| run 3 | 1.8 | 470.3 | 513.7 | 538.7 | 505.3 | 507.7 | 478.7 / 551.2 | 2.33% |

("rise" is the highest sample over the last 200 s against the sample at its start.) The
working set climbs from 454 to 470 MB after the fill to 540 to 550 MB by the circle's 150 s and
then stays there (run 3 dropped 40 MB at about 180 s: Windows trimmed it). Private bytes follow
the same shape: 639 to 669 MB after the fill, 744 to 765 MB from the circle's 50 s on.

**Old GPU meshes are freed (the question Task 9's review left open).** The process's dedicated
GPU memory (Windows' "GPU Process Memory" counters) was 138 to 150 MB after the fill and **185 to
191 MB for the whole circle**, while 31,388 chunk meshes were made and their slots overwritten
(`r.meshes[i] = make_mesh(...)` and `= none`); kept, they would have grown it by gigabytes. That
matches the code: assigning to a place that holds a GPU object drops the old value first
(`kir/src/lower/scopes.rs`, `assign_owning`; for a `Mesh?` only when it holds one,
`kir/src/lower/helpers.rs`, `drop_stmts`), which queues the mesh's id, and the next
`window.frame()` unloads it (`window/src/gpu.rs`, `drain`). No fix was needed.

**The plateau, sized.** Only 61 MB of the 540 MB is the program arena. A headless run of the same
build (no GPU, so no driver memory; 0.1 s samples) shows what the runtime itself holds: working
set 304 MB after the fill, 315 MB at the circle's start, 337 MB at 100 s, 352 MB at 200 s and
356 MB at 300 s (private bytes 325, 331, 341, 352, 355 MB). So about **240 to 295 MB of runtime
memory beyond the program arena**: the per-thread spare-block caches (freed blocks of the
per-frame meshing and generation arenas, kept for reuse, up to 256 MiB a thread,
`runtime/src/blocks.rs`) and the threads' stacks. The other ~190 MB of the hidden run's working
set is the GPU driver's and raylib's. **The headless curve had not quite levelled off**: it rose
5.9% from the circle's 100 s to its 300 s (1.2% in the last 100 s), more than E31's 5%, where the
hidden run's (with the driver's memory in the total) rose 2.2%. It is slowing and the caches have
a cap, but the cap is 256 MiB per thread on 17 threads, so this is the known runtime gap E31
predicted, reported rather than hidden: per-thread caches of freed blocks let a long run creep
up while different threads end up holding blocks of different sizes. `mesh_step` still builds
its job list and mesh data fresh each frame (in its arena); reusing them in place was not tried.

**One allocation outside any arena remains:** each edit's list of dirtied chunks (`dirty_for`,
`edit.kes:34`) is made in the program arena and never freed: about 143 bytes an edit on a chunk
corner (a probe calling `edit_at` on one: 1,769,472 bytes of program arena after 1,000 edits,
3,080,192 after 10,000, 15,990,784 after 100,000). The bench's 100 edits are in both flight
lengths, so the comparison above holds; in play it is about 0.5 MB an hour at one edit a second.
See the findings.

## Contracts

Question (E37, E39): which hot loops carry `@prove` contracts, and what checks are left where?
A contract stays in the engine only if `kestrel check` and `kestrel check --release` both pass
with it; one that fails becomes a probe test (`kestrel/voxel/tests/contract_<name>.kes`, which
adds it to a scratch copy and expects the build to refuse it, so a compiler that learns to prove
it fails the test and says to move the contract in) and a finding.

**Kept in the engine** (every engine test runs with `KESTREL_CHECK_PROOFS=1`, checked, as an
exe and, where marked, `--release`; `main.kes` and `bench.kes` pass `check` and
`check --release` with all of them):

| Hot code | Contract | Notes |
|---|---|---|
| generation, `gen.generate` | `@prove(no_alloc)` | added in Task 10 |
| `Grid.get`, `Grid.set` | `@prove(no_alloc)` | added in Task 10; the engine's hot loops index `items` directly instead |
| mesher, `copy_row` (the padded copy's inner loop, 32 of each row's 34 cells) | `@prove(no_checks)` | with `@assume`s on both lists' lengths and on the row's start (`mesher.kes:80-82`) and on the slot at the call (`mesher.kes:210`) |
| `ray.cast` | `@prove(no_alloc)` | |
| `collide.slide`, `collide.move_axis` | `@prove(no_alloc)` | |
| `Frustum.sees` | `@prove(no_alloc, no_checks, no_panic)` | pure float maths |
| `player.step_player`, `player.spawn` | `@prove(no_alloc)` | |

**Probes** (contracts that fail; each test expects exit 1 and "can't be proven `no_checks`"):

| Probe | Contract | What stops it, checked build | What stops it, `--release` |
|---|---|---|---|
| `contract_gen.kes` (new) | `no_checks` on `generate`'s loops | `octave` isn't inlined (the caller's inlining budget ran out), so its shift check `1 << shift` stays (`shift` any `int`) | the writes `g.items[i]`: `i` is at most 131,071 but nothing shows it is below the list's length |
| `contract_grid.kes` (new) | `no_checks` on `Grid.get`/`set` | overflow on `y * self.sz` | bounds check on `self.items[...]` (the index is any `int`: by design, `get` and `set` don't check that the cell is inside) |
| `contract_mesher.kes` | `no_checks` on the meshing loops | overflow on `wy * 32`, `cx * SIZE + x` | the block-type read `w.columns.items[slot].blocks.items[k]` (a list's length reached through an index can't be assumed) and `Builder.face` (not inlined, 258 nodes against 256; its occlusion reads `solid[front + su + sv]`) |
| `contract_ray.kes` | `no_checks` on `cast` | overflow on `c - 1`, `c + 1` (a float's cell is any `int`) | `World.block`'s `self.columns.items[slot]` (the world's check) |
| `contract_collide.kes` | `no_checks` on `slide`/`move_axis` | overflow on `int(ceil(p - SKIN)) - 1` | `World.block`'s check, through `layer_solid` |
| `contract_stream.kes` | `no_checks` on `copy_blocks` | passes with one `stream_step` call in a function, fails with two (the second inlined copy loses its `@assume`) | the same |

**Checks left in the hot functions of `main.kes`** (`kestrel check --report` and
`--report --release`, saved as `out/report_main.txt` and `out/report_main_release.txt`; whole
program: 1,749 checks, 1,481 removed, 268 left, checked; 151, 75 removed, 76 left, `--release`):

| Function | checked: left (removed) | `--release`: left | What is left, and why |
|---|---:|---:|---|
| generation (the stream's `par for` body: `generate` with `density`, `solid_at` inlined) | 10, plus `octave` 14 and `block_type` 1 as calls | 2 | release: the two `j.blocks.items[i]` writes (`gen.kes:59`, `64`): the grid list's length is unknown. Checked: the overflow checks on `cx * SIZE + x` and the like (a column's place is any `int`), and inside `octave` (not inlined: the budget) the shift and `c + 1` checks |
| `mesh_chunk` | 32 (394) | 6 | release: `w.columns.items[slot]` (`slot` is a parameter), the block reads in `flag` (4: slot and `blocks.items[k]`, lengths unknown) and the type read on line 226. Checked: overflow on the world-place arithmetic (`cx * 32 + px`, `layer * 32`, `wy * 32`): the column's place and layer are any `int` |
| `Builder.face` | 40 (34) | 12 | the 12 occlusion reads `solid[front + su + sv]`: in bounds only because of how `front`, `su` and `sv` relate, which ranges can't show; checked, also the overflow on the steps from `int(u.x)` etc. |
| `copy_row` (inlined into `mesh_chunk`) | 0 | 0 | proven (`no_checks`) |
| `World.block` / `World.slot_at` | 2 (7) / 1 (4) | 2 / 1 | `self.columns.items[slot]` and the grid read: the pool's list length is unknown |
| `cast` | 9 (3) | 0 | checked: overflow on `c - 1`, `c + 1`, `x + sx` (cells from floats are any `int`) |
| `slide` (`move_axis`, `layer_solid`) | 2 (2) each | 0 | checked: overflow on `int(ceil(...)) - 1` in `high_cell` |
| `Frustum.sees` (inlined into `draw_world`) | 0 | 0 | proven |
| `copy_blocks` (inlined into the frame loop) | 1 | 1 | the item copy's bounds check (no contract: `contract_stream.kes`) |

`Grid.get`/`set` don't appear: `main.kes` never calls them (the hot loops index `items`).

**The inlining budget, in `--release`:** the report has **27** "isn't inlined: the inlining
budget of ... ran out" messages, all from generation: 13 in the stream's `par for` body
(`__kestrel_par_171`) and 14 in `world.load_now`, for `octave` in `density` (`gen.kes:14`),
`lattice` and `mix` in `octave` (`noise.kes:45-48`) and `block_type` (`gen.kes:59`).

**What the budget costs (an A/B, not a fix).** A scratch copy of the compiler with the budget
(`kir/src/inline.rs`, `GROWTH_BUDGET`) raised from 4,096 to 2^20 added nodes, built into a
separate target folder; the repository's compiler is unchanged. With it the report has no
budget message. Same sources, both `--release`, in turn (load 5.5% before, 6.4% after):

| | budget 4,096 (as is) | budget 2^20 |
|---|---:|---:|
| 64 columns through `load_now`, one thread, whole run, median of 9 (range) | 501 ms (498 to 510) | 307 ms (305 to 310) |
| the same with the empty program's startup (12 / 11 ms) subtracted, per column | 7.64 ms | 4.63 ms |
| `bench.kes` (100 s circle, `GEN_BATCH` 2), work p99, 3 runs each: fly20 | 14.5, 14.5, 14.6 ms | 10.6, 8.9, 8.7 ms |
| fly100 | 15.6, 15.2, 15.5 ms | 11.8, 12.1, 11.7 ms |
| circle | 14.9, 15.2, 15.4 ms | 12.1, 11.7, 11.6 ms |

**Generation runs 1.65 times slower than it would with the noise inlined**, and the frame
budget would have 3 to 4 ms more room. With the raised budget, `GEN_BATCH` 8 still failed E32
(p99 20.4 and 20.6 ms in `fly100`, 20.3 and 20.4 ms in `circle`, two runs): eight columns at
once are limited by sharing the cores, not by the code. The general fix is the compiler's
(findings); nothing was changed here.

## Compile time and size

`py measure.py --compile 9`: `kestrel build`, `kestrel build --release` and `kestrel check` of
`kestrel/voxel/main.kes` (the whole engine: 17 modules), taken in turn, 9 times each, at idle
priority, timed from outside (process start to exit, so each includes about 16 ms of starting
`kestrel`). Load 5.9% before and 5.8% after.

| Command | median | range |
|---|---:|---:|
| `kestrel build` (checks on) | 534 ms | 480 to 603 ms |
| `kestrel build --release` | 485 ms | 473 to 538 ms |
| `kestrel check` | 352 ms | 339 to 369 ms |

The exes are 1,141,248 bytes (checks on) and 1,126,912 bytes (`--release`); most of it is the
window library (raylib).

**Lines per file** (`kestrel/voxel`, with comments; every file is under the 400-line guideline):

| Kind | Files (lines) |
|---|---|
| libraries | `grid` 76, `noise` 49, `ring` 66, `pool` 76, `ray` 103, `collide` 163, `frustum` 84 |
| engine | `blocks` 51, `column` 43, `gen` 75, `world` 125, `mesher` 251, `stream` 302, `player` 235, `edit` 82, `render` 205, `hud` 72 |
| entry programs | `main` 135, `bench` 261 and `script` 188, `parity` 243 and five 8-line `parity_*` entries |
| total | 2,925 lines in 26 files; the tests (`voxel/tests`, 26 files) 2,400 lines |

**Findings:** 26, in `docs/superpowers/plans/2026-10-09-core-2a-followups.md`, "Found while
building the voxel engine (2026-10-25)".

## Race, round 1: block-face meshing against `block-mesh` (E26)

Question: how fast is Kestrel's block-face meshing (a face for each side of a solid block that
touches air) against Rust's `block-mesh` crate (0.2.0, `visible_block_faces`, the simple mesher)
on the same input? Round 1 is the one with the same algorithm, so it measures the language and
the compiler, not the algorithm. **Result: Rust is faster, by 1.38 times for the faces and 1.83
times for the faces plus the mesh arrays** (Kestrel `--release` against Rust release).

### What each side does

**Input.** The frozen world's 256 chunks (8 x 4 x 8, seed 1337) as padded 34^3 solid flags, from
the frozen world's noise: Kestrel `race.kes` (`octave` from `noise.kes`, with `gen.solid_at`
written out in the file, because `gen` and `blocks` import `draw`, whose natives the timer's JIT
doesn't link), Rust `src/race.rs` (a port of `hash`/`lattice`/`smooth`/`mix`/`octave`/`density`
in `f32`, the same order of operations as `voxtuned/rust/voxels3d.rs`). Both find **463,749
faces** (1,811.5 a chunk). A different total is an error, not a number: Rust `assert_eq!`s it on
the first pass, Kestrel's `expect` stops the program ("index N out of bounds for length 1", exit
101; tried with a wrong constant), and `measure.py` refuses any "race" row whose checksum isn't
463,749.

**The same algorithm.** Per chunk, for each of the 32^3 interior blocks (the border of the 34^3
is only read, as neighbours): if the block is solid, for each of the six sides (in block-mesh's
order: -x -y -z +x +y +z) emit a face if the neighbour is air.

| | Kestrel (`voxel/race.kes`) | Rust (`bench/kernels/src/race.rs`) |
|---|---|---|
| block type | `Grid[u8]` (a `list[u8]`), 1 = solid | a newtype `Solid(u8)` implementing `Voxel` (`get_visibility`: `Empty` or `Opaque`), in a `Vec<Solid>` per chunk |
| layout | one `Grid[u8]` of 34 x (34 * 256) x 34, the chunks one after another along y; index `(y * 34 + z) * 34 + x` | one `Vec` per chunk, `ConstShape3u32<34, 34, 34>`: index `x + 34 y + 1156 z` |
| loops | y, z, x | `Extent::iter3`: z, y, x (memory order in both) |
| neighbour reads | `world.items[i +- 1 / 34 / 1156]`, bounds-checked | `voxels.get_unchecked(...)` inside the crate: no bounds checks |
| kernel 1 output | six `list[int]`, one per side, each `x \| y << 6 \| z << 12` of the block in padded coordinates | a `UnitQuadBuffer`: six `Vec<UnorientedUnitQuad>` (`minimum: [u32; 3]`, 12 bytes a face) |
| kernel 2 output | three lists: positions and normals (`list[vec3]`, 12 bytes each, like Rust's `[f32; 3]`) and indices (`list[int]`), 4, 4 and 6 a face, with the corners and winding of `RIGHT_HANDED_Y_UP_CONFIG` written out | three `Vec`s: `quad_mesh_positions`, `quad_mesh_normals` and `quad_mesh_indices` of each face appended with `extend_from_slice` |

**List management (E26), the same on both sides.** The lists are made once per call (Kestrel) or
once per run (Rust) and **cleared and reused for every chunk**: Kestrel `xs.clear()`, Rust
`UnitQuadBuffer::reset()` (its six `Vec::clear`s) and `Vec::clear`. After the first chunks have
grown them nothing allocates per chunk. Kestrel's whole call, input included, is inside one
`arena { }` (freed at the end of the call, so a call keeps nothing); the six face lists are
separate local `var`s (see the struct variant below).

**Beyond the totals.** `voxel/race_check.kes` and `rust ... --verify` print a checksum of every
packed face (kernel 1) and of every position, normal and index (kernel 2, weighted by corner and
by place in the six): **194,649 and 10,449,332 on Kestrel, block-mesh and the plain Rust port
alike.** So the two sides make the same faces with the same corners and winding. (It caught
nothing: the first version matched.)

### How it was measured

`kestrel/bench/kernels`' timer and `measure.py`: `py measure.py --rounds 9 --only race --threads 1
--rust-checks target/ovf/release`, from `kestrel/bench/kernels` after `cargo build --release`
(and a second build, `RUSTFLAGS="-C overflow-checks=on" cargo build --release --target-dir
target/ovf --bin rust`, whose overflow checks include the crate). Each round runs every row once,
in turn; a row is the median of 5 repetitions (after a warm-up one); the table is the median of
the 9 rounds with the range of the 9 medians. Idle priority, one thread (`KESTREL_THREADS=1`; the
Rust side is one thread). Kestrel is the JIT (compiled before the clock starts, after a warm-up
call); the Rust modes are an ordinary release build (`opt-level` 3, default `x86-64` target like
Cranelift's baseline, no `target-cpu=native`). "Overflow checks" is the same code built with
`-C overflow-checks=on`.

**Setup subtraction.** Each Kestrel call builds the 256 chunks' flags and then runs `iters`
passes; the timer runs the call with `iters` and with 0 and takes `(t - base) / (n * iters)`
with `n` = 256 chunks (the Rust side builds its inputs once, outside the clock). The setup,
measured as a whole-process difference (`kestrel run --release` with 256 chunks against 1 chunk,
9 runs): 584 ms against 221 ms, **363 ms**; ranges 581-589 and 219-225, so about +-4 ms. `iters`
is 60 for kernel 1 and 40 for kernel 2, so the timed kernel work is about 1.0 s in `--release`
(1.3 s with checks on), 2.8 to 3.6 times the setup, and the setup's jitter is about 0.4% of it.
The rounds' own ranges (0.4% to 0.9% of the median) say the same.

**Machine and load.** Windows 11, 16 logical cores, otherwise idle: CPU of the whole machine
before the run, 8 samples of 3 s: mean 0.2%; during the run, 136 samples of 5 s: mean 6.7%, max
10.9% (one busy core of 16 is 6.25%, so about 0.4% was other load). A first, identical run is
within 0.3% of every number below (kernel 1 `--release` 65,967 and kernel 2 103,075 ns; Rust
47,773 and 56,247).

### Results (ns per chunk of 32^3 blocks, 1,811.5 faces; median of 9 rounds, range)

| | Kestrel, checks on | **Kestrel `--release`** | **Rust release** | Rust, overflow checks | Rust, the Kestrel kernel written out (release) |
|---|---:|---:|---:|---:|---:|
| **Kernel 1: faces** | 82,175 (81,937-82,500) | **66,012** (65,813-66,399) | **47,791** (47,532-47,966) | 53,114 (52,967-53,390) | 41,109 (41,017-41,212) |
| **Kernel 2: faces, then positions, normals, indices** | 126,907 (126,671-127,646) | **103,177** (102,897-103,454) | **56,255** (56,207-56,423) | 66,372 (66,074-66,554) | 56,199 (56,038-56,384) |

| Ratio, Kestrel over Rust | Kernel 1 | Kernel 2 |
|---|---:|---:|
| **`--release` over release (the headline)** | **1.38** | **1.83** |
| checks on over Rust with overflow checks (checks against checks) | 1.55 | 1.91 |
| checks on over Rust release | 1.72 | 2.26 |
| `--release` over the same code written in Rust | 1.61 | 1.84 |

- A face costs kernel 1 36.4 ns on Kestrel, 26.4 on block-mesh and 22.7 on the plain port.
  Kernel 2's array stage alone (kernel 2 minus kernel 1) is **37.2 us a chunk on Kestrel (20.5 ns
  a face) against 8.5 us on block-mesh (4.7 ns a face)**, 4.4 times; the plain port's
  push-by-push version of that stage is 15.1 us.
- Kestrel's checks cost 24% (kernel 1) and 23% (kernel 2) over `--release`. Rust's overflow
  checks cost 11% and 18% on block-mesh's code (the crate's index arithmetic) and nothing on the
  plain port.
- **The plain Rust port** (`race_faces_plain`, `race_arrays_plain`: not part of the race, a
  diagnostic) is `race.kes`'s kernels written in Rust: the same layout, loops and checked
  indexing into a `Vec<u8>`, six `Vec<u32>` of the same packed faces, the same corners, a push
  per entry. It prints the same checksums. It shows the gap is not only block-mesh's own tuning:
  written like Kestrel's code, Rust is still 1.61 and 1.84 times faster. Its kernel 1 is faster
  than block-mesh's (41.1 against 47.8 us: its 4-byte packed faces move less memory than
  block-mesh's 12-byte `UnitQuad`s), its kernel 2 equal.

### Where Kestrel loses, and what is not known

Not profiled: these are differences in what the two sides do, not measured shares.

- **Bounds checks.** block-mesh reads voxels with `get_unchecked`. Kestrel keeps a check on the
  list index of the neighbour reads (the compiler can't show `i + 34 * 34` is below
  `items.len`; `check --report --release` lists 6 checks left in `find_faces`). The plain port
  has checked reads too and still beats Kestrel, so that is not the whole gap.
- **`clear()` zeroes.** Kestrel's `xs.clear()` writes zeros over the old items
  (`kestrel_rt_list_clear` does a `write_bytes`); `Vec::clear` doesn't. In kernel 2 that is about
  225 KB a chunk (87 KB each of positions and normals, 43 KB of indices, 7 KB of faces), roughly
  6 to 10 us of the 103 (an estimate from the byte count, not measured).
- **Many small pushes.** Kernel 2's array stage is 14 `add`s a face on Kestrel (each a capacity
  check, a store and a length update); block-mesh appends 4, 4 and 6 at once with
  `extend_from_slice`. The plain port, which pushes one entry at a time, is still 2.5 times
  faster than Kestrel on that stage, so that is not the whole gap either.
- **Six lists in a struct are slower than six locals.** The first version kept the six face lists
  in a struct (`faces.neg_x.add(p)`): kernel 1 `--release` measured 74.6 us against 65.6 us for
  six local `var`s, run back to back with 5 repetitions each (14% slower); kernel 2 110.2 against
  103.2 (earlier, not run alongside the final version). The raced code is the faster form. A list
  reached through a struct field is probably reloaded on every `add`; the engine's `Mesh` struct
  does it.
- **Inliner budget: none.** `kestrel check --report --release` on `race_faces.kes` and
  `race_arrays.kes` has no "inlining budget ... ran out" message. The only "isn't inlined" lines
  are for `build_arrays` ("it is 657 nodes; the limit is 256", a size limit, so a call per chunk).
  `find_faces` and the set-up's noise are inlined into `bench`. (`race_check.kes`, which is never
  timed, does run out of budget; it only builds the checksums.)

### The whole frozen world against the straight Rust port (`C:\kt\voxtuned\rust`)

It builds: `rustc -C opt-level=3 voxels3d.rs` (and with `-C overflow-checks=on`), rustc 1.97.0,
output in a scratch folder (the folder itself is untouched); it prints 463,749. Against the
frozen `bench/voxels3d/main.kes` and `voxels3d_tuned.kes` built with today's compiler (HEAD,
`--release`, exes in a scratch folder), 11 rounds in turn, whole runs, `bench.py` from
`C:\kt\voxtuned`, idle priority, machine idle (CPU 0 to 1% after):

| Whole run | median (range) | minus startup (6 ms) | against Rust release |
|---|---:|---:|---:|
| Rust, release | 328 ms (327-333) | 322 | 1.00 |
| Rust, overflow checks | 340 ms (338-376) | 334 | 1.04 |
| Kestrel frozen, `--release` | 375 ms (374-379) | 369 | **1.15** |
| Kestrel tuned, `--release` | 303 ms (302-322) | 297 | **0.92** |

Over the whole world (the noise is nearly all of it) Kestrel is level with Rust: 1.15 for the
frozen code and 0.92 for the hand-tuned copy, as in the earlier sessions (1.13 and 0.92). The
meshing alone is where Kestrel is behind. This is not the race (it is mostly the noise), only the
check that the machine and the Rust port still say what they said.

### What round 2 would need (greedy meshing against `binary-greedy-meshing`)

Written from the plan and from knowing the crate in outline, not from reading it. The crate works
on columns of bits (a `u64` a column), so a Kestrel version needs `u64` bit operations and
counting the set bits of a word (no `trailing_zeros` or `count_ones` is in the syntax document; a
loop is the fallback and would be the first thing to lose). Its chunk is 62^3 (64^3 padded), not
32^3, so the input would be rebuilt at that size on both sides. Its output (quads with a width and
a height, merged by block type) has a different shape from round 1's unit faces, so the face
total and the check value would be new. block-mesh's own `greedy_quads` on the same input would
be an intermediate round (the same crate, the 34^3 chunks of this one). The findings above are the
first things round 2 will meet: bounds checks on the bit-array reads, `clear()` zeroing, lists
held in structs, and `add` one entry at a time.

### Files

`kestrel/voxel/race.kes` (the kernels and the input), `race_faces.kes` and `race_arrays.kes` (the
timer's entries, each `pub bench(n, iters)`), `race_check.kes`; `kestrel/bench/kernels/src/race.rs`
(the Rust side), `src/bin/rust.rs` (modes `race_faces`, `race_arrays`, `race_faces_plain`,
`race_arrays_plain`, and `--verify`), `measure.py` (the "race" rows, `--rust-checks`, the
face-total check), `Cargo.toml` and `Cargo.lock` (`block-mesh = "0.2"`).
