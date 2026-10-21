# 3D voxel world benchmark: Kestrel vs C# vs GDScript

`voxels3d.kes` builds a full 3D voxel world, 8×4×8 chunks of 32³ blocks (256×128×256, 8.4M blocks):

- **Density:** 3D value noise in three octaves (every 32, 16 and 8 blocks), each blending the 8 lattice corners around the block, pulled toward air higher up. The world has hills, overhangs, floating islands and caves.
- **Sampling:** each chunk samples 34³ points (the chunk plus a one-block border), 20.1M samples and 482M lattice lookups for the world.
- **Meshing:** a face for every side of a solid block that touches air, as vertices, normals and indices (4, 4 and 6 per face).
- **Memory:** each chunk's mesh lives in an `arena { }` and is freed when the chunk is done.

`cs/` is the same code in C# (.NET 9, Release, tiered PGO). All versions make 463,749 faces. GDScript's floats are 64-bit, but its column below matches exactly too.

About 95% of the time is the density pass; meshing is the rest.

## Standalone

Time to build the world, with process startup (an empty world) subtracted; median of 7 runs per pass. The columns are the KIR inliner's versions (see below).

| World build | Inliner v1 (3 passes) | v2 (2 passes) | v3 (2 passes) |
|---|---:|---:|---:|
| Kestrel exe (checks on) | 1,616–1,775 ms | 1,258–1,287 ms | 1,182–1,232 ms |
| Kestrel exe (`--release`, checks off) | 1,166–1,268 ms | 809–817 ms | **481–505 ms** |
| C# .NET 9 | 1,416–1,522 ms | 1,478–1,517 ms | 1,441–1,464 ms |
| C# .NET 9, `[AggressiveInlining]` on the six helpers | | 542 ms | 604–642 ms |

Startup: Kestrel exe 7–16 ms, .NET 67–108 ms.

**The inliner made the difference:**
- **v1, the first version, tied C#.** The KIR inliner only took functions up to 64 KIR nodes, so `octave` stayed a call; RyuJIT doesn't inline it either.
- **v2 takes functions up to 256 nodes,** with a budget of 4,096 added nodes per caller so nested inlining can't blow up code size. `octave` is inlined into the sampling loop with its constant `shift`: 30% faster.
- **v3 also inlines the calls inside the arguments of an inlined call,** and follows chains 8 deep instead of 4. A Sonnet code review found that v1 and v2 left `lattice` a call in `mix(lattice(..), lattice(..), fx)`, because `mix` was inlined and its arguments never revisited. v3 is another 40% faster.
- **Kestrel `--release` now beats C# even with `[MethodImpl(MethodImplOptions.AggressiveInlining)]`** on `Hash`, `Lattice`, `Smooth`, `Mix`, `Octave` and `Density` (0.48–0.51 s vs 0.54–0.64 s). Performance-minded C# often puts that attribute on hot helpers, so this is the fair comparison of code quality. Against C# as written, Kestrel `--release` is 3x faster.
- **With checks on, Kestrel is about 20% faster than C# as written, and 2.4x slower than `--release`.** Each lookup has five checked 64-bit multiplies, plus checked adds, shift-amount checks, a narrowing check and bounds checks. Each check is a branch that keeps its work inside the loop, so the checked build gained little from v3. C# checks only array bounds.
- **The other benchmarks gained too** (`--release`): heightmap went from 78–88 ms to 52 ms (2x Rust, was 3.1x), and the 2D voxels from 10.9 to 8.1–8.6 ms per build. The 2a kernels are unchanged within noise; they make few calls.
- **Divides weren't the problem.** While v2 was 1.5x behind inlined C#, multiplying by the reciprocal of `scale` instead of dividing by it gave the same time.

## Inside Godot 4.7.2

`kestrel/godot/bench/bench3d.gd` runs Kestrel (through the add-on), C# (the same `Voxels3dCs.cs`, linked into the Godot project) and GDScript in one Godot process, and stops if their face counts differ. World: median of 5 calls. Column (chunks (0, 0..3, 0), 1/64 of the world): median of 9 calls, 3 for GDScript.

| | World, inliner v1 (1 pass) | World, v2 (2 passes) | World, v3 (2 passes) | Column, v3 |
|---|---:|---:|---:|---:|
| Kestrel (checks on) | 1,813 ms | 1,216–1,294 ms | 1,131–1,173 ms | 18.7–20.8 ms |
| Kestrel (checks off) | 1,307 ms | 774–837 ms | **482–516 ms** | 8.1 ms |
| C# | 1,227 ms | 988–1,026 ms | 1,027–1,100 ms | 15.9–19.6 ms |
| GDScript | about 180 s | about 170 s | about 160–190 s | 2,483–2,957 ms |

GDScript's world time is its column time × 64.

- **Kestrel with checks off is about 2x faster than C#** inside Godot. With checks on it is about 10% slower.
- **C# runs faster inside Godot than standalone** (1.0–1.2 s vs 1.4–1.5 s). Godot hosts .NET with its own settings; which one matters is not investigated.
- **GDScript is about 150x slower than C# and 350x slower than Kestrel `--release`.** A real game would mesh in C# or Kestrel, or in GDScript with a native noise library.

## With `par for`

`parallel.kes` builds the same world with `par for c in chunks { c.build(seed) }`; `build` is a `pure` method with `var self`. `cs/` has a matching `Parallel.For` mode. This machine has 8 cores and 16 threads, and all versions make 463,749 faces.

Standalone: median of 7 runs, startup subtracted, two passes. The single-thread rows from the same passes are repeated for comparison.

| World build | One thread | All threads | Speedup |
|---|---:|---:|---:|
| Kestrel, checks on | 1,065–1,092 ms | 144–147 ms | 7.4x |
| Kestrel `--release` | 454–463 ms | 63–66 ms | 7.1x |
| C# .NET 9 (`Parallel.For`) | 1,439–1,465 ms | 368–373 ms | 3.9x |
| C# .NET 9, tiered compilation off | 1,177 ms | 208 ms | 5.7x |

- **Kestrel scales about 7x on 8 cores.**
  - The one-thread row runs the `par for` program with `KESTREL_THREADS=1`, which costs nothing over the plain loop (454 ms both).
  - All threads finish the world in about 65 ms with `--release`, and in about 145 ms with checks on.
- **C# scales less, partly from JIT warm-up.**
  - .NET starts with quick, unoptimized code and recompiles hot methods while the program runs. That is only a small part of a 1.4 s run, but a much bigger part of a 0.37 s one.
  - With tiered compilation off (`DOTNET_TieredCompilation=0`, everything optimized up front), `Parallel.For` takes 208 ms. That is still 3.2x Kestrel `--release`, which is about the gap the single-threaded runs show.
  - Server GC didn't help (409 ms).

Inside Godot 4.7.2 (one pass, with the C# methods warmed up by earlier calls):

| World build | Time |
|---|---:|
| Kestrel `par for` (checks off) | 62 ms |
| C# `Parallel.For` | 149 ms |
| Kestrel, one thread (checks off) | 581 ms |
| C#, one thread | 1,255 ms |

The machine was noisier during this Godot run than during the earlier ones (the checked one-thread row took 1,434 ms vs 1,131–1,294 before).

## Reproduce

```powershell
# standalone (from kestrel/bench/voxels3d)
./run.ps1
# inside Godot (from kestrel/godot/bench)
./run.ps1 <path to Godot console exe> bench3d.gd
# C# without tiered compilation
$env:DOTNET_TieredCompilation = "0"; .\cs\bin\out\Voxels3dBench.exe 8 par
```

The `[AggressiveInlining]` row is a local copy of `cs/` with `[MethodImpl(MethodImplOptions.AggressiveInlining)]` added to those six methods; it isn't checked in.

## Range analysis

The range analysis (`docs/superpowers/specs/2026-10-19-range-analysis-design.md`) leaves out each check it proves can't fail, in the checked build as well as `--release`. Measured on 2026-10-20 (evening), the old compiler at commit b7166fe (the commit before the round) against the new one at dc78375 (the later commits change only docs), same machine as above (Ryzen 7 5800X, Windows 11). The user had said the machine was free: no game was running, but the usual background was (Roblox's launcher idle, OneDrive, Defender; 25-40% CPU before the run). No idle-priority watcher during timings. Four exes (old and new, checked and `--release`) built from `main.kes`, then timed in turns, one of each per round, with `Measure-Command` as in `run.ps1`; the startup of an empty world (12 ms) is taken out. All four print 463,749 faces.

**World build, one thread** (median of 9 runs, range in brackets):

| World build | Old | New | New over old |
|---|---:|---:|---:|
| Kestrel exe (checks on) | 1,415 ms (1,392-1,503) | **643 ms** (609-720) | 0.45 |
| Kestrel exe (`--release`) | 594 ms (578-685) | 592 ms (568-734) | 1.00 |
| **Checked over `--release`** | **2.38x** | **1.09x** | |

- **The target is met: checked over `--release` is 1.09x against the target of at most 1.2x** (it was 2.4x). The checked build is 2.2 times faster, 1,415 ms to 643, and now takes about the time the `--release` build did before the round; `--release` is not slower (594 and 592, ranges overlapping).
- The old rows are higher than the "v3" column above (1,182-1,232 and 481-505 ms) for the same compiler: the machine was slower this evening. Compare the rows of this table with each other, not with the older ones.
- With `par for` on one thread (`KESTREL_THREADS=1`, 5 runs): checked 1,410 (1,378-1,529) to **628** (604-793), `--release` 603 (571-728) to 589 (580-695): checked over `--release` 1.07x (was 2.34x).
- With `par for` on all threads (5 runs, whole run including about 10 ms of startup): checked 242 ms (234-283) to **107** (102-113), `--release` 102 (100-110) to 102 (100-109). Checked over `--release` 1.05x.

**What is left** (`kestrel check --report main.kes`): checked 339 checks, 318 removed, 21 left; `--release` 8 checks, 7 removed, 1 left (`mesh`'s bounds check). Totals count an inlined body in each function that holds it. `hash`, `lattice`, `smooth`, `mix`, `octave` and `density` are never called any more (inlined everywhere), and **no check is left in the inlined `hash`, `lattice`, `smooth`, `mix` and `octave` code**. The checks left are the ones the design predicted, about a dozen per sample, none of them in the noise lookups:
- **Per sample, 14 checks** (lines 62-65 of `voxels3d.kes`, in `solids`): the chunk offsets `cx * 32`, `(cx * 32) + x` and `((cx * 32) + x) - 1`, and the same for `y` and `z` (9 checks, because `cx`, `cy` and `cz` are parameters that can be any `int`); `seed + 1`, `seed + 2` and `y - ((4 * 32) / 2)` in `density`; `out[i]`; and `i + 1`. The last two stay because `i` is a separate counter bumped once per iteration of three nested loops, so nothing shows that it stays below `out.len` (34 cubed): a product in a fact.
- **Per block of the meshing pass, 6 checks** (line 98): the chunk offsets `cx * 32 + x` for `x`, `y` and `z` again, and one `faces + ...` in `bench`. The report lists no `solid[i + k]` bounds check of the mesher, which the design expected to stay: `solids` is inlined into `chunk_faces`, so the array's length (34 cubed) is known there, and the index is a bounded expression of three loop variables (I read this off the report and the source; I did not confirm it another way).

The chunk offsets don't depend on the loop they are in, and `out[i]` needs a product in a fact; hoisting loop-invariant checks and `@assume` are the next steps. Under `--release` only `mesh`'s bounds check is left. The old compiler has no report, so the "about 250 before" is the design's estimate (five checked multiplies, adds, shift checks, a narrowing check and bounds checks in each of the 482M lookups), not a count.

**Compile time** (`kestrel build` of the whole program, to an exe, with the MSVC link; median of 9 runs alternating old and new, range in brackets):

| Program | Old checked | New checked | Old `--release` | New `--release` |
|---|---:|---:|---:|---:|
| `voxels3d/main.kes` | 139 ms (132-207) | 181 ms (172-195) | 112 ms (107-114) | 160 ms (153-211) |
| `demos/particles/particles.kes` | 141 (131-220) | 144 (139-150) | 135 (131-146) | 138 (135-146) |
| `demos/voxels/voxels.kes` | 199 (192-248) | 263 (258-290) | 162 (156-166) | 234 (224-239) |

- **The target (the analysis adds under 10%) is missed on the voxel programs and met on the particles demo.** The voxel world builds 30% slower checked and 43% slower with `--release` (+42 and +48 ms); the voxel viewer 32% and 44% (+64 and +72 ms); the particles demo 2%.
- **The cost is the analysis itself, about 45 ms on the voxel world.** Compiling `main.kes` to object code with `kernels.exe --dump main` takes 58 ms with `--keep-checks` and 102 ms with the analysis (checked), 32 and 80 ms with `--release`: +44 and +48 ms, the same as the build's difference. It is the same in both builds because the analysis tracks everything either way; and it shows more in `--release` because that build otherwise compiles less.
- **Why the voxel programs and not the demo:** the voxel world inlines `octave`, `lattice` and `hash` into one big sampling loop with many tracked values (the report: one function, `mesh`, holds 337 of the 339 checks), so the fixed-point passes over the loops, the facts between terms (up to 64) and the budget of statement visits do a lot of work; the particles demo's functions are small. I did not profile the analysis; the knobs the design names are `MAX_TERMS` and the fact-work budget. `kestrel check` (front end only) is not affected: 21 ms; `check --report` on the voxel world takes 90 ms.
- A build is 110-260 ms either way, so the absolute cost is 40-70 ms per build of a large program. It is a miss against the target, not against any budget I know of.
