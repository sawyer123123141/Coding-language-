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
