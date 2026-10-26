# Benchmarks

This file summarizes every benchmark that compares Kestrel with another language. Kestrel is timed against Rust (`-O3`, one thread unless a row says otherwise) and, where a benchmark exists, against C (gcc `-O2`) and C# (.NET 9, Release, tiered PGO); the Kestrel-only table further down covers what the compiler's own passes bought. Every number here is copied or divided from a `results.md` file listed at the end, and each row names its source and the date it was measured.

**Headline:** on tight scalar loops, struct updates, SIMD noise and the particle step Kestrel is within about 1.0x to 1.2x of Rust, and it beats C# on the voxel benchmarks; it is clearly behind on recursion (fib 2.4x), integer division (collatz 2.3x), the voxel heightmap (2.1x), loops Rust vectorizes (`map` 1.8x, the draw copy 3.4x, direct interface loops 7-10x), `sort_by` (2.05x), interface calls (about 1.15x) and hash maps (up to 2.5x behind C#).

The ratio is Kestrel time divided by the other language's time, so above 1 means Kestrel is slower. All Kestrel numbers are `--release` builds unless a row says "checked" (overflow, narrowing and bounds checks on).

## Summary: Kestrel against Rust, C and C#

† marks a run the file calls busy, loaded or noisy; compare ratios, not raw times. ‡ marks a reference that was not re-measured after a later Kestrel change.

| Benchmark | What it does | Kestrel | Reference | Ratio | Date | Source |
|---|---|---:|---:|---:|---|---|
| fib(38) | recursive `i64` calls | 242 ms | 99 ms Rust | 2.4x † | 2026-10-11 | [2a](kestrel/bench/kernels/results.md) |
| loop | 200M `i64` multiply + mod | 703 ms | 570 ms Rust | 1.2x † | 2026-10-11 | [2a](kestrel/bench/kernels/results.md) |
| collatz | longest chain under 1M | 251 ms | 109 ms Rust | 2.3x † | 2026-10-11 | [2a](kestrel/bench/kernels/results.md) |
| mandelbrot | 2000², 200 iterations, f32 | 422 ms | 350 ms Rust | 1.2x † | 2026-10-11 | [2a](kestrel/bench/kernels/results.md) |
| sieve | primes below 50M in an `array[bool]` | 973 ms | 849 ms Rust | 1.15x † | 2026-10-11 | [2a](kestrel/bench/kernels/results.md) |
| swarm | 1M four-float structs in a `list`, 100 steps | 178 ms | 99 ms Rust | 1.8x † | 2026-10-11 | [2a](kestrel/bench/kernels/results.md) |
| particles | one struct of four floats, 100M `var self` steps | 202 ms (checked 163 ms) | 322 ms Rust (C 132 ms) | 0.63x (checked 0.51x) † | 2026-10-11 | [2a](kestrel/bench/kernels/results.md) |
| heightmap | 16.7M blocks, noise, `Block` enum, arenas | 58 ms (checked 73 ms) | 27 ms Rust (checks on 32 ms) | 2.1x (checked 2.3x) | 2026-10-11 | [heightmap](kestrel/bench/heightmap/results.md), [kernels](kestrel/bench/kernels/results.md) |
| voxels3d, tuned copy | 8.4M-block 3D world, hand-tuned source, whole run | 315 ms | 344 ms Rust | 0.92x | 2026-10-24 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| voxels3d, as written | same world, plain source, whole run | 390 ms | 344 ms Rust | 1.13x | 2026-10-24 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| voxels3d vs C# | same world, one thread, world build | 529-546 ms (checked 560-587 ms) | 1,527-1,573 ms C# | 0.35x (checked 0.37x) † | 2026-10-20 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| voxels3d `par for` vs C# | same world, all 16 threads | 63-66 ms | 368-373 ms C# `Parallel.For` | 0.17x | 2026-10-10 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| voxels (2D mesher) vs C# | 16 chunks of 32×32 columns, per build | 8.6 ms (checked 12.6 ms) | 13.1 ms C# | 0.66x (checked 0.96x) † | 2026-10-10 | [voxels](kestrel/bench/voxels/results.md) |
| `step.kes` | demo particle step, `par for`, 1 thread, per particle | 5.80 ns | 5.08 ns Rust | 1.14x | 2026-10-20 | [kernels](kestrel/bench/kernels/results.md) |
| `step_pow.kes` | the step with `pow` per particle, 1 thread | 19.12 ns | 17.84 ns Rust | 1.07x † | 2026-10-11 | [kernels](kestrel/bench/kernels/results.md) |
| `step_index.kes` | the step in a `for i` loop over `parts[i]` | 8.28 ns | 5.08 ns Rust | 1.63x | 2026-10-20 | [kernels](kestrel/bench/kernels/results.md) |
| `copy.kes` | copy each particle's position into a `list[vec2]` | 0.92 ns | 0.27 ns Rust | 3.4x | 2026-10-20 | [kernels](kestrel/bench/kernels/results.md) |
| `filter` | `xs.filter(...)` over 1M ints | 3.43 ns | 3.18 ns Rust | 1.08x | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| `map` | `vs.map(...)` over 1M `vec2` | 1.49 ns | 0.84 ns Rust | 1.8x | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| `sort_by` | stable sort of 100,000 structs by a float key | 44.76 ns | 21.80 ns Rust | 2.05x | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| Interface call, `hp` | `d.hp()` through a `list[Damageable]`, random order | 5.21-5.26 ns | 4.42 ns Rust `Box<dyn>` | 1.18-1.19x | 2026-10-19 | [kernels](kestrel/bench/kernels/results.md) |
| Interface call, `step` | `d.step()` through the same list | 5.23-5.24 ns | 4.54-4.60 ns Rust `Box<dyn>` | 1.14-1.15x | 2026-10-19 | [kernels](kestrel/bench/kernels/results.md) |
| Direct loop, `hp` / `step` | same loops over a `list[Goblin]` (Rust vectorizes) | 0.73-0.74 / 0.49-0.50 ns | 0.07 ns Rust | 10.4-10.6x / 7.0-7.1x | 2026-10-19 | [kernels](kestrel/bench/kernels/results.md) |
| Noise, scalar | 2D value noise, one sample at a time | 6.91 ns | 6.44 ns Rust | 1.07x | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| Noise, `vec4`/`ivec4` | four samples at once | 2.72 ns | 2.51 ns Rust SSE4.1 | 1.08x | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| Particles demo frame | 200,000 dots, bulk drawing, 16 threads, hidden window | 5.58 ms | 5.22 ms Rust (5.46 ms with the demo's copy) | 1.07x (1.02x) † | 2026-10-11 | [particles](kestrel/bench/particles/results.md) |
| Particles demo, one call per dot | 200,000 `draw.rect` calls, draw time | 28.27 ms exe, 29.28 ms JIT | 24.67 ms Rust | 1.15x exe, 1.19x JIT † | 2026-10-11 | [particles](kestrel/bench/particles/results.md) |
| `map`, integer keys in order | 1M keys, 5 rounds, 20M operations | 443-613 ms (checked 423-437 ms) | 168-172 ms C# `Dictionary` | 2.5x † | 2026-10-10 | [maps](kestrel/bench/maps/results.md) |
| `map`, integer keys scattered | same, keys scrambled over a billion values | 395-440 ms (checked 446-583 ms) | 310-319 ms C# | 1.3x † | 2026-10-10 | [maps](kestrel/bench/maps/results.md) |
| `map`, string keys | 10M word counts over 64 words | 339-344 ms (checked 321-330 ms) | 290-307 ms C# | 1.1x ‡ | 2026-10-10 | [maps](kestrel/bench/maps/results.md) |

Notes on the table:

- **2a rows** come from the 2026-10-11 run that compared the compiler before and after scalar replacement; the Rust column is `-O3` with the release defaults. Kestrel's `particles` `--release` exe was slower than its checked exe (202 against 163 ms) from code layout, not a regression. The earlier 2026-10-09 table also had C: fib 60 ms (Kestrel 229 ms), loop 537 ms (673 ms), mandelbrot 344 ms (410 ms), collatz 111 ms (237 ms after division by a constant was strength-reduced).
- **`map` string keys:** after strings became heap values (2026-10-10, later the same day) Kestrel's own time rose about 35% (to 423-452 ms against 318-332 ms); C# was not re-measured, so the 1.1x is the older ratio and the current gap is larger. `maps` is the most noise-sensitive benchmark here (see below); the 2026-10-20 attempt to re-read it was unusable.
- **voxels (2D) vs C#:** the C# time of 13.1 ms came from the last pass, when the machine was slower; earlier passes had C# at 10.6-11.9 ms, which makes Kestrel's `--release` 0.72-0.81x.
- **Particles demo frame:** the 2-7% gap is about the size of this machine's noise, though the direct Rust frame was shorter in all four rounds. The same file's update-step ratio (3.4x with `pow` hoisted) predates scalar replacement and is superseded by the `step.kes` rows above.
- **Rust's `pow`/`step` and `copy` figures** for `step.kes`, `step_index.kes` and `copy.kes` are from the same 2026-10-20 run; `step_pow.kes` is from the 2026-10-11 run, the newest with a Rust `pow` column.

## Summary: Kestrel against itself

Ratios are the first time divided by the second, so below 1 means the first is faster.

| Benchmark | What it compares | Kestrel | Other Kestrel | Ratio | Date | Source |
|---|---|---:|---:|---:|---|---|
| voxels3d, checks vs none | checked build over `--release`, as written | 439 ms | 382 ms | 1.15x | 2026-10-24 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| voxels3d tuned, checks vs none | same, hand-tuned source | 313 ms | 313 ms | 1.00x | 2026-10-24 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| voxels3d, range analysis | checked build, after against before the analysis | 643 ms | 1,415 ms | 0.45x | 2026-10-20 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| `step_index.kes`, range analysis | after against before (bounds checks removed) | 8.28 ns | 11.13 ns | 0.74x | 2026-10-20 | [kernels](kestrel/bench/kernels/results.md) |
| Noise, checks vs none | checked over `--release`, `noise_wide.kes` | 2.74 ns | 2.72 ns | 1.01x | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| Noise, all checks kept | `--keep-checks` over range analysis, checked wide kernel | 5.01 ns | 3.74 ns | 1.34x † | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| Noise, SIMD vs scalar | four samples at once over one at a time | 2.72 ns | 6.91 ns | 0.39x | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| Noise, four corners | four corners of one sample over scalar | 5.55 ns | 6.91 ns | 0.80x | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| Generic vs hand-written | generic `step_all[T: Damageable]` over the hand-written loop | 0.56 ns | 0.55 ns | 1.02x † | 2026-10-25 | [kernels](kestrel/bench/kernels/results.md) |
| Generic vs interface list | generic loop over the `list[Damageable]` loop | 0.56 ns | 5.18 ns | 0.11x † | 2026-10-25 | [kernels](kestrel/bench/kernels/results.md) |
| Interface vs direct | `hp` through `list[Damageable]` over `list[Goblin]` | 5.21 ns | 0.73 ns | 7.1x | 2026-10-19 | [kernels](kestrel/bench/kernels/results.md) |
| `filter` vs hand loop | method over hand-written loop | 3.43 ns | 3.37 ns | 1.02x | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| `map` vs hand loop | method over hand-written loop | 1.36 ns | 1.36 ns | 1.00x | 2026-10-18 | [kernels](kestrel/bench/kernels/results.md) |
| `sort_by` vs hand merge sort | method over a hand-written merge sort | 44.76 ns | 87.71 ns | 0.51x | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| `sort_by`, key caching | after against before | 44.76 ns | 140.60 ns | 0.32x | 2026-10-24 | [kernels](kestrel/bench/kernels/results.md) |
| `par for` scaling | voxels3d all threads over one thread | 63-66 ms | 454-463 ms | 0.14x (7.1x faster) | 2026-10-10 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| Scalar replacement | `step.kes`, one thread, after against before | 4.88 ns | 14.46 ns | 0.34x † | 2026-10-11 | [kernels](kestrel/bench/kernels/results.md) |
| KIR inliner | voxels3d `--release`, v3 against v1 | 481-505 ms | 1,166-1,268 ms | about 0.4x | 2026-10-10 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| Loop-invariant hoisting | voxels3d frozen `--release`, after against before | 500 ms | 557 ms | 0.90x † | 2026-10-21 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| Cranelift 0.137 | voxels3d frozen `--release`, after against before (a loss) | 385 ms | 373 ms | 1.03x | 2026-10-24 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| Strings as heap values | `map` string keys, after against before (a loss) | 435-448 ms | 318-323 ms | about 1.35x † | 2026-10-10 | [maps](kestrel/bench/maps/results.md) |
| Compile time, range analysis | `kestrel build` of voxels3d, checked / `--release` (target was under 10%) | 181 / 160 ms | 139 / 112 ms | 1.30x / 1.43x | 2026-10-20 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| Compile time, `@prove` x 3 | `kestrel build` of voxels3d, checked | 183 ms | 175 ms | 1.05x | 2026-10-21 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| Compile time, `@prove` + `@assume` | same, with an `@assume` around the noise code (target missed) | 296 ms | 175 ms | 1.69x | 2026-10-21 | [voxels3d](kestrel/bench/voxels3d/results.md) |
| Compile time, generics | `kestrel build` of voxels3d, no generics used | 202 ms | 203 ms | 1.00x † | 2026-10-25 | [kernels](kestrel/bench/kernels/results.md) |

## How it is measured

- **Machine:** one Windows 11 desktop with an AMD Ryzen 7 5800X (8 cores, 16 threads) and an NVIDIA RTX 3070, a 60 Hz display. It is a machine in daily use, so other programs are often open.
- **Rounds and medians:** every cell is a median. The `bench/2a` table is a median of 5 wall-clock runs from PowerShell (process start, about 15 ms, included); `maps` and `voxels` use 7; `heightmap` 5 to 9; the kernels use 3 to 15 rounds (9 for most of the tables above) and give ns per item with the range of the rounds in brackets, with the cost of building the data taken out. The particle demos take the median of one-line-a-second averages over about 11 seconds, first second dropped.
- **Taking turns:** when two builds are compared (old and new compiler, checked and `--release`, Kestrel and Rust), each round runs every variant once in turn, so they share the machine's conditions. Old compilers are rebuilt in a `git worktree`. Nothing is compared against a number saved from a different hour.
- **Noise:** loops move 6-30% with where the compiler puts the code (adding an unused function can flip two builds), so differences under 10% between different programs count as noise and sub-nanosecond loops move by 0.3 ns. `bench/maps` is the most sensitive: its old-compiler times ranged from 467 to 1,958 ms in one set of rounds. Each file says when a game, a build or another program was using the machine; those rows are marked † above.
- **Builds:** "checked" is the default build, with overflow, narrowing and bounds checks on (the compiler removes each check it proves cannot fail). `--release` turns the remaining checks off. Rust is `-O3` with overflow checks off unless a column says "checks on"; C is gcc `-O2`; C# is .NET 9 Release with tiered PGO. All variants of a benchmark print identical output, and the scripts check it.
- **Hidden windows:** the particle runs use `KESTREL_HIDDEN=1`, so the frame rate is not capped at 60 and "fps" means how fast the program could go. On 2026-10-11 hidden windows were for a while held at 60 fps; those runs were thrown away.

## How to rerun

Run from the folder named, on Windows with PowerShell. Kestrel needs `cargo build --release` in `kestrel/` first (the 2a, heightmap, voxels, voxels3d and maps scripts use `kestrel/target/release/kestrel.exe`).

| Folder | Commands |
|---|---|
| `kestrel/bench/2a` | `cd kestrel; cargo build --release -p kestrel-cli; cd bench\2a; .\run.ps1` (needs `rustc` and `gcc` on PATH) |
| `kestrel/bench/heightmap` | `.\run.ps1` (the results file gives no command; this is the script it was run with) |
| `kestrel/bench/kernels` | `cargo build --release`, then `py measure.py --rounds 3 --threads 1` and `py measure.py --rounds 3 --threads 16`; add `--only NAME` for kernels whose label contains `NAME` (`--only iface`, `--only noise`); `--keep-checks` and `--old DIR` add a column with the range analysis off and one for an older build |
| `kestrel/bench/maps` | `./run.ps1` |
| `kestrel/bench/particles` | `cargo build --release -p kestrel-cli`, `cd bench/particles/rust && cargo build --release`, then from `kestrel/`: `python bench/particles/measure.py --rounds 4 "kestrel-jit=KESTREL_HIDDEN=1 target/release/kestrel.exe run --release my_copy.kes" "rust-direct=bench/particles/rust/target/release/particles-rust --count 200000 --draw direct"` (more commands in the file) |
| `kestrel/bench/voxels` | `..\..\target\release\kestrel.exe build main.kes -o k_voxels.exe --release`, then `dotnet run -c Release --project cs -- 60` |
| `kestrel/bench/voxels3d` | `./run.ps1`; for C# without tiered compilation: `$env:DOTNET_TieredCompilation = "0"; .\cs\bin\out\Voxels3dBench.exe 8 par` |
| `kestrel/godot/bench` | `./run.ps1 <path to Godot console exe>` (add `bench3d.gd` for the 3D world) |
| `kestrel/demos/particles` | `KESTREL_HIDDEN=1 target/release/kestrel run --release demos/particles/particles.kes` from `kestrel/` |

The kernels folder is its own Cargo workspace, so `cargo test --workspace` does not build it. A single kernel runs with `target/release/kernels.exe step.kes --release`, and `--dump NAME` prints its Cranelift IR and machine code.

## Detailed results

- [`kestrel/bench/2a/results.md`](kestrel/bench/2a/results.md): fib, loop, collatz, mandelbrot, particles, sieve and swarm against Rust and C, from the first compiler to the arrays round.
- [`kestrel/bench/heightmap/results.md`](kestrel/bench/heightmap/results.md): the 16.7M-block heightmap milestone against Rust, before and after the KIR inliner.
- [`kestrel/bench/kernels/results.md`](kestrel/bench/kernels/results.md): small inner loops against Rust, with each compiler pass measured old against new: scalar replacement, range analysis, list methods, interface calls, SIMD vectors, generics, Cranelift 0.137, quiet reruns.
- [`kestrel/bench/maps/results.md`](kestrel/bench/maps/results.md): Kestrel `map` against C# `Dictionary`, integer and string keys.
- [`kestrel/bench/particles/results.md`](kestrel/bench/particles/results.md): the particles demo against a Rust port on the same raylib, and the investigation of the upload slowdown.
- [`kestrel/bench/voxels/results.md`](kestrel/bench/voxels/results.md): the 2D chunk mesher against C#, standalone and inside Godot 4.7.2 (with GDScript about 100x slower).
- [`kestrel/bench/voxels3d/results.md`](kestrel/bench/voxels3d/results.md): the 3D voxel world against C#, GDScript and a Rust port, `par for`, range analysis, `@prove`/`@assume` compile costs, loop hoisting, the Cranelift upgrade and the hand-tuned copy.
- [`kestrel/demos/particles/results.md`](kestrel/demos/particles/results.md): the particles demo's own times, one draw call per dot against bulk drawing.
