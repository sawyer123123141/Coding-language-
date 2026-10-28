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
