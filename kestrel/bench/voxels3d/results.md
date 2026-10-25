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

## Tuned copy (2026-10-20)

`voxels3d_tuned.kes` is `voxels3d.kes` tuned by hand, as fast as Kestrel makes it today with source changes only; `voxels3d.kes` stays the benchmark, unchanged. It builds the same world block for block: every one of the 24 lattice hashes per sample is still finished and divided by 65535, every float operation of the noise is done in the original's order, and the mesher makes the same faces in the same order. A comparison program (not checked in) built both modules' `solids` and `mesh` for all 256 chunks and found every solid flag, vertex, normal and index equal, checked and `--release`; it reports 30 million differences when one constant is changed by 0.00001. What moved is work the original repeats: the parts of a hash and of an octave that depend only on y and z are taken once per row of 34 samples, the parts that depend only on x once per chunk. It builds on its own (`kestrel build voxels3d_tuned.kes`, which prints 463,749); `run.ps1 -Tuned` adds it to the standard run.

**Compiler:** a copy of `kestrel.exe` (with `kestrel_runtime.lib` next to it) taken at commit 8105e9e, whose last compiler change is 0ad0280; `cargo build --release` at 8105e9e compiled nothing new, so the copy matches it. Round B was being built on `main` at the same time, so every build and run used the copy, from `C:\kt\voxtuned`.

**Rust:** there was no Rust version of this benchmark, so a straight port was written for this measurement, the way the Kestrel and C# versions are written, with no hand tuning (`voxels3d.rs`, kept outside the repository at `C:\kt\voxtuned\rust\`; rustc 1.97.0, `-C opt-level=3`, the checked row with `-C overflow-checks=on`). It prints 463,749.

**World build, one thread** (Ryzen 7 5800X, Windows 11). Two sessions, each taking turns (frozen, tuned, Rust, C#, the empty worlds, then again), 9 and 7 runs; medians, with the range of the whole runs in brackets, and the world column with the startup taken out (Session 2 uses Kestrel's startup for Rust). The machine was busier than in the "Range analysis" section (Roblox was running; the frozen `--release` build took 488 ms on a quiet stretch earlier the same evening and 529-546 here), so compare rows within a session:

| | Session 1: whole run | world | Session 2: whole run | world |
|---|---:|---:|---:|---:|
| Kestrel frozen, checks on | 602 ms (586-618) | 587 ms | 574 ms (572-598) | 560 ms |
| **Kestrel tuned, checks on** | 429 ms (415-466) | **414 ms** | 423 ms (410-434) | **409 ms** |
| Kestrel frozen, `--release` | 561 ms (546-581) | 546 ms | 543 ms (525-555) | 529 ms |
| **Kestrel tuned, `--release`** | 429 ms (410-452) | **414 ms** | 418 ms (416-430) | **404 ms** |
| Rust, overflow checks on | 451 ms (440-517) | 437 ms | 433 ms (427-445) | 419 ms |
| Rust, release | 441 ms (433-480) | 427 ms | 435 ms (418-449) | 421 ms |
| C# .NET 9 (as written) | 1,661 ms (1,636-2,864) | 1,573 ms | 1,615 ms (1,589-1,658) | 1,527 ms |
| Startup: Kestrel (empty world) | 15 ms (14-62) | | 14 ms (14-20) | |
| Startup: Rust, .NET (width 0) | 14 ms (13-17), 88 ms (85-97) | | -, 88 ms (84-111) | |

- **The tuned copy is about 25% faster than the frozen one** (tuned over frozen: `--release` 0.76 in both sessions, checked 0.71 and 0.73), and **its checked build costs nothing over `--release`** (1.00 and 1.01; the frozen one 1.08 and 1.06).
- **The tuned copy is level with a straight Rust port** (0.96-0.97x Rust `--release`; the frozen copy is 1.26-1.28x). The Rust port needs none of the hand changes below, because LLVM hoists loop-invariant code and reassociates on its own: most of what follows is what the Rust compiler already does.
- C# as written is about 3.8x the tuned copy.
- The inner sampling loop went from 703 instructions per sample to 465 (`--release`, read from the disassembly). About 420 of the 465 are the 24 hashes and the 21 mixes, so the scalar loop is close to its floor: what is left is about 85 cycles a sample (estimated from the time, at about 4.7 GHz), so about 5 instructions a cycle, near the core's dispatch width. Going further needs SIMD (see the last gap).
- **What `kestrel check --report` shows:** checked, 16 checks are left and none of them is per sample (the frozen copy has 14 per sample): 8 per chunk (`cx * 32 - 1` and the like, `seed + 1`, `seed + 2`), `bx + x` once per column of the x table, `by + y` and `y - 64` once per layer, `bz + z` once per row, 3 chunk offsets per mesh and `faces + ...` per chunk. `--release`: none (the frozen copy keeps `out[i]`).

**Each hand change, as a compiler gap.** Measured in two more sessions (5 runs each, checked and `--release` timed separately, whole runs: frozen 609 and 559 ms), adding one change at a time, in this order: step 1's plain locals, then its workaround 1 together with step 2, steps 3 and 4, step 5, step 1's workaround 2, step 7. The copies for the steps are not checked in. Except where it says otherwise, a change makes the same world.

1. **A compiler gap: the compiler should hoist loop-invariant work out of loops, and must not sink work written before a loop into it.** (Found and fixed on 2026-10-21: see "Loop-invariant code motion" below.) In every loop I looked at, Kestrel's code has no loop-invariant code motion: Cranelift's e-graph pass places every pure value at its use and, on Kestrel's loops, never hoists it back out. A small program shows it: `k`, written before the loop, is computed again on every iteration (`imul ...,0x60000005` inside the loop in the `--release` exe's disassembly; the same with `for`):

   ```
   pure f(seed: int, n: int) -> int {
       var acc: i64 = 0
       let k = i64(seed) * 1610612741
       var i = 0
       while i < n {
           acc = acc ^ (i64(i) * 3 ^ k)
           i += 1
       }
       int(acc & 0xFFFF)
   }
   ```

   The e-graph pass does run (`i64(i) * 8` becomes a shift), and Cranelift's elaborator has LICM, so something in the loops Kestrel emits stops it; I did not find what. Next step: dump that program's CLIF (`kernels.exe --dump`) and Cranelift's `elaborate_licm_hoist` statistic. In the frozen copy every sample recomputes the chunk offsets, `seed * 1610612741`, `y * 19349663`, `z * 83492791` and the y and z cell fractions of all three octaves. Effects:
   - **Moving the y, z and seed parts to the row as plain locals did nothing** (`--release` 565 ms against 559: the code generator moved them back into the x loop), except in the checked build (609 to 584 ms, -4%), because checks are not pure and stay where they are written. That 4% is the existing gap "hoisting loop-invariant checks" (follow-ups, range analysis).
   - **Workaround 1, the row's values in a small array** written once per row and read in the x loop (a load can't be sunk): 565 to 494 ms `--release` (-13%), 584 to 505 checked (-14%), from the plain locals above; 559 and 609 to 494 and 505 from the frozen copy. Kept for the 7 float values (`rf`).
   - **Workaround 2, the row's 12 hash parts as values the loop carries** (`var p0 = ...` before the x loop and a no-op `p0 |= 0` at its end, so each is a loop variable the code generator keeps rather than recomputes): 454 to 427 ms `--release` (-6%), 472 to 449 checked (-5%), measured last but one, after step 5, against keeping them in the array. It also drops the 12 masks per sample of step 4, since the analysis knows a loop variable's range.
2. **A compiler gap: the compiler should reassociate xor (and add) chains so that the loop-invariant operands are combined outside the loop.** `x * A ^ y * B ^ z * C ^ seed * D` is written as the x part xored with a row part made once per row. Measured together with workaround 1 above (the row parts are the values in the array).
3. **A compiler gap: the compiler should drop a mask whose cleared bits nothing reads (demanded bits), and move a mask through xor onto operands it can hoist.** The hash's second `& 0x7FFFFFFF` is redundant (only the low 16 bits of `p ^ (p >> 15)` are kept, and they come from bits 0..30), and the first one distributes over the xor, so each x part and row part is masked where it is made: 494 to 458 ms `--release` (-7%), 505 to 484 checked (-4%). Done naively it made the checked build 13% slower (573 ms), which is the next gap.
4. **A compiler gap: the range analysis should know a range for an array's elements (or Round B's `@assume` should say it).** With the masks moved into the rows, the hash's multiply keeps its overflow check, 24 per sample, because a value read from the array has the whole `i64` range. Masking again where it is read (12 `and`s per sample in both builds; 6 in the final copy, for the x table) takes the checks back out: 573 to 484 ms checked; `--release` pays about 2% (449 to 458, within the noise).
   - Related: storing the masked x parts as `int` (they fit) would let the load sign-extend for free, but the analysis's xor rule only bounds operands it knows are not negative, so the 24 checks came back (an earlier session: checked 396 to 515 ms, +30%; `--release` 404 to 396, noise). **A compiler gap: the xor, or and and rules should bound signed operands too** (`|a|` and `|b|` below 2^31 give `|a ^ b|` below 2^31).
5. **Work that depends only on x, once per chunk** (the x parts of the hashes and the x fractions, a 34-entry table each): 458 to 454 ms `--release` (-1%), 484 to 472 checked (-2.5%). This is loop-invariant code motion out of the outer loops into a table, which compilers rarely do; it is listed for completeness and is small. Not a gap worth building on its own.
6. **A compiler gap (small): the compiler should turn a float division by a power-of-two constant into a multiply** (exact): `float(x & mask) / scale` is written `* 0.03125` and so on. Not measured alone: once the fractions moved to the rows and the table, none is left per sample.
7. **A compiler gap, already listed ("facts across calls"): a function's returned array length should reach its caller.** `solids` is now too big to inline into `mesh` (822 nodes, limit 256), so its 39,304-item length is unknown there and the mesher's six neighbour reads keep their bounds checks, in `--release` too. A never-taken `if solid.len != PAD * PAD * PAD { return m }` tells the analysis the length; with the block's position as `base + vec3(x, y, z)` (exact) instead of `vec3(ox + x, ...)`, no check is left per block in either build. Effect: none measurable (427 to 432 ms `--release`, 449 to 446 checked).
8. **A language gap (small): a constant can't use a conversion.** `let INV = f64(1.0) / f64(65535.0)` is refused ("constants can only use literals, operators, ..."); `let INV: f64 = 1.0 / 65535.0` works.
9. **A compiler gap: SIMD.** What is left is 24 independent hash chains and 21 mixes per sample in scalar code, near the core's dispatch width. Doing four corners at once needs vector types (`vec4` and `ivec4` are in the language design but not built) or a compiler that combines independent scalar chains (SLP vectorisation). Not measured.

**Tried, no effect (not used):**
- **Multiplying by the reciprocal of 65535 in `f64`** instead of dividing in `float`: `float(f64(v) * (1.0 / 65535.0))` equals `float(v) / 65535.0` for every `v` from 0 to 65535 (checked one by one; the same multiply in `float` differs for 512 of them). It takes the 24 divisions per sample off the divider: 432 against 432 ms `--release` (450 against 446 checked); on the frozen copy's code, 3 runs, 467 to 491 `--release` and 509 to 494 checked (noise). The divider isn't the limit.
- **Unrolling `quad`'s `for k from 0 to 4 { normals.add(n) }`:** 430 against 432 ms `--release`, 435 against 446 checked (noise). Meshing is about 10% of the world build (about 40-45 ms in both copies, measured by building only the solid flags).

## `@prove` and `@assume` (Round B, 2026-10-21)

Compile time only: Round B changes no code that a program runs. Spec: `docs/superpowers/specs/2026-10-18-at-annotations-design.md` ("As built (Round B)"). The old compiler is the commit before Round B (a0f42fc, built in a worktree at `C:\kt\rb-old`, removed afterwards); the new one is 5be3f31. Both release builds, `CARGO_BUILD_JOBS=2`. `kestrel build` of `main.kes` to an exe (with the MSVC link), one warm-up, then 9 runs of each taking turns, timed with `Measure-Command`; medians, range in brackets. Same machine as above (Ryzen 7 5800X, Windows 11), lightly loaded, no game running. The annotated programs are scratch copies of `voxels3d.kes` outside the repo (`C:\kt\rb-annot`), with `@prove(no_alloc)` on `hash`, `lattice` and `octave` and, in one copy, `@assume(cx >= 0 and cx < 1000 and cy >= 0 and cy < 1000 and cz >= 0 and cz < 1000 and seed >= 0 and seed < 1000000)` as the first statement of `solids`. All contracts hold, and the assumption gets no "removes no check" warning. All builds print 463,749 faces.

**No annotations** (old against new, `main.kes` of this folder; target: no measurable change, under 2%):

| Build | Old | New | New over old |
|---|---:|---:|---:|
| checked, run 1 | 178 ms (170-190) | 181 ms (171-191) | +1.7% |
| checked, run 2 | 178 ms (174-198) | 181 ms (174-185) | +1.7% |
| `--release`, run 1 | 157 ms (151-185) | 159 ms (149-178) | +1.3% |
| `--release`, run 2 | 162 ms (157-180) | 160 ms (153-166) | -1.2% |

- **Met, at the edge:** the checked build is 3 ms slower in both sets (+1.7%, under the 2% target), with the ranges overlapping completely; `--release` is level (pooled medians of 18: 160.5 and 159.5 ms). Round A's own table for the same program says 181 (172-195) and 160 (153-211).
- `kestrel check --report main.kes`, checked and `--release`, is byte-identical between the two compilers (184 and 164 lines). `kestrel check main.kes` takes 20 ms in both. The exe is 512 bytes smaller (220,160 against 220,672 bytes; the runtime changed).

**With annotations** (new compiler only, `kestrel build` of each copy):

| Copy | Checked | `--release` |
|---|---:|---:|
| no annotations | 175 ms (169-180) | 155 ms (146-158) |
| `@prove(no_alloc)` x 3 | 183 ms (178-187), +4.6% | 159 ms (150-165), +2.6% |
| the same and the `@assume` in `solids` | **296 ms (290-303), +69%** | **255 ms (246-269), +65%** |

- **Contracts alone: target met** (within 10%). **With the assumption: target missed.** The cost is the range analysis of `solids`, which holds the inlined noise code (46 of the 75 ms of analysis Round A measured on this program): an assumption's block runs twice in the deciding pass. It is more than twice the analysis of that function, and I did not profile it. A bigger fact matrix (the assumption adds terms) is the first guess.
- The cost follows the size of the block an assumption wraps. The same kind of assumption (`@assume(seed >= 0 and seed < 1000000 and width >= 0 and width < 1000)`) as the first statement of `bench`, whose body is a small loop around a call, costs nothing: 185 ms (175-280) against 187 ms (179-211) for the three contracts alone, and `--release` 169 (155-177) against 165 (155-177) (a later set, when the machine was a little slower than for the table above).

**`kestrel check`** (front end alone without annotations):

| Copy | Checked | `--release` |
|---|---:|---:|
| no annotations (old: 20 ms, 18-22) | 20 ms (19-25) | 20 ms (19-22) |
| `@prove(no_alloc)` x 3 | 98 ms (89-101) | 72 ms (68-77) |
| the same and the `@assume` in `solids` | 278 ms (264-283) | 271 ms (264-289) |

- **With an assumption: target missed.** 2.84 times the contracts-only time checked and 3.76 times `--release` (target at most 2.2). A program with any annotation now runs the analysis in `kestrel check` (about 50-80 ms here), and one with an assumption runs it for both builds, to give the warning.

**An assumption can cost precision** (`kestrel check --report main.kes`, totals, checked | `--release`):

| Copy | Checks removed | Left |
|---|---|---|
| `@prove` x 3 | 318 \| 7 | 21 \| 1 |
| and the `@assume` in `solids` | 303 \| 4 | 36 \| 4 |

- The assumption does its job in `solids` (the nine chunk offsets, `seed + 1`, `seed + 2` and `y - 64` are gone from the report), but 27 checks come back in `mesh`: the bounds checks `solid[i]`, `solid[i + 1]`, `solid[i + 34 * 34]` and the 24 overflow checks `base + 1`, `base + 2`, `base + 3` in the six inlined `Mesh.quad` copies.
- An `@assume(seed >= 0)` that removes nothing (and gets the warning) at the top of `solids` or of `mesh` does the same. A small program shows the bounds half: an `@assume` that wraps the end of a function loses the length of the array it returns, so the caller's `solid[i]` keeps its check. **Fixed in the final review's fix wave** (the returned local no longer goes through a temporary): with the assumption the report is now 306 removed and 33 left checked, 7 and 1 `--release` (the same as without it). The `base + k` half comes from Round A's term cap (`MAX_TERMS` = 64, `range/terms.rs`): an assumption's names become terms early, and in `mesh`, with the noise code inlined ahead of the six `quad` copies, `base` and `m.vertices.len` are pushed past 64. Overflow checks, so checked builds only. Not fixed (follow-ups).

## Loop-invariant code motion (2026-10-21)

Gap 1 of the tuned copy: Cranelift hoisted nothing out of Kestrel's loops. The cause was found by dumping the repro's IR before and after optimisation (`dump_lowered` in `kestrel/codegen`, new) and by building Cranelift with its `trace-log` feature for one run (its `elaborate_licm_hoist` statistic was 0). There are three causes, all now worked around in the code generator (`kestrel/codegen`, tests in `codegen/tests/licm.rs`):

1. **A Cranelift 0.116 bug that loses the loop.** The egraph pass places each pure value where it is used and hoists it out of a loop when its operands come from outside. To know which loops it is in, it keeps a stack while it walks the dominator tree, and when it reaches a block outside a loop it pops that loop and doesn't put it back. A loop header's children in the dominator tree are its body and its exit, in CFG reverse postorder. Cranelift's depth-first search takes a `brif`'s first target first, which puts that target *last* in reverse postorder, so Kestrel's `brif more, body, exit` handed the exit to the walk first. The walk popped the loop there, and by the time it reached the body the stack was empty, so nothing in the body was hoisted. The bug was fixed upstream in wasmtime PR #14383 (merged 2026-09-23, first released in wasmtime 50 on 2026-10-20); Kestrel is on Cranelift 0.116 (wasmtime 29). **Workaround** (`codegen/src/loop_exits.rs`): after lowering, each `brif` in a loop whose first target stays in the loop and whose second leaves it is swapped to `brif !c, exit, body`, with the comparison negated (for floats, its exact opposite, NaN included). This covers every loop's own test, `else { break }` and the checks, which already branch to their failure first. It can't cover everything: a block after an `if` always comes after the `if`'s branches, so a `break`, `return` or check inside an `if`'s branch still stops hoisting in the code after that `if` until Cranelift is upgraded.
2. **Redundant block parameters for the variables of outer loops.** Lowering seals every block at the end (`seal_all_blocks`), so `cranelift_frontend` gives each loop header a parameter for every variable read in the loop, before it knows what the back edge passes. An inner loop that reads the outer loop's `y` kept `block5(y5)`, entered with `jump block5(y1)` and `jump block5(y5)`, which is always `y1`, the outer header's parameter. Cranelift's own pass (`remove_constant_phis`) only removes a parameter whose single value is not itself a parameter, so `y5` stayed, and everything computed from `y` looked as if the inner loop changed it. **Fix** (`codegen/src/phis.rs`): a parameter whose arguments are all one value or the parameter itself is replaced by that value, repeated until none is left.
3. **Checked arithmetic as side effects.** A checked `+ - *` was one `sadd_overflow` (or `ssub_`, `smul_`) instruction giving both the value and the overflow flag. Cranelift's egraph treats an instruction with two results as a side effect and leaves it where it is written, so in the checked build `cy * 32 + y - 1` (written in the x loop) stayed in the x loop along with everything computed from it. **Fix** (`codegen/src/func/arith.rs`): the value now comes from a plain `iadd`/`isub`/`imul`, which can be hoisted, and the `_overflow` instruction gives only the flag for the check. The check itself still runs where it is written (the open gap "hoisting loop-invariant checks"). When nothing is hoisted, this costs one extra add or multiply per check.

The repro above now computes `k` once, before the loop. In a nested loop, a value that depends only on the outer loop goes to the outer loop's body. A value with a constant operand moves out one loop at a time, which is Cranelift's rule for constants: `seed * 1610612741` goes to the outer body, not the entry.

**The frozen copy's sample loop** (the innermost loop of `voxels3d.mesh`, machine instructions counted from `kernels.exe main.kes --dump voxels3d.mesh`; the tuned copy has 465 `--release`):

| | Old | New |
|---|---:|---:|
| `--release`: instructions per sample | 679 | **556** |
| `--release`: multiplies per sample | 45 | 30 (the 24 hash mixes and the 6 x parts) |
| Checked: instructions per sample | 728 | **647** |
| Checked: overflow checks per sample (`seto`) | 13 | 13 |

Every y, z and seed multiply of the hashes and the y and z fractions now leave the x loop. What is left is the other gaps of the tuned copy: the xor chains aren't reassociated (`x * A ^ y * B ^ z * C ^ seed * D` still does three xors per hash in the loop), and the masks and the checks are still there.

**World build, one thread** (Ryzen 7 5800X, Windows 11; whole runs, so about 11-13 ms of startup is included). The old compiler is the `C:\kt\voxtuned` copy (commit 0ad0280's compiler, the base of this branch), and the new one is this branch. Each exe was built by its compiler, and all of them print 463,749. Two sessions of 9 rounds, each taking the variants in turn; medians, with ranges in brackets. The machine was noisy: Round B was being built on `main` in another session, and the frozen `--release` build's median moved from 627 to 557 ms between the sessions. So compare old with new within a session:

| | Session 1 old | Session 1 new | Session 2 old | Session 2 new | New over old |
|---|---:|---:|---:|---:|---:|
| Frozen, `--release` | 627 (560-677) | **556** (498-773) | 557 (544-612) | **500** (463-526) | 0.89, 0.90 |
| Frozen, checks on | 626 (575-690) | 618 (583-725) | 608 (581-644) | 595 (559-629) | 0.99, 0.98 |
| Tuned, `--release` | 473 (422-548) | 451 (424-598) | 429 (419-488) | 447 (412-480) | 0.95, 1.04 |
| Tuned, checks on | 491 (444-612) | 472 (441-568) | 437 (413-452) | 435 (420-455) | 0.96, 1.00 |

Session 1's new compiler had causes 1 and 2 fixed but not cause 3; session 2 had all three, and its checked sample loop is the 647 instructions above. Session 1's checked frozen build lost no instructions (635 in the sample block, as before).

- **The frozen `--release` build is 10-11% faster** (627 to 556 ms and 557 to 500). The goal was to approach the tuned copy (0.76 of the frozen time); this gets 0.90. The tuned copy is still 0.86 of the new frozen time in session 2 (429 against 500), because of the reassociation, the masks and the hand-made tables.
- **The checked frozen build is only 1-2% faster** (608 to 595 ms in session 2), though its sample loop lost 81 instructions. The 13 overflow checks per sample are still in the loop, each an `add`/`imul` with `seto` and a branch on it. Taking checks out of loops is the follow-up "hoisting loop-invariant checks".
- **The tuned copy doesn't move** (0.95-1.04, within the noise): its hand changes had already taken the invariant work out of the loop.

**Kernels** (`py measure.py --rounds 5 --threads 1 --old DIR`, `--release`, ns per item, median of 5 rounds with ranges; the old column is a `bench/kernels` build from before the fix; the new column was built with all three fixes):

| Kernel | New | Old | New over old |
|---|---:|---:|---:|
| `step.kes` | 5.54 (5.38-5.75) | 5.70 (5.37-5.85) | 0.97 |
| `step_index.kes` | 7.80 (7.49-7.97) | 7.86 (7.51-8.02) | 0.99 |
| `step_scalar.kes` | 5.64 | 5.62 | 1.00 |
| `step_pow.kes` | 21.85 | 22.35 | 0.98 |
| `copy.kes` | 0.86 (0.84-1.24) | 0.91 (0.86-1.34) | 0.95 |
| `filter.kes` | **4.97** (4.94-5.35) | 5.97 (5.92-6.87) | **0.83** |
| `filter_loop.kes` | **4.90** (4.79-5.09) | 5.46 (5.35-5.54) | **0.90** |
| `map.kes` | 2.03 | 2.00 | 1.02 |
| `map_loop.kes` | 2.00 | 2.11 | 0.95 |
| `sort_by.kes` | 180.52 | 181.67 | 0.99 |
| `sort_by_loop.kes` | 115.86 | 118.83 | 0.98 |
| `iface_hp.kes` | 5.06 | 5.23 | 0.97 |
| `iface_hp_direct.kes` | 0.34 (0.32-0.37) | 0.53 (0.53-0.58) | 0.64 |
| `iface_step.kes` | 5.46 | 5.61 | 0.97 |
| `iface_step_direct.kes` | 0.58 (0.56-0.59) | 0.89 (0.80-0.97) | 0.65 |

- **No kernel is slower** beyond the noise: the largest rise is `map.kes`, +1.5%, with overlapping ranges.
- `filter` and `filter_loop` are 10-17% faster. I did not take them apart; they are loops with an `if` that adds to a list, so the loop exit's order (cause 1) is the likely reason.
- The two `_direct` interface loops (0.34 and 0.58 ns against 0.53 and 0.89) are sub-nanosecond loops that moved by 0.3 ns with code layout before (the "Range analysis" section of `kestrel/bench/kernels/results.md`), so I don't count them as a gain.

**Compile time:** no measurable cost. Compiling the voxel world to object code (`kernels.exe main.kes --release --dump voxels3d.mesh`, 7 runs each in turns) takes 89 ms with both compilers. `kestrel build main.kes --release` (9 runs each, alternating which goes first) gave 225 ms old and 252 new as medians, but 170 and 167 as minimums, with ranges of 170-293 and 167-326: noise from the link and the busy machine.

## Cranelift 0.137 (2026-10-22)

Kestrel moved from Cranelift 0.116 (wasmtime 29) to 0.137 (wasmtime 50), which has the elaborator fix for cause 1 above (wasmtime PR #14383). The API changes were mechanical: `MemFlags` is now `MemFlagsData` at the builder, block arguments are `BlockArg`s, `FunctionBuilder::declare_var` makes the variable and `finalize` takes the target's frontend config, `stack_store` takes the pointer type, branch destinations take the exception tables, and the `*_imm` helpers became `*_imm_s` (the same code; they differ from `*_imm_u` only for `i128`).

**What each workaround does now** (`codegen/tests/licm.rs`):

- `loop_exits.rs` is no longer needed for hoisting: without it every hoisting test passes, including two new ones that 0.116 failed even with it, code after an `if` whose branch holds a `break` and code after an `if` whose branch holds a check. **It stays, for block layout.** Cranelift lays blocks out in reverse postorder, which puts a `brif`'s first target last, so `brif more, body, exit` puts the body after the exit and every iteration of a small loop takes two jumps instead of one. Without the swap, `iface_hp_direct` went from 0.55 to 0.88 ns per item and `iface_step_direct` from 0.45 to 0.84; with it they are back (table below).
- `phis.rs` is still needed: without it the inner-loop test fails (the outer loop's work stays in the inner loop).
- `arith.rs`'s plain value for checked arithmetic is still needed: 0.137's egraph still treats every instruction with two results (`sadd_overflow` and the others) as a side effect (`is_pure_for_egraph` in `inst_predicates.rs`).

**Cranelift dropped its tree-rebalancing rules.** 0.116 rewrote `((a ^ b) ^ c) ^ d` to `(a ^ b) ^ (c ^ d)` (and the same for `iadd`, `imul` and `band`); 0.137 has no such rule. For the frozen copy's hashes that rule happened to put the two invariant parts together (`z * C ^ seed * D`), which the old compiler then hoisted. With 0.137 the sample block of `voxels3d.mesh` has 108 xors where it had 84, one more per hash, and is 535 machine instructions where it was 504 (`--release`; the whole function is 2,311 against 2,449). The follow-up "reassociate xor/add chains" now has this to win back as well.

**World build, one thread** (whole runs, about 9-10 ms of startup included; each exe built by its compiler, all print 463,749; the old compiler is `main` at 0f44771; rounds take old and new in turn; medians with ranges; the machine was busy with the SIMD build on `main`):

| | Session 1 old | Session 1 new | Session 2 old | Session 2 new | New over old |
|---|---:|---:|---:|---:|---:|
| Frozen, checks on | 602 (532-661) | 603 (583-728) | 564 (494-904) | 567 (522-701) | 1.00, 1.01 |
| Frozen, `--release` | 525 (483-635) | 532 (507-560) | 472 (425-593) | 479 (454-600) | 1.01, 1.01 |
| Tuned, checks on | 463 (408-524) | **443** (392-459) | 410 (389-461) | **381** (357-459) | 0.96, 0.93 |
| Tuned, `--release` | 463 (442-554) | **432** (415-515) | 404 (386-474) | **394** (362-485) | 0.93, 0.98 |

Before `loop_exits` went back in, two sessions gave the frozen copy 1.04 and 1.02 checked and 1.07 and 1.03 `--release`, and the tuned copy 0.95-0.98.

- **The frozen copy is level** (1.00-1.01): the extra xor per hash costs about what the upgrade gains elsewhere.
- **The tuned copy is 2-7% faster.** Its hashes are already rebalanced by hand, so it loses nothing to the dropped rule.

**Kernels** (`py measure.py --rounds 5 --threads 1 --old DIR`, `--release`, ns per item, median of 5 rounds with ranges; old is `bench/kernels` built at 0f44771):

| Kernel | New | Old | New over old |
|---|---:|---:|---:|
| `step.kes` | 5.44 (5.29-5.52) | 5.31 (5.23-5.66) | 1.02 |
| `step_index.kes` | 7.20 (7.15-7.45) | 7.63 (7.34-7.83) | 0.94 |
| `step_scalar.kes` | 5.42 | 5.45 | 0.99 |
| `step_pow.kes` | 21.45 | 21.85 | 0.98 |
| `copy.kes` | 0.86 (0.83-0.89) | 0.79 (0.74-0.94) | 1.09 |
| `filter.kes` | **3.97** (3.91-4.16) | 4.86 (4.84-5.07) | **0.82** |
| `filter_loop.kes` | **3.98** (3.87-4.10) | 4.80 (4.70-4.99) | **0.83** |
| `map.kes` | 1.98 | 2.03 | 0.98 |
| `map_loop.kes` | 1.98 | 1.97 | 1.01 |
| `sort_by.kes` | 173.96 | 183.47 | 0.95 |
| `sort_by_loop.kes` | **104.43** (102.65-113.56) | 116.39 (111.17-120.90) | **0.90** |
| `iface_hp.kes` | 5.15 | 5.08 | 1.01 |
| `iface_hp_direct.kes` | 0.57 (0.52-0.65) | 0.59 (0.53-0.66) | 0.97 |
| `iface_step.kes` | 5.64 (5.27-6.06) | 5.24 (4.97-5.37) | 1.08 |
| `iface_step_direct.kes` | 0.57 (0.53-0.63) | 0.44 (0.42-0.46) | 1.30 |

- `filter`, `filter_loop` and `sort_by_loop` are 10-18% faster; an earlier run without `loop_exits` agreed (0.87-0.93).
- `iface_step_direct` is layout: its loop is the same instructions (0.137 even adds the immediate straight to memory), and with one unused function added to the file the new build is faster (0.40-0.46 ns against 0.57-0.66, three runs each).
- `iface_step` is 2-7% slower whatever the padding (three paddings, three runs each). The call site has one more register move before the indirect call (the new register allocator, regalloc2 0.15); not chased further.
- `copy.kes` (+9%) is a sub-nanosecond loop with overlapping ranges.

**Compile time:** no measurable change. `kestrel build main.kes --release`, 9 runs each in turns, took 2.49 s in all with the old compiler and 2.42 s with the new.

## Quiet rerun after the merge (2026-10-24, evening)

The Cranelift 0.137 upgrade was merged into `main` after the SIMD round (2dfe1e9). Measured on a quiet machine (the user closed every app): three compilers built in `git worktree`s at `C:\kt`, **pre-upgrade** ac885a3 (0.116), **pre-sort** 2dfe1e9 (0.137), **now** d9c2223 (plus `sort_by` key caching and the inliner's unused-argument skip). Each built the frozen `main.kes` and `voxels3d_tuned.kes`, checked and `--release`; all twelve exes print 463,749. 11 rounds, every exe once per round in turn (`C:\kt\vox\time_vox.ps1`); whole runs, medians with ranges; startup is 7 ms.

| | Pre-upgrade | Pre-sort | Now | Pre-sort over pre-upgrade |
|---|---:|---:|---:|---:|
| Frozen, checks on | 423 (420-434) | 439 (438-448) | 439 (437-465) | 1.04 |
| Frozen, `--release` | 373 (367-387) | 385 (381-402) | 382 (380-402) | 1.03 |
| Tuned, checks on | 325 (323-337) | **312** (308-329) | 313 (309-328) | **0.96** |
| Tuned, `--release` | 330 (325-345) | **312** (310-344) | 313 (310-334) | **0.95** |

- **The upgrade makes the frozen copy 3-4% slower and the tuned copy 4-5% faster.** The upgrade branch, on a busy machine, measured the frozen copy level (1.00-1.01) and the tuned 2-7% faster. The frozen copy's loss is the dropped xor rebalancing (one more xor per hash, see above); "reassociate xor/add chains" in the follow-ups would win it back.
- **Today's change is neutral here** (now against pre-sort, within 1%).
- **The checked builds cost nothing in the tuned copy** (313 against 313) and 15% in the frozen one (439 against 382).

**Against the straight Rust port** (`C:\kt\voxtuned\rust`, the same session, 11 rounds in turn): Rust release 344 ms (336-353), Rust with overflow checks 353 (351-366), Kestrel tuned `--release` 315 (310-319), Kestrel frozen `--release` 390 (381-421). **The tuned copy is 0.92 times Rust's time; the frozen copy 1.13 times** (it was 1.26-1.28 on 2026-10-20, before loop hoisting).
