# Kernels: Kestrel's inner loops against Rust

These are small loops from the particles demo, timed alone so that a change to the compiler's code for a loop shows up without a window, a GPU or a draw call in the way. Each kernel is a Kestrel program with a `pub bench(n: int, iters: int) -> float` that makes `n` particles, runs the kernel over them `iters` times and returns a checksum. The numbers are nanoseconds per particle (per item), the median of 9 repetitions, with the cost of making the particles taken out by also timing a run with 0 iterations.

- `step.kes`: the demo's `Particle.step` (pull towards a target, damping, move, bounce off the edges), in a `par for` over a `list[Particle]`. `pow(KEEP, dt)` is worked out once and passed in.
- `step_scalar.kes`: the same step written by hand with `float` locals: the four fields are read once, the maths uses no `vec2` values, and the fields are written once. This is the speed the compiler should reach on `step.kes` by itself.
- `step_index.kes`: `step.kes` with a plain `for i from 0 to parts.len { parts[i].step(...) }` in place of the `par for` (see "Range analysis" at the end).
- `step_pow.kes`: the step exactly as the demo writes it, with `pow(KEEP, dt)` for every particle.
- `copy.kes`: the demo's draw copy, every particle's position into a `list[vec2]`.
- `src/bin/rust.rs`: the same three kernels in Rust (`step`, `pow` and `copy`), one thread, same arithmetic.
- `iface_hp.kes`, `iface_step.kes`: calls through a `list[Damageable]` (see "Interface calls" at the end), each with an `_direct.kes` version over a `list[Goblin]`, and `dyn_hp`, `dyn_step`, `direct_hp` and `direct_step` modes in `src/bin/rust.rs`.
- `filter.kes`, `map.kes`, `sort_by.kes`: the list methods that take a closure (see "The list methods" at the end), each with a hand-written `_loop.kes` version doing the same work, and `filter`, `map` and `sort_by` modes in `src/bin/rust.rs`.
- `noise_scalar.kes`, `noise_corners.kes`, `noise_wide.kes`: one 2D value noise written three ways (plain `int`s, the four corners in one `ivec4`, four samples in one `ivec4`/`vec4`; see "SIMD vectors" at the end), with `noise_scalar` and `noise_wide` (SSE4.1 intrinsics) modes in `src/bin/rust.rs` from `src/noise4.rs`. Here an item is one noise sample.

## How to run it

From `kestrel/bench/kernels` (it is its own Cargo workspace, listed in `exclude` in `kestrel/Cargo.toml`, so the compiler's tests do not build it):

```
cargo build --release
py measure.py --rounds 3 --threads 1
py measure.py --rounds 3 --threads 16
```

`measure.py` runs every kernel for a few rounds, one after the other so they share the machine's conditions, and prints the median of the rounds' medians with their range. `--threads` sets `KESTREL_THREADS` for the Kestrel kernels, which is the thread count of their `par for`; the Rust kernels always use one thread. `py measure.py --rounds 9 --only iface` runs just the kernels whose label contains the text (here the interface ones, Kestrel and Rust). `py measure.py --rounds 7 --keep-checks --old DIR` adds a column with the range analysis off and one for an older build (`DIR` is its `target/release`); `kernels.exe` takes `--keep-checks` too. A single kernel can be run with `target/release/kernels.exe step.kes --release`, and takes `--n`, `--iters`, `--reps` and, without `--release`, checks integer overflow as `kestrel run` does.

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

**The fix.** The two passes' struct-store rule used to write a built struct into memory as one `Assign` per leaf, and codegen computed the place again for each one: its own bounds check on `positions[i]` and its own reload of the list's length and data pointer. Now `split.rs` writes a plain struct value into memory as ONE `Assign` of a `StructLit` whose fields are constants and `Value` locals (all computed first, so the value still can't read a half-written place, and the destination's index expressions still run after the value), and codegen reads those values first, then finds the place once and stores each into it (it does not build the struct in a temporary). A store into a split local is still one assignment per leaf. `positions[i] = p.pos` is one bounds check again (`a_struct_store_into_a_list_checks_the_index_once` counts the `trap user2` paths in the IR: 2 before the fix, 1 after).

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

## The list methods (`filter`, `map`, `sort_by`)

Measured on 2026-10-18 with the compiler at commit 29f802a (the end of the closures round), on the same machine as above (Ryzen 7 5800X, Windows 11), one thread, `--release`.

The claim to check is that a closure literal passed to a list method, which is specialised and inlined, "runs like a hand-written loop". Three kernels, each three ways: the list method, a hand-written Kestrel loop doing the same work, and Rust.

- **`filter`** (`filter.kes`, `filter_loop.kes`, Rust `iter().filter().collect()`): 1,000,000 ints from the generator `x = (x * 75 + 74) % 65537` (so the parity is not a pattern the branch predictor can learn), keeping the evens. About half are kept, so each pass makes a list of about 500,000 ints. The method is `xs.filter((e) { e % 2 == 0 })`; the loop is `for e in xs { if e % 2 == 0 { ys.add(e) } }`.
- **`map`** (`map.kes`, `map_loop.kes`, Rust `iter().map().collect()`): 1,000,000 `vec2` to their lengths: `vs.map((v) { v.length() })` against a loop that `add`s `v.length()`. The Rust version is `(x * x + y * y).sqrt()` on `f32`.
- **`sort_by`** (`sort_by.kes`, `sort_by_loop.kes`, Rust `sort_by(|a, b| a.x.total_cmp(&b.x))`): 100,000 `struct Item { x: float, id: int }` whose float key is the same generator's value. Every pass copies the items into a fresh list and sorts the copy; the checksum reads the sorted order. Kestrel's `sort_by` takes a key, not a comparator, and both it and Rust's `sort_by` are stable. There is no hand-written loop that *is* a sort, so `sort_by_loop.kes` is a bottom-up merge sort I wrote (stable, two lists, runs of 1, 2, 4, ... items). A better hand-written sort would be a tougher comparison.

All three versions of each kernel give the same checksum (checked at `--n 100000 --iters 3`; the Rust modes print theirs). In the Kestrel kernels each pass's result is made in its own `arena` that is freed at the end of the pass; in Rust each result is a `Vec` that is dropped. The numbers are ns per input item, the median of 9 rounds with the range of the rounds in brackets (`py measure.py --rounds 9 --threads 1`, which runs every kernel one after the other in each round). The whole set was run twice; the table gives both runs. `measure.py` passes `--n 1000000` (`filter`, `map`) or `--n 100000` (`sort_by`) and 10 iterations. The machine was quiet: the ranges are 1-3%. Still, treat differences under 10% between two different programs as noise, because code layout moves timings by 6-30% here.

| Kernel, ns per item | The list method | Hand-written Kestrel loop | Rust |
|---|---:|---:|---:|
| `filter`, run 1 | 4.82 (4.79-4.87) | 3.91 (3.89-3.95) | 2.67 (2.61-2.70) |
| `filter`, run 2 | 4.83 (4.78-4.87) | 3.90 (3.89-3.92) | 2.67 (2.61-2.70) |
| `map`, run 1 | 1.39 (1.36-1.40) | 1.35 (1.34-1.36) | 0.77 (0.76-0.90) |
| `map`, run 2 | 1.36 (1.35-1.40) | 1.36 (1.34-1.38) | 0.77 (0.75-0.78) |
| `sort_by`, run 1 | 139.5 (139.3-140.2) | 97.8 (97.6-101.9) | 22.8 (22.7-23.2) |
| `sort_by`, run 2 | 139.2 (138.7-139.9) | 97.6 (97.0-98.6) | 22.8 (22.7-22.9) |

| Ratio of the medians (both runs agree) | Method over hand loop | Method over Rust | Hand loop over Rust |
|---|---:|---:|---:|
| `filter` | **1.23-1.24** | 1.81 | 1.46 |
| `map` | 1.00-1.03 | 1.77-1.80 | 1.76-1.77 |
| `sort_by` | **1.43** | 6.1 | 4.3 |

**What it means.**
- **`map` runs like a hand-written loop**: 1.00-1.03 times the loop, inside noise. The claim holds for it. Both are about 1.8 times Rust, which can vectorise this loop (0.77 ns an item); Kestrel does one item at a time.
- **`filter` does not quite: it is 23-24% slower than the hand loop**, the same in both runs, and the ranges of the rounds (4.78-4.87 against 3.89-3.95) do not touch, so it is not noise. It is 1.8 times Rust's `collect()`; the hand loop is 1.46 times.
- **`sort_by` is 1.43 times a hand-written merge sort and 6.1 times Rust's sort.** That is a loss on both counts. Even the hand-written merge sort is 4.3 times Rust, so most of the gap to Rust is the speed of Kestrel's sort loops in general, not the method. The method's extra 43% over the hand-written sort I did not take apart. From reading `kir/src/lower/sort.rs` and `runtime/src/sort.rs`: the closure is not inlined into a sort loop. The runtime sorts a vector of positions and calls a generated `compare(env, a, b)` through a function pointer; the closure is specialised into that compare function, which computes the key of each side (so twice per comparison). Rust's `sort_by` inlines the comparison into its loop and compares two floats once.

**Why `filter` is slower** (`kernels.exe filter.kes --release --dump bench` against the same for `filter_loop.kes`). The closure is inlined: there is no call in the loop, and the test is the same instructions in both. The differences are in the code around it:
- In the hand loop the list's header is held in a register and the item loaded for the test is the item that is added. In the method's loop the header is in a stack slot (`rsp+120`) and is loaded twice per item: once for the data pointer of the test, and again in the branch that keeps the item, where the item is loaded a second time (IR values `v57` and `v72` are the same address). Cranelift did not merge the two loads, with a store to the new list's header between them (I did not find out whether it could not tell the two places apart, or just doesn't forward loads across a store).
- The loop also has a dead load of the closure's environment word (`v51 = load v38+8`, `movq 8(%r14), %r9`, overwritten at once). The literal's environment is still built on the stack, two stores per pass, although the inlined code never reads it.

Together that is a spill reload, two or three more loads per item and a little more register pressure, about 0.9 ns an item. It is not a bug; the code is right and only slower. What would close it is making the inlined literal bind the item once, as the loop does, and dropping the environment an inlined literal does not read. That is not done here; this round only measured it.

**In short.** `map` matches its hand loop; `filter` is about a quarter slower than its hand loop; `sort_by` is 1.4 times a hand merge sort. Against Rust all three lose: 1.8 times, 1.8 times and 6.1 times. For `filter` and `map` the hand-written loops lose to Rust by about the same factors (1.5 and 1.8), so for those two the method is not the main cost; for `sort_by` a plain loop is 4.3 times Rust and the method is 6.1 times.

## Interface calls

Measured on 2026-10-19 with the compiler at commit 93b4e3e (the end of the interfaces build, plus an uncommitted edit to `check/src/body/enums.rs` that was in the working tree and that these kernels don't touch), on the same machine as above (Ryzen 7 5800X, Windows 11), one thread, `--release`. It was a desktop with whatever else the user had open, and nothing of mine running but the measurement (the idle-priority watcher that keeps cargo out of the way was stopped before timing). The ranges are 1-3% except one round of `iface_hp.kes` and `iface_hp_direct.kes` in run 2, which was disturbed (7.63 and 1.07). Treat differences under 10% between two different programs as noise, as above.

The claim to check is that a call through an interface costs an indirect call and nothing else: no allocation in the loop, and a price in the nanoseconds. Two kernels, each three ways: through a `list[Damageable]`, over a `list[Goblin]` (a direct call the compiler can inline), and in Rust.

- **`hp`** (`iface_hp.kes`, `iface_hp_direct.kes`, Rust `dyn_hp` and `direct_hp`): 1,000,000 items, `sum += d.hp()` over all of them, 20 passes. The interface list holds `Goblin`s and `Slime`s (`hp` returns a field; `Slime.hp` adds 1, see below), chosen by the parity of the same generator the list kernels use (`x = (x * 75 + 74) % 65537`), so the branch predictor can't learn the order. Rust is a `Vec<Box<dyn Damageable>>` built the same way, against a `Vec<Goblin>`.
- **`step`** (`iface_step.kes`, `iface_step_direct.kes`, Rust `dyn_step` and `direct_step`): the same lists, `d.step()` on every item (`Goblin` adds 1 to its field, `Slime` adds 3: a `var self` call that writes through the pointer), 20 passes, and the sum of `hp()` at the end as the checksum.

Kestrel and Rust give the same checksum for each pair (checked at `--n 1000000 --iters 3`). The numbers are ns per item (one call each), the median of 9 rounds with the range in brackets (`py measure.py --rounds 9 --only iface`); the whole set was run twice and the table gives both runs.

| Kernel, ns per item | Kestrel, interface list | Kestrel, `list[Goblin]` | Rust `dyn` | Rust `Vec<Goblin>` |
|---|---:|---:|---:|---:|
| `hp`, run 1 | 5.26 (5.20-5.52) | 0.74 (0.72-0.78) | 4.42 (4.34-4.65) | 0.07 (0.06-0.11) |
| `hp`, run 2 | 5.21 (5.11-7.63) | 0.73 (0.70-1.07) | 4.42 (4.21-4.75) | 0.07 (0.06-0.14) |
| `step`, run 1 | 5.24 (5.18-5.33) | 0.50 (0.49-0.54) | 4.54 (4.48-4.91) | 0.07 (0.07-0.12) |
| `step`, run 2 | 5.23 (5.11-5.79) | 0.49 (0.48-0.59) | 4.60 (4.32-5.15) | 0.07 (0.07-0.09) |

| Ratios of the medians (both runs agree) | Interface over direct, Kestrel | Kestrel interface over Rust `dyn` | Kestrel direct over Rust direct |
|---|---:|---:|---:|
| `hp` | 7.1 | **1.18-1.19** | 10.4-10.6 |
| `step` | 10.7 | **1.14-1.15** | 7.0-7.1 |

**What it means.**
- **A call through an interface costs about 4.5-4.7 ns more than a direct call, in a loop whose order the predictor can't learn.** That is 7 to 11 times the direct loop, which the compiler inlines to a load and an add (0.5-0.7 ns).
- **Most of it is the mispredicted jump, not the table.** With the types in two blocks (all `Goblin`s, then all `Slime`s: a scratch variant of `iface_hp.kes`, not kept) the same interface loop takes 2.2 ns, three runs of 2.22-2.25. So the random order costs about 3 ns an item, and the predictable indirect call about 1.5 ns more than the inlined one. A game's list is usually somewhere between the two.
- **Against Rust's `Box<dyn>`, Kestrel loses by 14-19%** (5.2 against 4.4 and 4.5-4.6). That is a loss, and it is the same in both runs and outside the ranges. The gap is smaller than I expected, since both loops are an indirect call per item with a mispredicted jump about half the time. Reading the IR and machine code (`kernels.exe iface_hp.kes --release --dump bench` and `--dump Slime.hp`), the likely reasons, not separately timed:
  - Kestrel's method is a whole function with a frame: `pushq %rbp; movq %rsp,%rbp` ... `popq %rbp; ret` around one load (and one add). Rust's is a bare load and return.
  - The loop reloads the list's data pointer from its header on every item (`movq 0(%rsi),%r8`), because the call might have changed the list, and keeps the running sum in a callee-saved register across the call. Rust's slice is in registers.
  - Kestrel's items are 16-byte (data, table) pairs, and their boxes are in the arena one after another in the order they were made; Rust's `Box<dyn>` is also 16 bytes and its boxes are separate allocations. This favours Kestrel if anything, so it isn't the cause of the loss.
- **Against a direct loop in Rust the loss is bigger and is not about interfaces.** Rust turns the `Vec<Goblin>` loop into a vectorised sum or add (0.07 ns); Kestrel's direct loops take 0.5-0.7 ns, the 10 and 7 times in the last column. The same factor shows in `map` above, where Kestrel does one item at a time.
- **No allocation in the loop:** `iface_memory.rs` has the test (100,000 calls, equal arena reports). The 1,000,000 boxes are made once, before the timed passes.

**A mistake this caught, kept as a warning.** The first version of `iface_hp.kes` gave `Slime.hp` the same body as `Goblin.hp` (`self.size`, a load from offset 0). Rust then ran at 2.8-3.0 ns, not 4.4: LLVM merges identical functions, so both vtable entries pointed at one address and the "random" indirect call was always predicted. Changing Rust's `Slime::hp` to `self.size + 1` moved it to 5.1-5.2 ns. Both languages' kernels now have the `+ 1`, so neither can fold the two methods. Kestrel does not merge them, so its first numbers (5.6-5.7 ns) were right; only the comparison with Rust was wrong. A test of virtual calls whose targets are identical measures the call, not the branch.

**Losses, in short.** Kestrel's interface call is 14-19% slower than Rust's `Box<dyn>` call, and 7-11 times a direct call it can inline (Rust's gap there is larger still, since it also vectorises). Devirtualising (a `CallVirtual` whose `MakeIface` is visible after inlining becoming a direct call) is still undone, and it would remove the call only where the type is known; it does nothing for a mixed list. A cheaper call would need a leaf function without a frame, and the data pointer of the list kept in a register across the call. Neither is done here; this round only measured.

## Range analysis

The range analysis (2026-10-19 and 2026-10-20, `docs/superpowers/specs/2026-10-19-range-analysis-design.md`) leaves out each overflow, shift, division, narrowing and bounds check it proves can't fail. Measured on 2026-10-20 (evening), the old compiler at commit b7166fe (the commit before the round, built in a `git worktree` with a short `CARGO_TARGET_DIR`) against the new one at dc78375 (the later commits only change docs), on the same machine as above (Ryzen 7 5800X, Windows 11), one thread, `--release`. The user had said the machine was free: no game was running, but the desktop had its usual background (Roblox's launcher open and idle, OneDrive sync, Defender), and the system sat at 25-40% CPU before the run started. No idle-priority watcher during timings. `py measure.py --rounds 7 --threads 1 --keep-checks --old DIR` ran the three compilers in turn within each round; the Rust kernels once per round. `--keep-checks` is the new compiler with `remove_checks: false`: every check stays, so it is the old compiler's code plus the new item limit. New: `step_index.kes`, a copy of `step.kes` whose loop is `for i from 0 to parts.len { parts[i].step(dt, target, keep) }` (one thread, no `par for`), the case the analysis was built for: 8 bounds checks per particle before.

**`--dump bench` (`trap user2` is one check):**

| Kernel | Old, `--release` / checked | New, `--release` / checked |
|---|---:|---:|
| `step_index.kes` | 29 / 31 | 0 / 0 |
| `copy.kes` | 1 / 5 | 0 / 0 |
| `step.kes` | 0 / 1 | 0 / 0 |

`--keep-checks` gives the old counts again. So `copy.kes` has no bounds check in its loop and `step_index.kes` has none anywhere in `bench` (which includes the spawn loop).

**The kernels you were asked about** (ns per item, median of 7 rounds, range in brackets):

| Kernel | New | New `--keep-checks` | Old | New over old |
|---|---:|---:|---:|---:|
| `step_index.kes` | **8.28** (8.15-8.48) | 11.03 (10.86-11.32) | 11.13 (11.05-11.30) | **0.74** |
| `step.kes` (`par for`, one thread) | 5.80 (5.61-6.00) | 5.82 | 5.80 (5.59-6.17) | 1.00 |
| `copy.kes` | 0.92 (0.91-0.98) | 1.21 (1.18-1.30) | 0.98 (0.94-1.04) | 0.94 |
| Rust `step` / `copy` | 5.08 (4.94-5.19) / 0.27 (0.26-0.28) | | | |

- **`step_index.kes` is 26% faster, 8.3 ns against 11.1** (both `--keep-checks` and the old compiler: 11.0 and 11.1), the same in three code layouts (7.9-8.1 against 10.6-11.0 with one or two unused functions added). It is still **1.43 times `step.kes`** (5.80, was 1.92 times) and 1.63 times Rust's step (was 2.2). The 2.5 ns that remains is, most likely (I did not take it apart), the loop reading the list's data pointer from the header for each `parts[i]`, and `parts[i].step()` working on the element in memory, not on promoted fields: the header reload and promoting an indexed place are the two things the design left out.
- **`copy.kes` has no bounds check in its loop, and is 0.94 times the old time: not a clear win.** In three layouts (as written, and with one or two unused functions added, which only move the code) it takes 0.93, 0.91 and 0.92 ns now; the old compiler took 0.95, 1.17 and 1.20 and `--keep-checks` 1.19, 1.20 and 0.92. So with the check gone the loop is steady, and the old loop was 0-24% slower depending on where the compiler put it. The ranges of the as-written cells overlap (0.87-1.13 against 0.87-1.36); the gain is real in two of the three layouts and absent in the third. It is **3.4 times Rust's 0.27 ns** (3.6 before): the check was never most of the gap. The design's expected reason is the data pointer reloaded after every store (Cranelift can't prove the store didn't change the header); I did not confirm it in the machine code.
- **`step.kes` is unchanged** (it indexes nothing; the one check the old checked build had is gone, which this timing doesn't show).

**Every other kernel** (ns per item; new / `--keep-checks` / old; median of 7, ranges 1-5% unless noted):

| Kernel | New | `--keep-checks` | Old | New over old |
|---|---:|---:|---:|---:|
| `step_scalar.kes` | 5.88 | 5.82 | 5.89 | 1.00 |
| `step_pow.kes` | 23.40 (23.02-26.50) | 23.06 | 23.26 | 1.01 |
| `filter.kes` | 6.48 | 6.42 | 6.36 | 1.02 |
| `filter_loop.kes` | 5.70 | 5.69 | 5.38 | **1.06** |
| `map.kes` | 2.07 | 2.05 | 2.09 | 0.99 |
| `map_loop.kes` | 2.07 | 2.09 | 2.03 | 1.02 |
| `sort_by.kes` | 192.45 | 193.04 | 194.27 | 0.99 |
| `sort_by_loop.kes` | 126.81 | 131.73 | 130.78 | 0.97 |
| `iface_hp.kes` | 5.61 | 5.65 | 5.92 | 0.95 |
| `iface_hp_direct.kes` | 0.58 | 0.58 | 0.88 | **0.66** |
| `iface_step.kes` | 6.04 | 5.96 | 5.98 | 1.01 |
| `iface_step_direct.kes` | 0.87 | 0.86 | 0.59 | **1.47** |

Three differences are over 5%, and all three are code layout, not the analysis: the second column (`--keep-checks`) matches the new compiler to the digit on each of them (0.86 against 0.87, 5.69 against 5.70), so the analysis did not change them. With one or two unused functions added to the program (7 rounds each, new / old, ns):

| Kernel | As written | + 1 unused function | + 2 unused functions |
|---|---:|---:|---:|
| `iface_step_direct.kes` | 0.82 / 0.56 | 0.84 / 0.84 | 0.81 / 0.83 |
| `iface_hp_direct.kes` | 0.54 / 0.82 | 0.55 / 0.54 | 0.54 / 0.55 |
| `filter_loop.kes` | 5.47 / 5.13 | 5.35 / 5.45 | 5.10 / 5.31 |

The old compiler's `iface_step_direct.kes` loop is faster as written (0.56) and equal when padded; the new one is the same in all three. So **no kernel is slower beyond the layout noise**, and nothing in the table is a loss from the analysis. This is the same effect as in the 6-30% note in the follow-ups file: a 0.5 ns loop moves by 0.3 ns when code moves.

**Check counts** (`kestrel check --report`, the last line; checked / `--release`; totals count an inlined body in each function that holds it, so they are upper bounds on distinct checks):

| Program | Checks | Removed | Left |
|---|---:|---:|---:|
| `step.kes`, `step_scalar.kes`, `step_pow.kes` | 6 / 0 | 6 / 0 | 0 / 0 |
| `step_index.kes` | 62 / 58 | 62 / 58 | 0 / 0 |
| `copy.kes` | 10 / 2 | 10 / 2 | 0 / 0 |
| `filter.kes`, `map.kes` | 12, 14 / 2, 4 | 6, 6 / 0, 0 | 6, 8 / 2, 4 |
| `sort_by.kes` | 12 / 0 | 4 / 0 | 8 / 0 |
| `iface_hp.kes`, `iface_step.kes` | 9, 12 / 0 | 2, 4 / 0 | 7, 8 / 0 |

**What the numbers say.** The analysis does what the design said for the loop it was built for (`step_index.kes`: no check left, 26% faster), removes the draw copy's bounds check (a gain of 0-24% that depends on where the compiler puts the loop), and costs the other kernels nothing. The kernels with checks left (`filter`, `map`, `sort_by`, the interface ones) keep some checks (`kestrel check --report` lists them); I did not look at why. No check is left in `step_index.kes` or `copy.kes`, so their remaining distance to Rust is not checks; the list-header reload is the open candidate.

**`bench/2a` and `bench/heightmap`** (whole-process wall time in ms, including about 15 ms of process start; exes built by each compiler, run in turns, `Measure-Command` from PowerShell; median of 7 runs, 9 for the heightmap, range in brackets; checked / `--release`; same evening and conditions; each pair's outputs agree):

| Workload | Old checked | New checked | Old `--release` | New `--release` |
|---|---:|---:|---:|---:|
| fib | 281 (278-387) | 272 (269-376) | 271 (265-365) | 268 (267-373) |
| loop | 755 (740-848) | 754 (741-845) | 752 (738-846) | 747 (741-853) |
| collatz | 348 (340-463) | 346 (339-452) | 281 (275-377) | 283 (275-375) |
| mandelbrot | 499 (481-594) | 485 (475-584) | 479 (478-584) | 483 (482-586) |
| particles | 169 (167-275) | 173 (168-269) | 212 (210-323) | 213 (209-314) |
| sieve | 901 (855-1,008) | 878 (860-1,005) | 883 (852-969) | 896 (852-989) |
| swarm | 200 (193-303) | 201 (193-310) | 201 (195-298) | 201 (189-306) |
| heightmap | 82 (78-186) | **61 (60-166)** | 61 (58-159) | 58 (56-157) |

- **Nothing is slower beyond noise** (the largest difference among the 2a rows is 3%, with the ranges overlapping); the high ends of the ranges are single slow runs, on both compilers alike.
- **The checked heightmap is 26% faster, 82 ms to 61**, and now equals the old `--release` build (61); its `--release` build is unchanged (61 to 58). `kestrel check --report` says 114 of its 127 checks are removed, and 7 of 7 under `--release`.
- The analysis proves few of the 2a programs' checks, as the report shows: fib 2 of 3 removed, loop 1 of 2, collatz 0 of 3, mandelbrot 1 of 3, sieve 4 of 5, swarm 2 of 2. None of that moved their time.

## SIMD vectors (2026-10-24)

What `vec4` and `ivec4` (the SIMD vectors round, `docs/superpowers/specs/2026-10-21-simd-vectors-design.md`) buy on a 2D value noise: one noise written three ways in Kestrel, and twice in Rust. Compiler at b2b4535 (the last commit of the round's Task 5), Ryzen 7 5800X, Windows 11, one thread.

**The kernels.** `hash(x, y, seed)` is the 32-bit wrapping hash of `tests/programs/prove_vector_hash.kes` (three wrapping multiplies, xors and arithmetic shifts after mixing the inputs), `lattice(h) = float(h & 0xFFFF) / 65535.0`, `smooth(t) = t * t * (3.0 - 2.0 * t)`. A sample at `(x, y)` hashes the four lattice corners around `(x >> 4, y >> 4)` and blends them by the smoothed fractions of `x & 15` and `y & 15`. `bench(n, iters)` sums `n * iters` samples (`x = i & 1023`, `y = (i >> 10) + k`) into an `f64` in sample order, so all versions give the same sum to the last bit (8383.914960693568 for `n` 4096, `iters` 4, and 393774.1092315486 for 262,144 and 3; checked, `--release` and both Rust modes alike). Timed at `n` 262,144 (256 rows of 1,024), `iters` 10.

- `noise_scalar.kes`: plain `int`s and `float`s, four `hash` calls per sample.
- `noise_corners.kes`: the four corners at once: one `hash4` over `ivec4(cx) + ivec4(0, 1, 0, 1)` and `ivec4(cy) + ivec4(0, 0, 1, 1)`, one `vec4` of lattice values, the blend from its lanes.
- `noise_wide.kes`: four samples at once (`x .. x + 3` of one row in the lanes): four `hash4` calls, one per corner, and the blend lane by lane in `vec4`s; the four lanes are added to the sum one by one.
- Rust (`src/noise4.rs`): `noise_scalar` is the scalar kernel in `i32` (`wrapping_mul`) and `f32`; `noise_wide` is `noise_wide.kes` with SSE4.1 intrinsics (`_mm_mullo_epi32`, `_mm_srai_epi32`, `_mm_cvtepi32_ps`, `_mm_div_ps` ...). Both are the default `--release` target (SSE2 baseline, SSE4.1 only inside the `#[target_feature]` function); the Kestrel JIT uses the host's AVX encodings.

**The machine was busy.** Every timing below ran while another program (a Python job at about five cores) and a game (about three cores) kept the system at 60-90% CPU; I could not get a quiet window. The kernels are one thread and ran one at a time, in turns within each round, so the ratios are fair, but each cell's range is wide (often 20-30%) and the absolute times are probably 20-40% above a quiet machine's: a first look in a calmer moment gave 2.70 ns for `noise_wide.kes` and 2.46-3.00 for Rust's `noise_wide`. **A quiet rerun is owed** (`py measure.py --rounds 9 --only noise --threads 1`).

**Times** (ns per sample, median of 15 rounds, range in brackets; `py measure.py --rounds 15 --only noise --threads 1`, run twice; run 1 also had the `--keep-checks` column):

| Kernel | Checked | `--release` | Checked (run 1) | `--release` (run 1) | `--keep-checks`, checked (run 1) |
|---|---:|---:|---:|---:|---:|
| `noise_scalar.kes` | 11.86 (10.47-13.25) | 11.57 (10.26-13.57) | 11.58 (10.00-13.84) | 11.78 (10.74-13.19) | 16.23 (14.19-18.62) |
| `noise_corners.kes` | 8.10 (6.53-9.03) | 7.89 (7.29-9.11) | 8.11 (6.89-9.02) | 8.24 (6.78-9.14) | 9.57 (7.80-10.72) |
| `noise_wide.kes` | 3.76 (3.13-3.96) | **3.67** (3.14-3.96) | 3.74 (3.19-4.20) | 3.67 (3.26-4.11) | 5.01 (3.72-6.54) |
| Rust `noise_scalar` | | 10.29 (9.31-11.51) | | 10.48 (8.86-12.92) | |
| Rust `noise_wide` (SSE4.1) | | **3.58** (2.90-3.89) | | 3.47 (2.87-3.82) | |

Two 9-round runs earlier the same afternoon agree (`--release`: scalar 10.86 and 11.04, corners 6.37 and 7.71, wide 3.37 and 3.49, Rust scalar 9.82 and 9.26, Rust wide 3.03 and 3.24). Code layout (7 rounds, `--release`, as written / one unused function added / two): scalar 10.21 / 11.00 / 10.16, corners 6.47 / 7.18 / 6.81, wide 3.39 / 3.31 / 3.45, so nothing here is a layout effect beyond the noise.

**Speedups** (from the medians of both runs):

- **Four samples at once: 3.15-3.2 times scalar Kestrel**, checked or `--release` (11.57 to 3.67). The guess was 2-3 times; it is a little over.
- **Four corners at once: 1.43-1.47 times scalar.** Below the guess: see below.
- **`noise_wide.kes` against Rust's hand-written SSE4.1: 1.03-1.06 times its time**, within the noise (the cells' ranges overlap almost entirely). It is 2.8 times faster than Rust's scalar loop (10.29 against 3.67).
- **`noise_scalar.kes` against Rust's scalar: 1.12 times its time** in both runs.
- Rust's own wide over scalar: 2.9-3.0 times. Kestrel's gain from `vec4`/`ivec4` (3.15-3.2) is the same size.

**The checked build costs nothing measurable on this vector code:** checked over `--release` is 0.98-1.03 for every kernel. The range analysis removes every lane check (`kestrel check --report`: `noise_wide.kes` 46 checks, 45 removed; `noise_corners.kes` 18 and 17; `noise_scalar.kes` 40 and 39); the one left in each is the scalar `(i >> 10) + k` in the loop head (`k` is unknown). Even `noise4` compiled on its own, not inlined, keeps no check: `x >> 4` is at most 2^27, so `cx + 1` can't overflow a lane. With every check kept (`--keep-checks`), the checked wide kernel takes 1.34 times as long (5.01 against 3.74) and scalar 1.40 times (16.23 against 11.58): the per-lane checks cost about what scalar checks cost, in proportion. That column overstates the lane checks, though: of about 20 compare-and-branches in the wide loop under `--keep-checks`, only 3 are lane checks (`vpxor`, `vpxor`, `vpand`, `vmovmskps`, `test`, `jnz`, six instructions each) and one a scalar `seto`; the rest are checks whose condition Cranelift folded to a constant false but whose branch it kept (`xorl %esi, %esi; testb %sil, %sil; jnz`).

**What the dump shows** (`target/release/kernels.exe noise_wide.kes --release --dump bench`; `hash4` and `noise4` are inlined into `bench`; the inner loop's machine code, not counting its three-instruction loop head):

| Loop | Instructions per sample | Integer multiplies | Divides | Notes |
|---|---:|---:|---:|---|
| `noise_scalar.kes` | 123 | 12 `imull` | 6 `vdivss` | |
| `noise_corners.kes` | 68 | 4 `vpmulld` | 1 `vdivps`, 2 `vdivss` | lanes 1-3 read with `vpshufd` |
| `noise_wide.kes` | 30.8 (123 per 4) | 10 `vpmulld`, 2 `imull` per 4 | 6 `vdivps` per 4 | 3 constants reloaded from the stack |
| Rust `noise_scalar` | 113 (with loop control) | 10 `imull` | 2 `divps` (two lanes each) | |
| Rust `noise_wide` | 35.2 (141 per 4) | 10 `pmulld` per 4 | 4 `divps` per 4 | 36 register copies (two-operand SSE) |

- **The wide loop is what was wanted:** `vpmulld`, `vpxor`, `vpsrad` on whole registers, no `explicit_slot`, no lane extract in the hash; the only lane reads are the three `vpshufd` that feed the `f64` sum (which the in-order sum needs). Cranelift did two good things unasked: the corners that share `cx` (and `cx + 1`) share the `x` products (10 `vpmulld` instead of 12), and because `y` is the same in every lane it multiplies `cy` as a scalar `imull` and broadcasts it.
- **The corners loop reads its lanes where expected:** lane 0 straight from the register, lanes 1-3 by one `vpshufd` each, right after the one `vdivps`.
- **Why the corners only gain 1.45 times:** it has 55% of the scalar loop's instructions but one hash chain instead of four independent ones, so each sample waits on one long chain (multiply, shift, xor, multiply ... convert, divide, then a scalar blend that waits on the lane reads). The scalar loop's four chains overlap. Doing four samples at once keeps the chains independent and wide, hence 3.2 times.
- **Where Kestrel loses to Rust, a little:** (1) the e-graph rewrites `h = a ^ C; h ^ (h >> 15)` into `a ^ (((a ^ C) >> 15) ^ C)`, one `xor` more per hash (4 per iteration of the wide loop, 4 per sample in the scalar one); (2) `/ 16.0` stays a divide (LLVM multiplies by 0.0625), 2 divides more per iteration or sample: replacing it by hand with `* 0.0625` changed nothing measurable (wide 3.43 against 3.69, scalar 11.34 against 11.13, 9 rounds in turns); (3) in the wide loop 3 vector constants don't fit in the 16 `xmm` registers and are reloaded from the stack each iteration, and in the scalar and corners loops the `float` constants 2.0 and 3.0 are rebuilt (`movl`, `vmovd`) each sample; (4) for the scalar loop, LLVM turns `(cx + 1) * C` into `cx * C + C` (10 multiplies against Kestrel's 12) and pairs two of the lattice divides into one two-lane `divps` (SLP vectorising). None of these is large: the wide loop is level with Rust's within the noise, and the scalar one 12% behind.
- **Rust's scalar loop is slower than its instruction count suggests:** LLVM converts three of the four lattice values with `cvtsi2ss` into registers it doesn't clear first (`xmm9`, `xmm10`, `xmm11`), a false dependency on the previous iteration's divide. Kestrel clears each target with `vxorps` first. I did not confirm that this is what costs Rust's loop its time.

**In short:** four samples at once in `vec4`/`ivec4` are 3.2 times scalar Kestrel and level with hand-written SSE4.1 Rust; four corners at once are 1.45 times. The checked build costs nothing on this code because the analysis proves every lane check. The small gaps to Rust (an extra `xor` per hash, a divide by 16, spilled constants) are in `docs/superpowers/plans/2026-10-09-core-2a-followups.md` under "Found while measuring SIMD vectors".
