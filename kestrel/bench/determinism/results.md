# Fast float math (2026-10-28)

Measured 2026-10-29 on the user's PC (Ryzen 7 5800X, 8 cores, 16 threads, Windows 11), the PC
free. `float` transcendental math (`sin cos tan asin acos atan atan2 exp log pow` and float `%`)
is now Kestrel's own wide functions (computed in `f64`, rounded once to `float`), inlined
branch-free but for one special-input branch, except `pow`, which stays a call; pure math whose
operands a loop never changes is computed once before the loop. Spec:
`docs/superpowers/specs/2026-10-28-fast-math-design.md`; plan:
`docs/superpowers/plans/2026-10-28-fast-math.md`.

**Short version:** `sin`, `cos`, `tan`, `atan2` and `log` met their per-call targets, and the
`pow`-per-particle kernel went from 33.4 to 4.4 ns (`pow` hoisted). `exp`, `pow` and float `%`
got faster but **missed** their targets. `asin`, `acos` and `atan` got **26-28% slower**, which
the spec forbids. `trig_step.kes` **missed** its 22 ns target by 0.3 ns (22.3). Every program
without hot math stayed within noise. Accuracy is at most 0.501177 ulp, and the results hash is
the same in release, debug and `target-cpu=native` builds.

## How it was measured

**Two compilers**, each built `--release` (and `bench/kernels` with each) in a `git worktree`
(`C:/kt/fm-before`, `C:/kt/fm-after`, removed afterwards):

- **before**: 49ae9f0, the commit before the round (`float` math widened to `f64` `libm` calls);
- **after**: 3a1a713, the round's last code commit.

Both compiled the same files, from the main checkout (the `math_<op>.kes` kernels and
`trig_step.kes` are plain Kestrel, which the old compiler compiles too). The method is the
determinism round's (`measure.py`, run from this folder; raw outputs in
`.superpowers/sdd/2026-10-28-fast-math/task-6b-raw/`, local only): every compiler in turn within
each round, at normal priority, nothing else of mine running, medians of 9 rounds (11 for the
exes, 21 and 31 for compile times) with the range. **Load** is the whole machine's CPU use, 2 s
before and after each block and over the whole block ("during" includes the timed process: one
busy thread of 16 is 6.25%). Another agent had a Godot editor open (spinning one thread) when the
session started; no timing ran until it had closed, and `tasklist` showed no Godot before or after
any block below. The per-function and `trig_step` blocks were run twice; the second run is within
2% of the first everywhere.

## Per call: the `math_<op>.kes` kernels (G3)

Each kernel fills a list (two for `atan2`, `pow`, `%`) with 65,536 random inputs from a seeded
`Rng` in the spike's ranges, then sums the function over it 100 times, through the JIT
(`bench/kernels`); the operands change every item, so the inline code is what is timed. ns per
item, median (range). **`math_none.kes`** sums `x` itself (the loop and the add: 0.63 ns before
and after); **`math_none2.kes`** sums `x + ys[i]` over two lists (two loads, a bounds check and
the adds: 0.64 ns before and after). One-argument functions are given net of `math_none`,
`atan2`, `pow` and `%` net of `math_none2` (net of `math_none` they are 0.01 ns more). The
target is the spec's, for the time per call; both ways are shown, since the loop's add chain
overlaps the math.

Load 4.6% before, 12.7% during, 9.4% after (second run: 3.9%, 8.9%, 0.9%).

| function | before | after | after / before | net before | net after | target | met? |
|---|---:|---:|---:|---:|---:|---:|---|
| `sin` | 15.17 (15.10-15.28) | **4.25** (4.20-4.30) | 0.280 | 14.54 | 3.62 | ≤ 6 | yes |
| `cos` | 15.28 (15.25-15.36) | **3.81** (3.79-3.85) | 0.249 | 14.65 | 3.18 | ≤ 5.5 | yes |
| `tan` | 15.98 (15.94-16.10) | **5.13** (5.12-5.16) | 0.321 | 15.35 | 4.50 | no slower | yes |
| `asin` | 8.65 (8.60-8.69) | 10.86 (10.84-11.01) | **1.255** | 8.02 | 10.23 | no slower | **no** |
| `acos` | 8.38 (8.32-8.44) | 10.60 (10.59-10.77) | **1.265** | 7.75 | 9.97 | no slower | **no** |
| `atan` | 7.16 (7.11-7.20) | 8.99 (8.95-9.11) | **1.256** | 6.53 | 8.36 | no slower | **no** |
| `atan2` | 20.49 (20.44-20.71) | **10.26** (10.23-10.45) | 0.501 | 19.85 | 9.62 | ≤ 11 | yes |
| `exp` | 7.37 (7.36-7.45) | 5.58 (5.58-5.68) | 0.757 | 6.74 | 4.95 | ≤ 4 | **no** |
| `log` | 6.11 (6.10-6.26) | **4.38** (4.36-4.47) | 0.717 | 5.48 | 3.75 | ≤ 4.5 | yes |
| `pow` (a call) | 37.63 (37.52-38.26) | 24.31 (24.22-24.74) | 0.646 | 36.99 | 23.67 | ≤ 12 | **no** |
| float `%` | 5.71 (5.66-5.74) | 5.03 (5.02-5.08) | 0.881 | 5.07 | 4.39 | ≤ 3 | **no** |

The before column matches the spec's "today" (the spike's figures, 15.4 for `sin` to 36.1 for
`pow`) within 1-4%.

**Supplement: the Rust functions alone** (`cargo test -p kestrel-runtime --release wide_math_speed
-- --ignored --nocapture`, `runtime/src/math/wide_speed.rs`): 65,536 inputs times 153 passes, 9
interleaved rounds, each function called through an `extern "C"` pointer. "today" is 49ae9f0's
path (`libm` in `f64`, widened and narrowed; `libm::fmodf` for `%`), "wide" the `kestrel_rt_<op>f`
function, which is the inline code's slow path and `pow` itself. ns per call, median (range); the
harness alone (an identity function) is 1.04 ns with one argument and 1.86 with two. Random
inputs: load 8.0% before, 8.7% during, 2.8% after; in order (`ORDER=seq`): 2.9%, 8.6%, 2.5%.

| function | today, random | wide, random | today, in order | wide, in order |
|---|---:|---:|---:|---:|
| `sin` | 13.30 (13.29-13.67) | 12.03 (12.02-12.21) | 6.20 (6.17-7.94) | 9.30 (9.30-9.32) |
| `cos` | 13.93 (13.74-14.13) | 6.51 (6.51-6.70) | 6.73 (6.72-6.75) | 3.02 (3.00-3.03) |
| `tan` | 14.90 (14.89-14.92) | 11.07 (11.07-11.08) | 9.27 (9.23-9.31) | 11.07 (11.07-11.08) |
| `asin` | 10.72 (10.64-10.74) | 12.13 (12.10-12.26) | 7.99 (7.98-8.14) | 6.67 (6.67-6.89) |
| `acos` | 9.58 (9.51-9.63) | 13.08 (12.84-13.11) | 7.05 (7.05-7.09) | 6.27 (6.27-6.32) |
| `atan` | 6.86 (6.40-6.86) | 4.25 (4.23-4.39) | 4.84 (4.78-4.88) | 4.11 (4.09-4.14) |
| `atan2` | 19.83 (19.79-20.26) | 13.20 (13.19-13.40) | 8.71 (8.70-8.81) | 8.39 (8.38-8.48) |
| `exp` | 6.83 (6.82-6.85) | 3.25 (3.24-3.25) | 6.76 (6.75-6.82) | 3.25 (3.25-3.29) |
| `log` | 5.48 (5.47-5.56) | 3.46 (3.46-3.48) | 5.47 (5.47-5.49) | 3.46 (3.46-3.49) |
| `pow` | 35.62 (35.58-35.98) | 24.23 (24.22-24.44) | 33.10 (33.05-33.22) | 24.24 (24.23-24.25) |
| `%` | 5.11 (5.10-5.17) | 5.21 (4.61-5.46) | 4.46 (4.45-4.47) | 5.21 (5.20-5.23) |

What the two tables say together:

- **`pow`** is a call to the wide Rust function in both tables, 24.2-24.3 ns: the inline route
  was a non-goal, and the target of 12 needed a faster `powf` (followups entry 52).
- **Random inputs are hard on `libm`** (its branches mispredict): `sin` today is 13.3 ns random
  and 6.2 in order. The kernels use random inputs, as the spec says. In order, the inline `sin`
  and `cos` (branch-free, so the same speed either way) would still be ahead of today's.
- **For `atan`, `exp` and `log` the Rust function, the same operations built by LLVM and called
  through a pointer, costs less than the inline Cranelift copy**: `atan` 4.25 ns with the call
  against 8.99 inline, `exp` 3.25 against 5.58, `log` 3.46 against 4.38 (the two timings use
  different loops, so this is a pointer, not a measurement of the fix). Why wasn't looked into
  here. The inline code is branch-free, so it pays for every range's reduction on every item,
  where today's `libm` branches; for `asin`, `acos` and `atan` that cost more than it saved.
  Calling the Rust function for `atan` and `exp` instead of inlining them may be the fix; it is
  not measured here.

## Kernels (G4)

`bench/kernels` through the JIT, 1 thread, 9 rounds; ns per item (per particle or mover).

| Kernel | before | after | after / before | target | met? |
|---|---:|---:|---:|---:|---|
| `step_pow.kes` (`pow(KEEP, dt)` per particle) | 33.44 (33.39-33.77) | **4.38** (4.34-4.41) | **0.131** | ≤ 5 | yes |
| `step.kes` (`pow` hoisted by hand) | 4.35 (4.33-4.37) | 4.36 (4.33-4.39) | 1.002 | | |
| `step_index.kes` | 5.65 (5.62-5.74) | 5.66 (5.62-5.68) | 1.002 | | |
| `step_scalar.kes` | 4.35 (4.32-4.38) | 4.36 (4.34-4.38) | 1.002 | | |
| `trig_step.kes` (`atan2`, `cos`, `sin` per mover) | 35.52 (35.12-35.73) | 22.34 (22.30-22.48) | 0.629 | ≤ 22 | **no** |
| `noise_scalar.kes` checked | 6.71 (6.66-6.73) | 6.71 (6.70-6.71) | 1.000 | ±2% | yes |
| `noise_scalar.kes --release` | 6.62 (6.62-6.63) | 6.63 (6.62-6.63) | 1.002 | ±2% | yes |
| `noise_corners.kes` checked | 5.45 (5.44-5.45) | 5.45 (5.44-5.45) | 1.000 | ±2% | yes |
| `noise_corners.kes --release` | 5.41 (5.41-5.44) | 5.42 (5.41-5.44) | 1.002 | ±2% | yes |
| `noise_wide.kes` checked | 2.70 (2.70-2.70) | 2.70 (2.70-2.70) | 1.000 | ±2% | yes |
| `noise_wide.kes --release` | 2.71 (2.70-2.71) | 2.71 (2.70-2.71) | 1.000 | ±2% | yes |
| `iface_step.kes` | 4.33 (4.30-4.36) | 4.32 (4.30-4.36) | 0.998 | | |

Load: step 4.6% before, 12.4% during, 4.8% after; trig 3.8%, 11.4%, 7.4%; noise 6.2%, 11.3%,
7.4%. `trig_step.kes` was 22.34, 22.33 and 22.31 ns in three blocks (the step block, the trig
block, and the second run with the per-function block), so it misses by 0.3 ns (1.5%). The spike
estimated 19.8; the inline `atan2` (9.6 ns net) is most of it. **`step_pow.kes` now matches
`step.kes`, which hoists `pow` by hand** (4.38 against 4.36): `pow(KEEP, dt)` runs once before the
loop. Rust references in the same rounds: `step` 3.75 ns, `noise_scalar` 6.10, `noise_wide` 2.40.

## Programs (G4)

**Exes** (the whole run minus the empty program's, about 6 ms; ms, median (range of the whole
runs)):

| | before | after | after / before |
|---|---:|---:|---:|
| frozen world `bench/voxels3d/main.kes`, checked | 419.1 (423.2-560.3) | 418.4 (422.5-538.5) | 0.998 |
| frozen world, `--release` | 375.8 (379.6-490.8) | 376.3 (379.3-429.7) | 1.001 |
| generation, 64 columns through `load_now`, `--release` | 302.0 (306.6-348.2) | 302.3 (306.9-347.2) | 1.001 |

Load: frozen world 7.0% before, 13.6% during, 4.3% after; generation 5.0%, 11.9%, 6.7%. Both
print the same results (463,749 faces; 18,500).

**Engine** (`kestrel/bench/engine`, `voxel/bench.kes --release` in a hidden window, 3 runs per
compiler in turn; work p99 per phase, ms, median (range)):

| Phase | before | after | after / before |
|---|---:|---:|---:|
| fill | 10.33 (9.80-11.63) | 9.48 (9.24-12.21) | 0.918 |
| fly20 | 8.31 (8.24-8.33) | 8.13 (8.04-8.14) | 0.978 |
| fly100 | 8.46 (8.45-8.56) | 8.33 (8.24-8.42) | 0.985 |
| edits (p99 = max) | 5.12 (4.67-5.34) | 5.02 (4.84-5.09) | 0.980 |
| circle | 8.51 (8.42-8.64) | 8.52 (8.37-8.53) | 1.001 |
| generation, ms per column (circle `gen_ms`) | 2.44 (2.44-2.46) | 2.45 (2.45-2.46) | 1.004 |

Load per run: 0.6-6.4% before, 3.5-7.2% others during. The engine calls a few `sin`, `cos`,
`tan` and `pow` per frame (camera and frustum), too few to show. No phase got slower; fill's p99
is 8% lower, but its two ranges overlap almost entirely (fill's p99 is its noisiest number, as in
the determinism round's table below).

## Compile time

`measure.py compile` (new in this round): `kestrel check --release` (the front end, the checks and
KIR with its passes, no machine code), `kestrel run --release` (that plus Cranelift into memory and
the program's `main`; not for `voxel/`, which opens a window) and `kestrel build --release` (that
plus the object file and the link), every compiler in turn. `voxel/main.kes`, 21 rounds (load
7.1% before, 13.2% during, 9.1% after):

| `voxel/main.kes` | before | after | after / before |
|---|---:|---:|---:|
| `check --release` | 215.1 (211.3-220.8) | 222.4 (217.4-246.0) | **1.034** |
| `build --release` | 320.4 (313.7-340.2) | 332.4 (323.8-366.1) | **1.037** |

**Compiling the voxel engine takes about 3.5% longer, 7 ms of it before code generation** (two
9-round blocks agree: `check` 1.033 and 1.017, `build` 1.024 and 1.002). `check` runs the same
front end at both commits, so the extra time is in KIR; loop hoisting (`kir/src/hoist.rs`) is the
round's new KIR pass. Machine code (`build` minus `check`) grew by about 5 ms, 105 to 110. The
empty program's build (the link) ranges 62-111 ms, so the builds are not shown net of it.

**The small kernels, 31 rounds** (load 2.4%, 9.8%, 3.6%): `kestrel run --release` minus `run` of
the empty program is 3-6 ms per kernel with either compiler. Summed over the 13 `math_*.kes`
kernels, using each one's fastest run, it is 42.0 ms before and 47.8 after (+0.45 ms a kernel,
+14%); `trig_step.kes` 3.9 against 6.1 ms. That is the inline math's machine code. Process times
here fall into two groups (about 14 and 19 ms) from run to run, so a single kernel's median moves
by ±3 ms; the totals are the readable number. `check` didn't move (10.2 against 9.9 ms in total).
The kernels' builds are lost in the link's noise.

## Accuracy and same bits

**Same bits everywhere:** the runtime's results hash (`wide_results_hash_is_pinned`, FNV-1a of
every wide function's results over its sweep) is `0xF8962BD9040E8F6C` in a release build, a debug
build and a `RUSTFLAGS=-C target-cpu=native` release build (own target folder, so AVX2 and FMA
were available to LLVM); all three passed. `kestrel check --report` and `--report --release` print
the same bytes with both compilers for the frozen world, and for `voxel/main.kes` and every
`bench/kernels/*.kes` (68 pairs, 0 differ): hoisting doesn't change the report.

**Accuracy** (`print_wide_report`, the 1M-input sweep per function, ulps against `libm`'s `f64`
result; "same as today" counts bit-identical results with 49ae9f0's):

| function | inputs | max ulp, sweep | max ulp, every `f32` (input) | mean ulp | same as today, sweep | differ from today, every `f32` |
|---|---:|---:|---:|---:|---:|---:|
| `sin` | 1,000,101 | 0.5009 | 0.500900 (5.5782076e7) | 0.2317 | 99.9773% | 146,766 |
| `cos` | 1,000,101 | 0.5009 | 0.500900 (1.5705271e4) | 0.2320 | 99.9721% | 170,626 |
| `tan` | 1,000,101 | 0.5010 | 0.501177 (3.4509312e8) | 0.2314 | 99.9641% | 221,788 |
| `asin` | 1,000,101 | 0.5000 | 0.500012 (1.8791507e-3) | 0.1928 | 100.0000% | 1,498 |
| `acos` | 1,000,101 | 0.5000 | 0.500007 (9.779774e-1) | 0.2402 | 100.0000% | 55 |
| `atan` | 1,000,101 | 0.5000 | 0.500012 (7.618335e-3) | 0.2132 | 99.9999% | 1,502 |
| `exp` | 1,000,101 | 0.5000 | 0.500013 (-5.5412523e-2) | 0.1763 | 99.9997% | 1,431 |
| `log` | 1,000,101 | 0.5000 | 0.500000 (1.5856676) | 0.2244 | 100.0000% | 9 |
| `atan2` | 1,010,201 | 0.5000 | (two arguments) | 0.2650 | 99.9980% | |
| `pow` | 1,010,201 | 0.5000 | (two arguments) | 0.1267 | 99.9836% | |
| `fmod` (float `%`) | 1,010,201 | 0 (exact) | (two arguments) | 0 | 100.0000% | |

The "every `f32`" column is `exhaustive_one_argument_functions` (all 2^32 inputs, 359 s on 8
threads), run again at 3a1a713: the same maxima and counts as at Task 1 (e25d79a), as expected,
since later commits changed only `pow` and the sweep's inputs. **Every function is within the
0.51 ulp gate**; out of 4.3 billion inputs at most 221,788 (0.005%, `tan`) differ from today's
bits, each by one ulp. `pow`'s "same as today" is lower than at Task 1 (99.9836% against
99.9995%) because its sweep changed at b45d8c7: 10% of its random inputs are now integer and
dyadic `x` with small integer and half exponents, whose results are often exact or exact
midpoints, where the wide path can round differently from `libm` (below). Its maximum is still
0.5000 ulp. (Not traced input by input.)

**`pow`:** `pow(x, y)` for `y` of exactly 1, -1, 2 and 0.5, with `x > 0`, returns `x`, `1/x`,
`x*x` and `sqrt(x)`, each one `f64` operation rounded once to `float`. It tests `y`'s bits, so its
value, whether it came from a literal, a constant or a computation. That is what `libm::pow`
returns early for those exponents, so they keep the old bits, and it equals the same expression
written in `float` (`pow(x, 2.0) == x*x`). Zeros, infinities, NaN and negative `x` take the `libm`
path, with today's results (`pow(-0, 0.5)` is `+0` and `pow(-inf, 0.5)` is `+inf`, as in C, while
`sqrt` of those gives `-0` and NaN). Every other exponent, `-2` included, takes the wide path,
within 0.5 ulp: an exact `1/(x*x)` would be neither the old bits nor provably correctly rounded.
The wide path can round an exact midpoint away from even where `libm` rounded to even:
`pow(2, -150)` is `2^-149` (the smallest subnormal), where the old code gave 0.

**Program output that changed** (all checksums of simulations, which mix every float result into
one number; the raw values on the other lines didn't change):

- `tests/programs/det_par.kes` line 33: `-4727177381731691941` to `1766166045610435347`;
- `tests/programs/det_sim.kes` lines 87-90 (frames 250, 500, 750, 1000):
  `8833392080462543213` to `6629063034239713950`, `-7362058023118986372` to
  `-8923671571335071607`, `2187843396518032676` to `-3003307686104160227`,
  `-3165968733279055487` to `-2363078924110686833`;
- `tests/programs/det_math_float.kes`, the `pow` checksum: `323258267209170825` (Task 2's wide
  path) to `-2935346233227922869`, when the exact exponents went in: 2 of its 2,000
  exact-exponent inputs changed, and both are now the old bits.

The 36 `det_math` float rows (raw values) and the 163 pinned `FLOAT_TABLE` rows didn't change.
`hoist_math.kes` case 1 (a sequential `for` and a `par for` over the same `dt`) now computes
`pow(KEEP, dt)` once before both loops, since a `par for` passes a `float` capture as a copy;
`int` captures are still passed by pointer (followups entry 55).

## The goals, met or missed

**G3, speed per call** (the `math_<op>.kes` kernels; met or missed is the same gross and net):

- `sin` ≤ 6: **met**, 4.25 ns (3.62 net), from 15.17.
- `cos` ≤ 5.5: **met**, 3.81 (3.18 net), from 15.28.
- `exp` ≤ 4: **missed**, 5.58 (4.95 net), from 7.37.
- `log` ≤ 4.5: **met**, 4.38 (3.75 net), from 6.11.
- `pow` ≤ 12: **missed**, 24.31 (23.67 net), from 37.63; `pow` is still a call.
- `atan2` ≤ 11: **met**, 10.26 (9.62 net), from 20.49.
- float `%` ≤ 3, inlined: **missed**, 5.03 (4.39 net), from 5.71; it is inlined.
- `tan` no slower: **met**, 5.13 from 15.98.
- `asin`, `acos`, `atan` no slower: **missed**: 25-27% slower (10.86 from 8.65, 10.60 from 8.38,
  8.99 from 7.16).

**G4, speed in kernels and programs:**

- `trig_step.kes` ≤ 22 ns: **missed**, 22.34 (three blocks: 22.31-22.34), from 35.52.
- `step_pow.kes` ≤ 5 ns: **met**, 4.38, from 33.44, level with `step.kes`'s hand-hoisted 4.36.
- Noise kernels within ±2%: **met**, at most +0.2%.
- The frozen world within ±2%: **met**, -0.2% checked, +0.1% `--release`; generation +0.1%.
- The engine's p99 within ±2%: **met**, in that no phase got slower than +0.1% (circle); fly20,
  fly100 and edits are 1.5-2.2% faster, and fill 8% faster with overlapping ranges.
- Not a goal, but asked for: compiling `voxel/main.kes` takes 3.5% longer (7 ms in KIR, 5 ms in
  machine code); each small math kernel's JIT compile about 0.5 ms more.

**Accuracy and determinism** (G1, G2): **met**. Every function is at most 0.501177 ulp from
`libm`'s `f64` result, the stand-in for the true value (the gate is 0.51); the runtime's results
are the same bits in release, debug and host-CPU builds. The JIT against exe half of G2 is the
test suite's (Task 2: the 163 pinned rows and 13 sweep checksums are the same in the JIT, a
checked exe and a `--release` exe), not measured here.

**Follow-ups this points at:** call the Rust function instead of inlining `atan` (and perhaps
`asin`, `acos` and `exp`), where LLVM's build of the same code is faster than Cranelift's inline
copy; a faster `powf` (followups entry 52); and, if the compile time matters, the hoisting pass's
cost (followups entry 54).

# Determinism round 1 (2026-10-27)

Measured 2026-10-28 on the user's PC (Ryzen 7 5800X, 8 cores, 16 threads, Windows 11), the user
away and the PC free. What the round costs and gains: Kestrel's own math (`libm`) against the
platform's, pinning the float mode (MXCSR), compiling for the SSE4.1 baseline instead of the host
CPU, and snapshot copies and `checksum`.

## How it was measured

**Three compilers**, each built `--release` in a `git worktree` with its own short target folder
(`C:/kt/det-<name>`, removed afterwards):

- **before**: c743e8a, the commit before the round (platform math, no pinning, host ISA);
- **after**: 45d39e7, the round's last commit (`libm`, pinning, the SSE4.1 baseline for exes and
  the JIT);
- **host**: 45d39e7 with `codegen/src/isa.rs` changed back to `cranelift_native::builder()` (the
  host's features: AVX2, FMA, BMI and so on), a scratch change that was never committed. **after
  against host is the baseline ISA's effect alone**; before against after is the whole round.

All three compiled the same sources (`voxel/`, `bench/voxels3d/` and `bench/kernels/` are the same
at both commits). Every exe prints the same result for all three (463,749 faces, generation 18,500,
the snapshot and checksum values), and the script stops if one doesn't.

**Method** (`measure.py`, run from this folder; raw outputs in
`.superpowers/sdd/2026-10-27-determinism/task-12-raw/`, local only): each block runs every
compiler's program once per round, in turn, at normal priority, nothing else of mine running.
Exes: the matching empty program's median (process start-up, about 6 ms; for the snapshots, the
state's set-up) is subtracted. Medians of 9 rounds (11 for the frozen world and generation), with
the range of the raw times. **Load** is the whole machine's CPU use, sampled for 2 s before and
after each block and over the whole block ("during" includes the timed process: one busy thread
of 16 is 6.25%). OneDrive used about one thread of its own (about 6%) for most of the session, so
"before" loads are mostly 6-8%; it affects all three compilers alike, since they run in turn.

## Own math against the platform

`cargo test -p kestrel-runtime --release own_math_against_the_platform -- --ignored --nocapture`
(`runtime/src/math/speed.rs`), 10,000,000 calls per function over a spread of inputs, 9 runs
(`measure.py math 9`). Load 0.4% before, 6.8% during, 0.6% after. ns per call, median (range):

| Function | `libm` (now) | platform (before) | ratio |
|---|---:|---:|---:|
| `sin` | 6.09 (6.07-6.13) | 5.22 (5.21-5.23) | 1.17 |
| `cos` | 6.22 (6.20-6.26) | 5.12 (5.11-5.17) | 1.21 |
| `tan` | 7.82 (7.81-7.85) | 6.45 (6.22-6.53) | 1.21 |
| `asin` | 8.33 (8.30-8.45) | 5.41 (5.33-5.44) | 1.54 |
| `acos` | 7.54 (7.50-7.57) | 5.04 (4.93-5.12) | 1.50 |
| `atan` | 5.37 (5.36-5.41) | 4.84 (4.75-4.89) | 1.11 |
| `atan2` | **8.89** (8.88-8.97) | 9.72 (9.69-10.26) | **0.91** |
| `exp` | 7.34 (7.33-8.03) | 3.91 (3.89-3.98) | 1.88 |
| `log` | 6.64 (6.63-6.73) | 4.61 (4.60-4.64) | 1.44 |
| `pow` | 32.81 (32.78-32.98) | 12.29 (12.26-12.52) | **2.67** |
| float `%` (`fmod`) | 7.51 (7.49-7.72) | 3.93 (3.91-3.94) | 1.91 |

**Kestrel's math is slower, by 1.1 to 2.7 times per call; only `atan2` got faster.** The platform's
functions (the MSVC runtime) pick FMA code on this CPU, which is exactly what made their results
differ between machines. In a program it shows where math is in the hot loop: `step_pow.kes` (one
`pow` per particle, through the JIT) went from **16.59 to 33.45 ns per particle, 2.02 times slower**
(kernels table below). Where the math is hoisted out of the loop (`step.kes`) or absent (the voxel
world, generation, the engine), nothing changed.

## Pinning the float mode

`pinning_the_float_mode` (same file), 100,000,000 iterations, 9 runs, same block as above:

| | ns per iteration |
|---|---:|
| empty loop | 0.21 (0.21-0.21) |
| `fpu::pin` inlined (one `ldmxcsr`) | 0.41 (0.41-0.42) |
| `kestrel_rt_fpu_pin` called through a pointer, as compiled code calls it | 4.14 (4.14-4.15) |

**Per frame:** a scratch counter in the runtime (`task-12-raw/pincount.patch`, not committed)
counted the pins compiled code makes after window-library natives: the voxel engine bench pins
5,467,240 times over 20,586 frames (**266 a frame, about 1.1 µs a frame**, 0.007% of a 16.7 ms
frame), rollback Pong (headless, 600 frames) 24,002 times (**40 a frame, about 0.17 µs**). Pinning
at `main` and when a thread joins a `par for` job happens once per program or job. Not measurable
in any program below.

## The baseline CPU, and the whole round, on programs

**Exes** (whole run minus the empty program's 6 ms; ms, median (range of the whole runs)):

| | before | after | host | after / before | after / host |
|---|---:|---:|---:|---:|---:|
| frozen world `bench/voxels3d/main.kes`, checked | 425.4 (429.5-472.5) | **418.5** (423.0-459.8) | 425.1 (429.9-476.4) | 0.984 | 0.984 |
| frozen world, `--release` | 369.9 (373.3-424.9) | 375.1 (378.7-417.4) | 370.1 (373.4-412.1) | 1.014 | 1.014 |
| generation, 64 columns through `load_now`, `--release` | 299.7 (303.8-357.1) | 301.7 (306.4-350.6) | 299.2 (303.9-343.8) | 1.007 | 1.008 |

Load: frozen world 6.8% before, 14.5% during, 7.7% after; generation 7.2%, 13.2%, 6.5%. The high
ends of the ranges are each block's first, cold round. **The baseline ISA moves the exes by -1.6%
to +1.4%**, in both directions: within what code layout alone moves these programs
(`bench/voxels3d/results.md`).

**JIT, whole programs** (`kestrel run --release`, compile included, minus `kestrel run` of the
empty program; 9 rounds; load 6.8% before, 13.2% during, 7.3% after):

| | before | after | host | after / host |
|---|---:|---:|---:|---:|
| frozen world | 370.4 (430.5-442.1) | 370.4 (431.3-439.9) | 371.0 (431.4-439.8) | 1.00 |
| generation | 352.4 (362.9-370.5) | 355.1 (365.3-369.0) | 353.6 (362.5-368.5) | 1.004 |

**JIT, kernels** (`bench/kernels`, which compiles each kernel the way `kestrel run` does and times
it through the JIT; 1 thread, 9 rounds, every compiler in turn within a round; ns per item, or per
chunk for the race; median of the rounds' medians (range)):

| Kernel | before | after | host | after / before | after / host |
|---|---:|---:|---:|---:|---:|
| `race_faces.kes` checked | 83,610 (82,846-84,294) | 84,089 (83,233-84,865) | 83,586 (82,728-84,387) | 1.006 | 1.006 |
| `race_faces.kes --release` | 66,515 (66,118-67,570) | 66,641 (66,232-67,517) | 66,659 (66,086-67,617) | 1.002 | 1.000 |
| `race_arrays.kes` checked | 128,085 (126,691-129,816) | **124,999** (123,731-126,781) | 128,260 (126,536-129,383) | 0.976 | **0.975** |
| `race_arrays.kes --release` | 103,795 (102,738-105,410) | **101,318** (100,074-102,540) | 104,026 (103,141-105,544) | 0.976 | **0.974** |
| `noise_scalar.kes` checked | 6.65 (6.65-6.70) | 6.71 (6.70-6.72) | 6.66 (6.65-6.67) | 1.009 | 1.008 |
| `noise_scalar.kes --release` | 6.55 (6.55-6.55) | 6.63 (6.62-6.63) | 6.55 (6.54-6.56) | 1.012 | 1.012 |
| `noise_corners.kes` checked | 5.29 (5.29-5.31) | **5.45** (5.45-5.45) | 5.29 (5.29-5.29) | 1.030 | **1.030** |
| `noise_corners.kes --release` | 5.26 (5.26-5.27) | **5.42** (5.41-5.42) | 5.26 (5.26-5.27) | 1.030 | **1.030** |
| `noise_wide.kes` checked | 2.60 (2.60-2.60) | **2.70** (2.70-2.70) | 2.60 (2.60-2.60) | 1.038 | **1.038** |
| `noise_wide.kes --release` | 2.58 (2.58-2.59) | **2.71** (2.70-2.71) | 2.58 (2.58-2.59) | 1.050 | **1.050** |
| `step.kes` | 4.29 (4.29-4.31) | 4.32 (4.32-4.34) | 4.29 (4.28-4.30) | 1.007 | 1.007 |
| `step_index.kes` | 5.51 (5.48-5.54) | 5.58 (5.56-5.60) | 5.51 (5.50-5.55) | 1.013 | 1.013 |
| `step_scalar.kes` | 4.29 (4.29-4.32) | 4.32 (4.32-4.34) | 4.29 (4.29-4.32) | 1.007 | 1.007 |
| `step_pow.kes` (`pow` per particle) | 16.59 (16.54-16.82) | **33.45** (33.30-34.18) | 33.26 (33.22-34.99) | **2.02** | 1.006 |
| `iface_step.kes` | 4.30 (4.29-4.57) | 4.30 (4.29-4.67) | 4.31 (4.30-4.58) | 1.00 | 1.00 |
| `iface_step_direct.kes`, `iface_step_generic.kes` | 0.42 | 0.42 | 0.42 | 1.00 | 1.00 |

Load: race 6.8% before, 11.5% during, 1.6% after; noise 1.4%, 7.3%, 9.2%; step 8.2%, 9.8%, 1.0%.
Rust references in the same rounds: `race_faces` 48,086, `race_arrays` 57,029, `noise_scalar`
6.11, `noise_wide` 2.40, `step` 3.75 ns.

**The JIT loses 3.0% on `noise_corners` and 3.8-5.0% on `noise_wide` to the baseline ISA, and
gains 2.5% on `race_arrays`.** The noise kernels' code shows why: with the host's AVX, Cranelift
uses three-operand VEX instructions and needs no register copies (`noise_wide`'s `bench` is 195
instructions with the host ISA against 223 with the baseline: 22 `movdqa` copies gone, splats as
one `vpbroadcastd`; `noise_corners` 135 against 146; `task-12-raw/dump-mnemonics.txt`). The race
gain isn't explained by an instruction count; it is the same size in both directions as other
layout effects seen here.

**Engine** (`kestrel/bench/engine`, `voxel/bench.kes --release` in a hidden window, 3 runs per
compiler in turn; work p99 per phase, ms, median (range)):

| Phase | before | after | host | after / host |
|---|---:|---:|---:|---:|
| fill | 10.31 (9.03-12.86) | 9.83 (9.38-10.63) | 10.16 (9.92-11.21) | 0.97 |
| fly20 | 8.04 (8.01-8.20) | 8.38 (8.22-8.42) | 8.20 (8.03-8.39) | 1.02 |
| fly100 | 8.41 (8.23-8.58) | 8.52 (8.34-8.66) | 8.43 (8.22-8.65) | 1.01 |
| edits (51 frames: p99 = max) | 5.38 (4.89-5.55) | 5.54 (4.90-5.61) | 5.16 (4.90-5.42) | 1.07 |
| circle | 8.42 (8.21-8.50) | 8.41 (8.33-8.59) | 8.30 (8.29-8.49) | 1.01 |
| generation, ms per column (circle `gen_ms`) | 2.46 (2.43-2.47) | 2.48 (2.45-2.49) | 2.46 (2.42-2.46) | 1.01 |

Load per run: 0.5-7.4% before, 0.9-7.3% others during (the first two rounds had OneDrive's thread,
the last had none). Every p99 is far inside E32's 16.7 ms. The ranges of after and host overlap in
every phase; fly20 p99 is 4% above before (8.38 against 8.04), and those two ranges don't overlap
(8.22-8.42 against 8.01-8.20). The
first engine block (`task-12-raw/engine.txt`) was thrown away: in its first two runs (before and
after) the display waited for vsync (work median 16.6 ms in every phase, runs of 344 and 236 s
instead of 56), so those p99s measure the display, not the code. Its other seven runs agree with
the table.

## Snapshots and `checksum`

`snapshot.kes` (`measure.py snap 9`): **Pong's `Game`** (a copy, with its `Rng` written out as a
struct of one `i64` so the old compiler compiles it) saved into a ring of 16 and restored, and a
**voxel-sized `Pool[Column]`** (441 columns of 32 x 128 x 32 `u8` blocks, 58 MB) saved into a
snapshot and restored. A pair is one save and one restore; the loop's other work and the set-up are
taken out by the control programs (`pong_control`, `pool_pairs(0)`). Load 6.1% before, 13.7%
during, 6.7% after.

| | before (copy into new memory) | after (reuse) | host |
|---|---:|---:|---:|
| `Game`, ns per pair (100,000,000 pairs) | 1.08 (whole 175.6-181.4 ms) | 1.09 (176.2-180.0) | 1.09 (175.3-179.5) |
| `Game`, ns per pair (1,000,000 pairs, as the plan asks) | 1.3 (7.8-11.0) | 1.6 (7.7-11.4) | 1.3 (7.7-11.3) |
| `Pool`, ms per pair, 10 pairs | **48.7** (whole 534-603 ms) | **19.4** (235-251) | 18.8 (234-249) |
| `Pool`, ms per pair, 100 pairs | - (would need 11.6 GB) | **19.1** (1,862-1,972) | 18.8 (1,871-1,966) |
| `Pool` program, peak private memory, 10 pairs | **1,225 MB** | **113 MB** | 113 MB |
| `Pool` program, peak private memory, 100 pairs | - | 113 MB | 113 MB |

- **Reuse makes a voxel-sized snapshot pair 2.5 times faster and stops the growth**: before, every
  save and every restore allocated a fresh 58 MB (1,225 MB after 10 pairs, so 100 pairs would need
  11.6 GB and weren't run); after, the program stays at its set-up's 113 MB (the pool and one
  snapshot) for 10 and 100 pairs. A pair is two 58 MB copies, about 6 GB/s.
- **Pong's `Game` is level** (about 1 ns a pair either way): it holds no list, so it was always a
  plain copy; the 1,000,000-pair row is below the timer's resolution (whole runs of 7.7-11.4 ms,
  of which the set-up takes 6.6; the measured net is 1.3 ms before and 1.6 ms after) and is given only because the plan asks for it.

`checksum` (today's compiler only; the old one has none), with its controls:

| | after | host |
|---|---:|---:|
| `checksum(g)` of a `Game`, ns per call (1,000,000 calls) | 6.52 (whole 12.4-21.2 ms) | 6.73 (12.6-17.3) |
| `checksum(p)` of the 58 MB `Pool`, ms per call (20 calls) | 72.1 (whole 1,466-1,485 ms) | 71.9 (1,464-1,468) |

A pool checksum is about 1.25 ns per block (0.8 GB/s): it folds one `u8` at a time, a multiply
chain. Fine for a desync check every 30 frames on a Pong-sized state; for a voxel world, checksum
the columns that changed.

## Where Kestrel got slower

- **Math calls: 1.1 to 2.7 times slower per call** (`pow` 2.67, `fmod` 1.91, `exp` 1.88); a kernel
  with `pow` per item is **2.02 times slower**. This is the price of the same bits on every CPU.
- **The JIT with the baseline ISA: 3.0% slower on `noise_corners`, 3.8-5.0% on `noise_wide`**, 0.7
  to 1.3% on `noise_scalar` and the `step` kernels; 2.5% faster on `race_arrays`.
- **Exes with the baseline ISA: the frozen world `--release` 1.4% slower** (checked 1.6% faster),
  generation 0.8% slower, engine p99 within 1-2% in most phases (edits, 51 frames: 7% against a host-ISA build and 3% against before; fly20 +4% against before, see the engine section).
- Pinning the float mode: nothing measurable.

## The JIT's ISA: a proposal for the controller

The JIT lost more than 2% (`noise_corners` 3.0%, `noise_wide` 3.8-5.0%), so by spec decision 4 the
host ISA can come back for the JIT without touching the promise (no FMA is ever emitted; AVX
encodings compute the same IEEE operations; `det_math` and the cross-build test would still pin
JIT against exe output). **Proposed: the JIT compiles for the host CPU again; exes keep the SSE4.1
baseline.** Against it: one ISA means the JIT and an exe compile the same instructions, so a
host-only lowering bug can't make them disagree; and the host ISA made `race_arrays` 2.5% slower.
The controller rules.
