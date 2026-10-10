# Voxel mesh benchmark: Kestrel vs C#

The Godot demo's chunk mesher (`voxels.kes`) builds a 4×4 world: 16 chunks of 32×32 columns, with vertices and normals, 355,020 vectors in all. `cs/` is the same code in C# (.NET 9, Release, tiered PGO). The outputs match exactly.

## Standalone, 2026-10-10

Each program builds the world 60 times. The table gives the time per build, with process startup (measured separately) subtracted; median of 7, two passes.

| Per build | Before block recycling | After |
|---|---:|---:|
| Kestrel exe (checks on) | 19.5 ms | 12.1–12.4 ms |
| Kestrel exe (`--release`, checks off) | 18.3 ms | 10.8–11.0 ms |
| C# .NET 9 | 11.0 ms | 10.6–11.9 ms |

Startup: Kestrel exe 9–12 ms, .NET 78–91 ms.

**What changed:**
- The first version lost to C# on growing lists. The noise math alone was a tie (Kestrel 220 ms vs C# 202 ms for 60 runs).
- The cause was memory, not code: freed arena blocks went back to the OS, so every new list paid page faults for fresh memory. C#'s GC reuses memory instead.
- The runtime now keeps freed blocks per thread (up to 256 MB) and reuses them.
- A growing list no longer zeroes its unused capacity.
- `bench` frees each round's lists with `arena { }`, the idiomatic way.

## Inside Godot 4.7.2

`kestrel/godot/bench` runs Kestrel (through the add-on), C# (a Godot C# script) and GDScript in the same Godot process. Median of 9 calls (3 for GDScript), three runs; the machine was noisy.

| | Build all 16 chunks | Fetched from GDScript as arrays (32 calls) |
|---|---:|---:|
| Kestrel (checks off) | 10.8–15.6 ms | 15.9–17.5 ms |
| Kestrel (checks on) | 12.8–16.8 ms | 19.7–21.6 ms |
| C# | 10.9–16.3 ms | 12.7–15.3 ms |
| GDScript | 1,472–1,499 ms | 1,434–1,497 ms |

- **The computation is a tie with C#,** and both are about 100x faster than GDScript.
- **Calling from GDScript costs Kestrel a few ms more than C#** over 32 calls. Each call hands its work to the Kestrel thread (which keeps a runtime panic from taking down the editor) and copies the result into a packed array. The first version of that copy converted element by element, and took 39 ms.

## Reproduce

```powershell
# standalone (from kestrel/bench/voxels)
..\..\target\release\kestrel.exe build main.kes -o k_voxels.exe --release
dotnet run -c Release --project cs -- 60
# inside Godot (from kestrel/godot/bench)
./run.ps1 <path to Godot console exe>
```

## After the inliner changes for the 3D voxel benchmark

`bench/voxels3d` changed the KIR inliner: callees up to 256 nodes (was 64), and calls inside the arguments of an inlined call are inlined too. Per build, median of 7, one pass each; the machine was slower during the last pass (C# 13.1 ms).

| Per build | Before | 256-node limit | + calls in arguments |
|---|---:|---:|---:|
| Kestrel exe (checks on) | 12.1–12.4 ms | 11.4 ms | 12.6 ms |
| Kestrel exe (`--release`) | 10.8–11.0 ms | 8.1 ms | 8.6 ms |
| C# .NET 9 | 10.6–11.9 ms | | 13.1 ms |
