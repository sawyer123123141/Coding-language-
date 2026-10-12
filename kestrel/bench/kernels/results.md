# Kernels: Kestrel's inner loops against Rust

These are small loops from the particles demo, timed alone so that a change to the compiler's code for a loop shows up without a window, a GPU or a draw call in the way. Each kernel is a Kestrel program with a `pub bench(n: int, iters: int) -> float` that makes `n` particles, runs the kernel over them `iters` times and returns a checksum. The numbers are nanoseconds per particle (per item), the median of 9 repetitions, with the cost of making the particles taken out by also timing a run with 0 iterations.

- `step.kes`: the demo's `Particle.step` (pull towards a target, damping, move, bounce off the edges), in a `par for` over a `list[Particle]`. `pow(KEEP, dt)` is worked out once and passed in.
- `step_scalar.kes`: the same step written by hand with `float` locals: the four fields are read once, the maths uses no `vec2` values, and the fields are written once. This is the speed the compiler should reach on `step.kes` by itself.
- `step_pow.kes`: the step exactly as the demo writes it, with `pow(KEEP, dt)` for every particle.
- `copy.kes`: the demo's draw copy, every particle's position into a `list[vec2]`.
- `src/bin/rust.rs`: the same three kernels in Rust (`step`, `pow` and `copy`), one thread, same arithmetic.

## How to run it

From `kestrel/bench/kernels` (it is its own Cargo workspace, listed in `exclude` in `kestrel/Cargo.toml`, so the compiler's tests do not build it):

```
cargo build --release
py measure.py --rounds 3 --threads 1
py measure.py --rounds 3 --threads 16
```

`measure.py` runs every kernel for a few rounds, one after the other so they share the machine's conditions, and prints the median of the rounds' medians with their range. `--threads` sets `KESTREL_THREADS` for the Kestrel kernels, which is the thread count of their `par for`; the Rust kernels always use one thread. A single kernel can be run with `target/release/kernels.exe step.kes --release`, and takes `--n`, `--iters`, `--reps` and, without `--release`, checks integer overflow as `kestrel run` does.

To see what the compiler made of a loop, `target/release/kernels.exe step.kes --release --dump par` prints the optimised Cranelift IR and then the machine code of every `par for` body. `--dump NAME` does the same for the function with that name. The `explicit_slot` lines at the top of the IR are stack slots. Before scalar replacement `step.kes` had 8 of them, each holding a `vec2` temporary of the step (the vector to the target, its direction, the pull, and so on) that was stored to the stack and loaded back; now it has none.

## Before scalar replacement

Measured on 2026-10-11, with the compiler at commit abc4d6a, on an AMD Ryzen 7 5800X (8 cores, 16 threads), Windows 11. The machine is noisy, so compare rows inside a column, and compare columns only for the same run. Each cell is the median of 3 rounds with the range of the rounds in brackets, in ns per particle.

| Kernel | One thread | 16 threads |
|---|---:|---:|
| `step.kes` (the demo's step, `pow` hoisted) | 13.06 (12.93-13.19) | 1.86 (1.84-1.86) |
| `step_scalar.kes` (written with float locals) | 4.42 (4.36-4.44) | 0.78 (0.76-0.78) |
| `step_pow.kes` (`pow` per particle, the demo) | 24.65 (24.59-24.83) | 3.38 (3.37-3.40) |
| `copy.kes` (the draw copy) | 0.70 (0.68-0.70) | 0.68 (0.68-0.69) |
| Rust step | 3.85 (3.82-3.86) | 3.84 (3.84-3.86) |
| Rust pow | 16.10 (16.00-16.15) | 16.06 (16.03-16.09) |
| Rust copy | 0.20 (0.20-0.21) | 0.21 (0.20-0.22) |

The Rust kernels are one thread in both columns, so their two columns are two measurements of the same program: they differ by under 1%.

So at one thread `step.kes` takes 3.4 times as long as Rust's step, while `step_scalar.kes` takes 1.15 times as long: the difference between the two is the `vec2` values the compiler keeps in memory. `copy.kes` takes 3.5 times as long as Rust's copy. The `copy.kes` kernel does not use `par for`, so its two columns agree.

The "After" section below has the numbers for the finished work. Those were measured against a rebuilt old compiler, side by side, because the machine ran hotter later in the day than when this table was taken; this table is kept as it was measured.

## After scalar replacement

Measured on 2026-10-11 with the compiler at commit 04358c9 (the end of the scalar replacement plan), on the same machine as above.

**How it was measured.** The machine ran hot and noisy that afternoon: Rust's own kernels were 10-15% slower than in the table above, and the old compiler's `bench/2a` particles took 351 ms where the saved baseline run had 317. So a fixed "before" table would have misled. Instead the old compiler was rebuilt in a temporary git worktree at commit f329f57 (the pre-change compiler abc4d6a plus `kestrel_codegen::dump` and the kernels harness, and nothing else), and every comparison below ran old and new one after the other, taking turns in each round, so both saw the same conditions. The tables give the median of the rounds with the range of the rounds in brackets. The Rust kernels are the same program for both compilers; their two timings differ by 0-9% (see the Rust rows), which shows the noise.

### The kernels

7 rounds each, ns per particle (per item). The `measure.py` figures from the same build (3 rounds, run alone, no old compiler beside them) are in the second table.

| Kernel | One thread, before | One thread, after | 16 threads, before | 16 threads, after |
|---|---:|---:|---:|---:|
| `step.kes` (the demo's step, `pow` hoisted) | 14.46 (14.18-15.13) | **4.88** (4.71-5.17) | 2.10 (1.99-2.27) | **0.85** (0.82-0.90) |
| `step_scalar.kes` (written with float locals) | 4.88 (4.76-5.33) | 4.79 (4.72-5.20) | 0.89 (0.85-0.98) | 0.90 (0.84-0.96) |
| `step_pow.kes` (`pow` per particle, the demo) | 27.62 (26.74-29.90) | **19.12** (18.56-21.78) | 3.84 (3.70-4.35) | **2.59** (2.44-2.79) |
| `copy.kes` (the draw copy) | 0.80 (0.74-1.08) | 1.11 (1.02-1.32) | 0.79 (0.75-0.82) | 1.07 (1.03-1.11) |
| Rust step (old tree's binary / new tree's) | 4.24 / 4.25 | | 4.12 / 4.12 | |
| Rust pow | 17.89 / 17.84 | | 17.18 / 17.26 | |
| Rust copy | 0.24 / 0.22 | | 0.23 / 0.23 | |

| Kernel (`measure.py`, 3 rounds) | One thread | 16 threads |
|---|---:|---:|
| `step.kes` | 4.77 (4.74-5.03) | 0.86 (0.84-0.87) |
| `step_scalar.kes` | 4.83 (4.76-4.88) | 0.92 (0.84-0.94) |
| `step_pow.kes` | 18.89 (18.79-18.91) | 2.65 (2.51-2.72) |
| `copy.kes` | 1.09 (1.06-1.13) | 1.09 (1.07-1.10) |
| Rust step | 4.26 (4.24-4.26) | 4.17 (4.15-4.25) |
| Rust pow | 17.78 (17.55-17.90) | 17.48 (17.40-17.73) |
| Rust copy | 0.26 (0.25-0.26) | 0.24 (0.24-0.27) |

**The targets** (from the design):

| Target | Result |
|---|---|
| `step.kes` within 1.2x of Rust's step | **Met:** 1.15x (4.88 against 4.25; `measure.py`: 1.12x). It was 3.4x. |
| `step.kes` within 5% of `step_scalar.kes` | **Met:** +2% (4.88 against 4.79; `measure.py`: -1%). The compiler now reaches the speed of the hand-written version by itself. |
| `step_pow.kes` within 1.15x of Rust's `pow` version | **Met:** 1.07x (19.12 against 17.84; `measure.py`: 1.06x). It was 1.55x. |
| No other kernel slower beyond noise | **Not met at this commit:** `copy.kes` is 0.3 ns an element slower (see below). Nothing else was. **Met after Task 6** (the section "After the one-check struct store" below): `copy.kes` is back to the speed it had before scalar replacement, within noise. |

`step.kes` has no stack slots now (`--dump par` shows no `explicit_slot` lines), and its loop is the same arithmetic as `step_scalar.kes`. On 16 threads `step.kes` is 2.5 times faster (2.10 to 0.85) and `step_pow.kes` 1.5 times (3.84 to 2.59). The math change alone (the last commit of the plan) moved `step_pow.kes` by about 1%, inside the noise, so the `pow` gain is the struct work. The cost of the `pow` call itself (`step_pow.kes` minus `step.kes`) is 13.2 ns before and 14.2 after, and 13.6 for Rust: the call costs what it did, and everything around it got cheaper.

### The copy loop got slower, and is still about 4.5 times Rust

(This section is what was found at commit 04358c9. The slowdown is fixed by Task 6; see "After the one-check struct store" below.)

`copy.kes` takes 1.11 ns an element on one thread where it took 0.80 (0.3 ns, or 39%, more; every one of the 7 rounds agrees, on one thread and on 16, since this kernel has no `par for`). Rust takes 0.22-0.26, so the loop is now 4.2-4.8 times Rust's time; it was 3.5 times.

It is slower because the new code does more checking per element. The loop is `positions[i] = p.pos` for each `p` in the particles. Before, that was one copy of 8 bytes from a place to a place: one bounds check on `positions[i]`, then the two 4-byte moves, in one block. Now `promote` keeps `p.pos` in two registers (it loads both fields at the top of the iteration), and `scalarize`'s struct-store rule splits the store of those registers into one store per field. Each of the two stores to `positions[i].x` and `positions[i].y` checks the index against the list's length and reads the list's data pointer again. The machine code of the hot loop (`--dump bench`) has two compares and branches, and two reloads of the length and the data pointer, where it used to have one of each. That is real extra work per element, and it is the cost the design's "bounds checks and list-header reloads" follow-up names.

Why the loop is still far from Rust, which does it in about one cycle an element:
- it reloads the list header (data pointer and length) from memory for each check, because Cranelift can't prove the store to the list's elements didn't change it;
- it checks the index against the length for each store (twice, now);
- it moves 8 bytes as two 4-byte moves, where Rust's single 8-byte move (and, after vectorising, wider ones) does the same work.

The old number is also somewhat lucky. The copy loop is sensitive to where the compiler puts the code. Adding an unused function before `spawn` (four layouts, 0 to 3 unused functions, 3 runs each) moved the old compiler's loop between 0.70 and 0.77 ns on three layouts and to 0.94-1.06 on the fourth, while the new compiler's loop stayed between 0.92 and 1.04 on all four. So the slowdown is 0.2-0.3 ns when the old code was lucky, and none when it wasn't. In the particles demo the copy runs once a frame over 200,000 particles, so 0.3 ns is 0.06 ms of a 5.4 ms frame.

### After the one-check struct store (Task 6)

**The fix.** The two passes' struct-store rule used to write a built struct into memory as one `Assign` per leaf, and codegen computed the place again for each one: its own bounds check on `positions[i]` and its own reload of the list's length and data pointer. Now `split.rs` writes a plain struct value into memory as ONE `Assign` of a `StructLit` whose fields are constants and `Value` locals (all computed first, so the value still can't read a half-written place, and the destination's index expressions still run after the value), and codegen builds that `StructLit` straight into the place, which it finds once. A store into a split local is still one assignment per leaf. `positions[i] = p.pos` is one bounds check again (`a_struct_store_into_a_list_checks_the_index_once` counts the `trap user2` paths in the IR: 2 before the fix, 1 after).

**How it was measured.** Three harnesses built from three commits, run one after another in a rotating order in each round, one thread (`KESTREL_THREADS=1`), `--release`, the same iteration counts as `measure.py`; two runs of 11 rounds, pooled to 22. They are the compiler before scalar replacement (f329f57), at the end of the scalar replacement plan (c4fac4a, the "After" above) and with this fix. **The machine was loaded** by another program (a game) for the whole hour, so everything below is 35-40% slower than the tables above (Rust's copy kernel, the gauge, took 0.31 ns against 0.23 then), and the 16-thread runs had ranges of 40% or more, so they are not given. Compare cells inside this table only. The cells are the median of the 22 rounds with the range in brackets, in ns per item.

| Kernel, one thread | Before scalar replacement | Without the fix | With the fix |
|---|---:|---:|---:|
| `copy.kes` (the draw copy) | 1.17 (0.87-1.53) | 1.72 (1.26-2.11) | **1.27** (0.86-1.67) |
| `step.kes` | 20.45 (16.05-24.68) | 6.59 (5.53-7.76) | 6.58 (5.67-7.55) |
| `step_pow.kes` | 40.03 (30.90-46.52) | 25.95 (20.85-32.46) | 26.37 (21.09-30.60) |
| `step_scalar.kes` | 6.76 (5.41-8.02) | 6.40 (5.58-8.16) | 6.39 (5.44-7.82) |
| Rust copy (from the two newer builds' binaries) | | 0.31 (0.28-0.43) | 0.32 (0.28-0.39) |

Round by round (the same round, so the same load), the new time over the old time:

| | Median | Range |
|---|---:|---:|
| `copy.kes`, with the fix over without | **0.72** | 0.62-0.90 |
| `copy.kes`, with the fix over before scalar replacement | **1.01** | 0.77-1.27 |
| `step.kes`, with the fix over without | 0.99 | 0.88-1.14 |
| `step_pow.kes`, with the fix over without | 0.98 | 0.81-1.27 |
| `step_scalar.kes`, with the fix over without | 0.98 | 0.82-1.30 |

- **`copy.kes`** takes 28% less time than at the end of the plan, and the same time as before scalar replacement within what this machine can tell (the paired ratio is 1.01; the two medians in the table are 8% apart, and the ranges overlap almost completely). The draw copy is no longer slower than it was.
- **The loop is the same work per element as before scalar replacement** (`kernels.exe copy.kes --release --dump bench`): one load of the particles' data pointer, two 4-byte loads of `p.pos`, one bounds check (a load of the list's length, a compare, a branch), a load of the destination's data pointer, two 4-byte stores. The only difference is the order: both loads of `p.pos` now come before the check, because `promote` loads them at the top of the iteration.
- **It is still about 4 times Rust** (1.27 against 0.31 in this run; 3.8 times for the compiler before scalar replacement, 5.5 times without the fix). The reasons are the ones listed above: the list header is reloaded for each check, the index is checked, and 8 bytes move as two 4-byte moves. They belong to the bounds-check and vectorizing follow-ups, not to this work.
- **`step.kes`, `step_pow.kes` and `step_scalar.kes` did not move** (paired ratios 0.98-0.99, with ranges that cross 1). The particle step's write-backs to the element are stores through the element pointer, which have no bounds check, so this change doesn't touch them, and the particle step still has no stack slots (the codegen test `the_particle_step_in_a_par_for_uses_no_stack_slots` passes).
- **`bench/2a` `particles` and `swarm`** (`kestrel build` exes and `kestrel run`, 7 rounds, old compiler at c4fac4a against new, taking turns, ms, loaded machine; the Rust -O3 column of these runs was 437-494 ms for particles and 358-554 for swarm, against 322-329 and 99-114 in the table above, so only compare old against new):

  | | exe (checked) | exe (`--release`) | `kestrel run` |
  |---|---:|---:|---:|
  | `particles`, without the fix → with it | 270 → 273 | 371 → 347 | 355 → 358 |
  | `swarm`, without the fix → with it | 531 → 538 | 519 → 525 | 477 → 522 |

  All differences are inside the ranges of the rounds (`particles` 225-463 ms for the checked exe; `swarm` 434-811 ms across its cells), so neither moved beyond noise. The one `bench/2a/run.ps1` run on the new compiler (checked / `--release` / JIT) gave `particles` 265 / 328 / 385 ms and `swarm` 319 / 398 / 346 ms, against Rust -O3's 521 and 213 (the release-default column) in that run. As a multiple of Rust's time in the same run, `particles` is 0.51 / 0.63 / 0.74 (0.51 / 0.63 / 0.65 at the end of the plan) and `swarm` is 1.5 / 1.9 / 1.6 (1.8 / 1.8 / 1.9). The JIT cells include the compile time, so they are the least comparable with a Rust run. The absolute figures are higher than in the plan's table because the machine was loaded.

### `bench/2a`, `heightmap`, `voxels3d`, `maps`

Each `run.ps1` ran 3 times for the old and 3 for the new compiler, taking turns (the old one with `run.ps1` copied to a scratch folder and pointed at the old `kestrel.exe`). The cells are medians of those 3 runs, each of which is the script's own median of 5 or 7, in ms. The old tree's saved baseline from Task 1 (a quieter hour) had, for example, particles at 317 ms and heightmap at 66 / 46 ms; the "before" cells here are 5-15% higher because the machine was.

**`bench/2a`** (Kestrel columns; the last column is Rust -O3 release as a noise gauge: it is the same program in both runs):

| Workload | exe (checked), before → after | exe (`--release`), before → after | `kestrel run`, before → after | Rust -O3 (noise), before → after |
|---|---:|---:|---:|---:|
| fib | 257 → 251 | 250 → 242 | 262 → 265 | 96 → 99 |
| loop | 692 → 711 | 689 → 703 | 692 → 706 | 588 → 570 |
| collatz | 322 → 309 | 256 → 251 | 318 → 278 | 106 → 109 |
| mandelbrot | 437 → 426 | 433 → 422 | 443 → 443 | 350 → 350 |
| particles | 351 → **163** | 354 → **202** | 359 → **209** | 329 → 322 |
| sieve | 984 → 915 | 926 → 973 | 979 → 939 | 924 → 849 |
| swarm | 200 → 174 | 248 → **178** | 231 → 186 | 114 → 99 |

- **`particles`** is 1.7-2.2 times faster: the whole particle stays in registers across the 100 million steps (`kernels.exe --dump main` shows no stack slot, and no load or store of the particle's fields, in the loop). It is now faster than Rust -O3 (322-329 ms) in this build, and C (132 ms) is still ahead. I didn't look into why Rust's version of this loop is slow.
- **`swarm`** is 1.1-1.4 times faster (its `for p in ps` loop now keeps `p`'s fields in registers).
- **Everything else is inside the noise:** `fib`, `loop`, `collatz` and `mandelbrot` moved by 0-4%, within the 0-3% that Rust's own column moved; `sieve` moved by -7% to +5%, while the Rust and C columns of `sieve` moved by up to 13% (this machine is at its noisiest on `sieve`). The `collatz` JIT cell fell 13%; that is the one cell that might be real, and I did not investigate it.
- **In the new compiler the `particles` exe built with `--release` is slower than the checked one** (202 against 163 ms here; 172 against 137 on a quiet re-run). That is not a regression (both are about twice as fast as the old exe's 350), and it comes from where the code lands. Adding one unused function in front of the program flips the two (checked 194, `--release` 167), and the old compiler's two exes stayed at 351-366 on all four layouts I tried. The new loop runs at 148-158 or 167-196 ms depending on the layout.

**`bench/heightmap`** (ms):

| Variant | Before | After |
|---|---:|---:|
| Kestrel exe (checked) | 76 (74-77) | 73 (72-76) |
| Kestrel exe (`--release`) | 52 (52-53) | 58 (58-62) |
| `kestrel run` (JIT, incl. compile) | 100 (97-102) | 99 (95-101) |
| `kestrel check` (front end only) | 19 (16-19) | 18 (17-19) |
| Rust -O3 (checks on / release) | 31 / 27 | 32 / 27 |

The `--release` exe looks 12% slower (58 against 52 ms; a longer re-run of 25 alternating runs gave 53 against 49, +8%). I found no extra work in it: the machine code of the hot function (`Chunk.checksum`) is the old code minus one load per iteration, since `promote` loads the column's height once instead of in each of the 64 steps of the inner loop, and `Chunk.generate` is identical. The difference is the code layout again: with two or three unused functions added in front of `hash`, the order flipped (old 52.4 ms against new 49.3, 15 runs). So I count it as noise from where the compiler puts code, not a slowdown, but note that this program's `--release` figure moves by about 6% with layout alone.

**`bench/voxels3d`** (the world-building time, ms, startup taken out):

| Variant | Before | After |
|---|---:|---:|
| Kestrel exe (checks on) | 1,141 (1,129-1,222) | 1,157 (1,123-1,166) |
| Kestrel exe (`--release`) | 494 (493-537) | 491 (488-500) |
| `par for`, 1 thread (checks on) | 1,147 (1,134-1,185) | 1,192 (1,142-1,193) |
| `par for`, 1 thread (`--release`) | 493 (489-513) | 505 (503-519) |
| `par for`, all threads (checks on) | 175 (166-180) | 170 (169-173) |
| `par for`, all threads (`--release`) | 75 (73-82) | 78 (72-80) |
| C# .NET 9 / C# `Parallel.For` (noise gauge) | 1,497 / 357 | 1,497 / 391 |

No change: the differences (-3% to +4%) are inside the noise, and C#'s own `Parallel.For` row moved by 10% between the two runs. The mesher builds `vec3` values and pushes them into lists, so the passes do rewrite it; whatever they gain is smaller than the noise, and they cost nothing visible.

**`bench/maps`** (ms, 6 rounds each because the first 3 were very noisy):

| Workload | Checks on, before → after | `--release`, before → after |
|---|---:|---:|
| int | 550 → 600 | 594 → 509 |
| scattered | 483 → 521 | 428 → 490 |
| words | 472 → 472 | 468 → 464 |

This benchmark can't be read here. The old compiler's own times ranged from 467 to 1,958 ms across the 6 rounds, against 229-242 ms on the quiet hour of the saved baseline, and the best runs of the two compilers are the same (best of 6, checks on, before / after: int 490 / 305, scattered 452 / 389, words 444 / 434). The time is in the runtime's map calls, which the passes don't touch, so I expect no change, but the measurement doesn't show that on its own.

### The particles demo, at 200,000 particles

`demos/particles/particles.kes` with `START_COUNT = 200000` and the mouse target fixed at (640, 360), run with `KESTREL_HIDDEN=1` as `kestrel run --release` for about 11.5 seconds and read from its one-a-second lines (the first dropped). 3 runs of each compiler, taking turns, 26-27 per-second averages pooled.

| | Before | After |
|---|---:|---:|
| update, 16 threads | 1.33 ms (1.25-1.52) | **0.95 ms** (0.89-1.14) |
| update, 1 thread (`KESTREL_THREADS=1`, 2 runs each) | 6.23 ms (5.89-7.21) | **4.29 ms** (4.00-4.68) |

The update is 29% faster on 16 threads and 31% on one thread. The draw and frame times of these runs can't be used: the hidden window was held at 60 fps (every frame of every run was 16.6-16.7 ms, and the draw time of 10-11 ms includes waiting for the display driver), the cap the handoff warns about. The update is the time of the `par for` and is valid either way.

This matches the kernels: `step_pow.kes` is 1.44 times faster on one thread and 1.48 on 16, and the demo's update is 1.45 and 1.40 times faster.

### Compile time of the voxel viewer

`kestrel check` and `kestrel build` of `demos/voxels/voxels.kes`, old and new alternating, in ms (process start is about 15 ms of each):

| | Before | After |
|---|---:|---:|
| `check` (21 runs) | 20.3 (18.1-28.1) | 21.5 (19.1-32.3) |
| `build` (15 runs) | 193.8 (174.0-315.2) | 185.0 (168.7-253.6) |
| `build --release` (7 runs) | 203 (181-324) | 183 (166-269) |

`check` is about 1 ms slower (5%; an earlier 7-run set had 20 against 23 ms) and `build` is the same within the noise. The exe is a little smaller (1,050,112 bytes against 1,051,648 checked; 991,744 against 993,280 with `--release`).

### In short

Faster: the demo's step (3.0x on one thread, 2.5x on 16, to within 15% of Rust), the `pow` version of it (1.4-1.5x), `bench/2a`'s `particles` (1.7-2.2x) and `swarm` (1.1-1.4x), and the particles demo's update at 200,000 (29-31%). The same: `fib`, `loop`, `collatz`, `mandelbrot`, `sieve`, `heightmap`, `voxels3d`, compile time, and (as far as this machine could say) `maps`. Slower at the end of the plan: `copy.kes`, the demo's draw copy, by 0.2-0.3 ns an element (two bounds checks where there was one); the matching loop in the particles demo (`positions[i] = p.pos`) is the same code. Task 6 fixed it (one bounds check again; the copy takes the same time as before scalar replacement within noise, and about 4 times Rust's). Not slower but worth knowing: this machine's results move by 6-30% with where the compiler puts the code, which is why every comparison here is old against new taking turns.
