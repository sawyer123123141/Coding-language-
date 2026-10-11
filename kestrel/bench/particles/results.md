# Particles demo: Kestrel against Rust

Measured on 2026-10-11, every run in a hidden window. The Kestrel demo (`demos/particles/particles.kes`, drawing with one `draw.rects` call) was ported to Rust (`rust/`) on the same vendored raylib, so the two can be compared honestly at 200,000 dots, and so the Rust port can be used to look for the cause of the slowdown that `demos/particles/results.md` could not explain.

**Short version.** With bulk drawing at 200,000 dots a Kestrel frame takes 5.58 ms against 5.22 ms for the Rust port that fills the vertices straight from the particles and 5.46 ms for the one that copies the positions first, as the demo does: Kestrel is 2-7% behind, which is about the size of this machine's noise, though the direct Rust frame was the shorter one in each of the four rounds. The draw call is the same Rust code in both, so the gap is only Kestrel's own loops: its update (0.75 ms against 0.57) and its copy loop (0.44 ms against 0.15-0.24). With one `draw.rect` per dot Kestrel is 14-16% behind (28.3-29.3 ms against 24.7). The slowdown of the vertex fill and the upload is **not** Kestrel's and **not** `par for`'s: Rust shows it with no update at all, and the cause looks like the NVIDIA driver's own upload thread (see "The slowdown").

## What was compared

- **Kestrel**: `kestrel run --release` (the JIT) and a `kestrel build --release` exe of the demo, as in `demos/particles/results.md`: a `par for` on 16 threads steps every particle, a loop copies the positions into a `list[vec2]`, one `draw.rects` call draws them, then the text and `window.frame()`. The measured copies set `target` to (640, 360) instead of the mouse, and `START_COUNT` to 50,000, 200,000 or 500,000. They live in the scratch folder, not in the repo.
- **Rust** (`rust/`, a small Cargo project outside the `kestrel/` workspace that depends on `kestrel-window` by path, so it uses the same vendored raylib build and the same `kestrel_window::raylib` declarations). It has the demo's constants, per-particle arithmetic, timers, window flags (1280×720, resizable, vsync hint, 4x MSAA, and **always hidden**), three text lines and once-a-second line. The start state comes from a fixed seed (splitmix64), in the demo's order: x, y, then vx, vy of each particle. There is no Space key. Three ways to draw:
  - **bulk, direct**: one dynamic mesh, 6 vertices per dot (`bulk.rs` here is a copy of the window crate's `bulk.rs` and `material.rs`: the same fill loop, the same flush of raylib's batch, culling off around `UpdateMeshBuffer` and `DrawMesh` with the default material), with the vertices filled straight from the particles. This is what a Rust programmer would write.
  - **bulk, with copy**: the same, but the positions are first copied into a `Vec<V2>` kept across frames, as the Kestrel demo does. The copy counts as draw, as it does in the demo.
  - **per dot**: one `DrawRectangleV` per particle, the demo's `BULK = false`.
- **The update** in Rust is rayon's `par_iter_mut` on its global pool (rayon 1.12.0, rayon-core 1.13.0; the crates could be fetched). There are other ways to run it for the experiments: a pool built like Kestrel's `par for` (`pool:N`: workers blocked on a channel, an atomic chunk counter, the caller works too), `std::thread::scope` over N chunks, one thread, or none.
- `pow(KEEP, dt)` is called **for every particle**, as the demo does. The optimiser might otherwise hoist the call out of the loop, so the Rust code hides the base from it with `black_box`. `--pow hoisted` calls it once a frame.

## What the numbers are

The same as in `demos/particles/results.md`. **update** is the time of the parallel step, **draw** is `draw.clear` plus the drawing (including the copy loop in the demo, and in the Rust copy variant), **frame** is the whole loop iteration (update, draw, the three text lines, and the end of the previous frame and start of this one), **fps** is frames divided by seconds. The Rust port adds columns for the parts of the draw: **copy**, **fill** (writing the vertices), **upload** (`UpdateMeshBuffer`) and the `DrawMesh` call (always under 0.01 ms, left out of the tables).

Each run is about 11 seconds, killed by `measure.py` after that (it kills the process it started). The first per-second line is dropped, which leaves 9 values a run. A cell is the **median of the per-second values with their range in brackets**. Runs are repeated in rounds that run every command once, one after the other (Kestrel JIT, Kestrel exe, then the Rust variants), so they share the machine's conditions; the values of all rounds are pooled (36 values for 4 rounds). Differences under about 10% are noise: the same program's per-round medians differ by 5-10% from one round to the next. Because the windows are hidden the frame rate is not capped, so fps is how fast the program could go with nothing to wait for; a visible window would show at most 60.

## Setup

- AMD Ryzen 7 5800X (8 cores, 16 threads), NVIDIA GeForce RTX 3070 (driver 591.86; its PCIe link is Gen 4 with 8 lanes while it works), Windows 11. `par for` used 16 threads, rayon 16.
- Release builds: `kestrel run --release`, a `kestrel build --release` exe, `cargo build --release` for the Rust port (rustc 1.97.0, default release profile, the same as the Kestrel crates).
- The machine is noisy (a desktop with other programs open). The batches below ran at different moments and their baselines differ by 10-20% (the same configuration measured 4.1 and 4.9 ms of draw in two batches), so **compare rows inside a table, not across tables**.

## Results at 200,000 dots

4 rounds, all five commands interleaved, 36 values each. Rust is "bulk direct", "bulk with copy" and "per dot" as above. The last four columns are the Rust parts of the draw (the demo has no equivalent for fill and upload; see the split below).

| Run | update ms | draw ms | frame ms | fps (hidden, not capped) |
|---|---:|---:|---:|---:|
| Kestrel JIT | 0.75 (0.73-0.81) | 4.79 (4.18-5.68) | 5.58 (4.96-6.55) | 179 (153-202) |
| Kestrel exe | 0.75 (0.73-0.83) | 4.77 (4.15-5.77) | 5.57 (4.94-6.66) | 179 (150-203) |
| Rust bulk, direct fill | 0.57 (0.56-0.65) | 4.61 (4.09-5.47) | 5.22 (4.70-6.19) | 192 (162-213) |
| Rust bulk, with copy | 0.57 (0.56-0.64) | 4.84 (4.24-5.60) | 5.46 (4.84-6.30) | 183 (159-207) |

| Rust parts of the draw | copy ms | fill ms | upload ms |
|---|---:|---:|---:|
| bulk, direct fill | - | 1.40 (0.73-2.23) | 3.23 (3.16-3.44) |
| bulk, with copy | 0.21 (0.14-0.45) | 1.62 (0.90-2.12) | 3.05 (2.97-3.18) |

Per round (median draw / frame ms), to show the direction is steady: Kestrel JIT 5.20/6.03, 4.63/5.42, 4.79/5.58, 4.33/5.12; Rust direct 4.81/5.46, 4.65/5.27, 4.43/5.04, 4.21/4.84; Rust with copy 5.13/5.78, 4.86/5.47, 4.70/5.33, 4.31/4.92. Rust direct had the shorter frame in all four rounds, by 0.15 to 0.57 ms against the JIT.

**One draw call per dot** (3 rounds interleaved, 27 values; this is the demo with `BULK = false`):

| Run | update ms | draw ms | frame ms | fps |
|---|---:|---:|---:|---:|
| Kestrel JIT, `draw.rect` per dot | 0.76 (0.75-0.77) | 29.28 (29.11-32.15) | 30.13 (29.96-32.99) | 33.2 (30.3-33.4) |
| Kestrel exe, `draw.rect` per dot | 0.76 (0.75-0.77) | 28.27 (28.20-28.48) | 29.13 (29.05-29.31) | 34.3 (34.1-34.4) |
| Rust, `DrawRectangleV` per dot | 0.59 (0.57-0.62) | 24.67 (24.45-24.75) | 25.34 (25.12-25.50) | 39.5 (39.2-39.8) |

(The 4-round run above, with 36 values, gave the Rust per-dot row as 0.59 / 24.79 / 25.48 / 39.2.)

**Other counts** (bulk drawing, 3 rounds interleaved, 27 values; Kestrel is the JIT):

| Particles | Run | update ms | draw ms | frame ms | fps |
|---:|---|---:|---:|---:|---:|
| 50,000 | Kestrel | 0.20 (0.20-0.20) | 0.97 (0.95-0.97) | 1.21 (1.19-1.22) | 829 (821-838) |
| 50,000 | Rust direct | 0.19 (0.19-0.21) | 0.93 (0.92-0.95) | 1.17 (1.16-1.22) | 853 (823-859) |
| 50,000 | Rust with copy | 0.20 (0.19-0.22) | 0.94 (0.93-0.98) | 1.18 (1.17-1.24) | 848 (805-856) |
| 500,000 | Kestrel | 1.73 (1.71-1.92) | 16.12 (15.59-19.49) | 17.92 (17.45-21.62) | 55.8 (46.2-57.3) |
| 500,000 | Rust direct | 1.29 (1.26-1.39) | 15.33 (14.95-17.98) | 16.66 (16.29-20.29) | 60.0 (49.3-61.4) |
| 500,000 | Rust with copy | 1.29 (1.26-1.37) | 16.22 (15.99-19.70) | 17.65 (17.41-21.47) | 56.7 (46.6-57.4) |

At 500,000 Rust's fill is 7.5 ms and its upload 7.7 ms (direct): the fill gets slow once the 36 MB vertex buffer no longer fits the 32 MB L3 cache, as the demo's results already found.

## Where each one's time goes

- **Update.** Rust's rayon update takes 0.57 ms and a pool built like Kestrel's 0.44, against Kestrel's 0.75 (all 16 threads). On one thread Kestrel's step takes 4.89 ms and Rust's 3.27 (1.5 times as long): that is the compute, not the pool. Most of that compute is the `pow` the demo calls for every particle: with it hoisted out (one call a frame) Rust's one-thread update takes 0.76 ms, as long as Kestrel's whole 16-thread update, and its rayon update 0.25 ms (pool 0.17). Hoisting it in a scratch copy of the Kestrel demo (`let keep = pow(KEEP, dt)` before the `par for`, passed into `step`) takes Kestrel's update from 0.80 to 0.47 ms with 16 threads, and from 5.41 to 2.95 ms with one (table below). Rust gains more, so after the hoist Kestrel's step costs 3.4 times Rust's on one thread (14.8 ns a particle against 4.3). I did not look at why; autovectorising is a candidate.

  **The update alone** (the update run 20 times a frame and the time divided by 20, so the cores stay busy and the frame's idle time is out of it; 3 rounds, 18 values; the draw and frame columns of these runs mean nothing and were not used, because hidden windows were being held to 60 fps at the time, see the end):

  | Update, ms | Kestrel, `pow` per particle (the demo) | Kestrel, `pow` hoisted | Rust, `pow` per particle | Rust, `pow` hoisted |
  |---|---:|---:|---:|---:|
  | 16 threads (`par for`; rayon in Rust) | 0.80 (0.75-0.90) | 0.47 (0.45-0.53) | 0.61 (0.52-0.86) | 0.27 (0.25-0.32) |
  | 16 threads, a pool built like Kestrel's (Rust) | - | - | 0.52 (0.47-0.54) | 0.23 (0.20-0.25) |
  | one thread | 5.41 (5.08-6.32) | 2.95 (2.67-4.73) | 3.52 (3.31-4.31) | 0.86 (0.79-0.95) |

  These agree with the per-frame numbers above within about 15% (0.75, 0.57, 0.44 and 3.27 there).
- **The copy loop.** Kestrel's loop that copies `p.pos` into the list takes 0.44 ms at 200,000 (2.2 ns a particle; measured with `window.ticks_usec()` in a scratch copy, JIT). Rust's equivalent takes 0.15-0.24 ms. That is 2 to 3 times as long; I did not look at why (bounds checks and no vectorising are the candidates), and it is only 0.2-0.3 ms of the frame. The direct variant has no copy at all, which is one of the two reasons its frame is shorter than the demo's.
- **The `draw.rects` call** is the same Rust code and the same raylib calls in both, so the two cost the same: 3.76 ms in the Kestrel copy that timed it, against fill 0.95 plus upload 3.16 (4.1 ms) in the Rust copy variant run next to it in the same round (frame 4.99 and 4.90 ms in that round). The time is fill and upload; `DrawMesh` is microseconds. The vertices are 72 bytes a particle, 14.4 MB at 200,000.
- **The rest of the frame** (the three text lines, `window.frame()`, the native-call boundary) is about 0.04 ms in both: frame minus update minus draw is 0.04 ms for Kestrel (5.58 - 0.75 - 4.79) and for Rust direct (5.22 - 0.57 - 4.61).
- **One draw call per dot.** Rust takes 123 ns a dot (24.7 ms for 200,000 `DrawRectangleV` calls: that is raylib's batch and the driver), Kestrel 141-146 ns. The extra 3.6-4.6 ms is the loop and the native-call boundary (the demo's results found about 10 ns a call with the raylib calls taken out), plus Kestrel's own loop being slower than Rust's.
- **500,000 dots** is the same story: Kestrel's frame is 7% longer (17.92 against 16.66 ms). At 50,000 it is 3% longer (1.21 against 1.17 ms).

## The slowdown

`demos/particles/results.md` found that the vertex fill took 2.6-3.8 times as long in the demo as in a standalone `rustc -O` copy of the same loop (0.35 ms at 200,000), that the upload was also slower, and that both came back near standalone with `KESTREL_THREADS=1`. It suspected the `par for` pool. **That is not the cause.** The Rust port shows the same thing, with no pool and no update at all.

### What I tried

Rust, 200,000 dots, bulk direct, draw in ms (fill + upload = most of it). Each table is one interleaved batch (3 rounds, 21 values per row), so compare rows inside a table only.

**1. Which update runs before the draw.**

| Update | update ms | draw ms | fill ms | upload ms | frame ms |
|---|---:|---:|---:|---:|---:|
| none | 0 | 4.40 | 0.61 | 3.75 | 4.53 |
| one thread (the main one) | 3.25 | 2.34 | 0.56 | 1.75 | 5.63 |
| rayon, 1 thread (on a worker) | 3.92 | 2.17 | 0.40 | 1.75 | 6.14 |
| rayon, 4 threads | 1.05 | 3.77 | 0.77 | 2.99 | 4.89 |
| rayon, 16 threads | 0.56 | 4.10 | 0.76 | 3.34 | 4.73 |
| pool like Kestrel's, 16 threads | 0.44 | 4.14 | 0.69 | 3.45 | 4.62 |
| (no GL calls: a `memcpy` of the vertices instead) none | 0 | 1.13 | 0.35 | 0.78 | 1.17 |
| (no GL calls) one thread | 3.22 | 1.16 | 0.37 | 0.80 | 4.42 |
| (no GL calls) rayon, 16 threads | 0.66 | 1.31 | 0.51 | 0.80 | 2.03 |

- With no update at all the upload is already 3.75 ms. The draw is as slow as after a 16-thread update, or slower. So the parallel update is not what slows it.
- Take the GL calls out (`UpdateMeshBuffer` and `DrawMesh` replaced by a `memcpy` into another `Vec`) and the fill is 0.35-0.37 ms after no update or a one-thread update, the standalone number. After the 16-thread update it is 0.51: reading particles that other cores wrote costs about 0.15 ms. The rest of the slowdown needs the GL calls.
- The draw gets slower the less time the main thread spent on something else before it. Update plus draw stays at about 4.4-4.8 ms while the update is under 1 ms, and is 5.6-6.1 ms when the update takes over 3 ms. The frame cannot go below about 4.5 ms.

**2. A wait before the draw.** (The wait is after the update and before `draw_start`, so it is in the frame but in neither column. 16-thread rayon update.)

| Wait before the draw | draw ms | fill ms | upload ms | frame ms |
|---|---:|---:|---:|---:|
| none | 4.12 | 0.79 | 3.32 | 4.76 |
| spin 3 ms | 2.21 | 0.44 | 1.76 | 5.85 |
| sleep 3 ms (a few seconds at the end of the last run were capped, see the end; the three runs' medians were 2.65, 2.65 and 2.67) | 2.65 | 0.55 | 2.09 | 6.57 |
| `glFinish` at the end of every frame (GPU idle at the start of the next) | 2.23 | 0.46 | 1.77 | 6.39 |
| no update, `glFinish` at the end of every frame | 2.29 | 0.52 | 1.77 | 5.90 |

- A wait of 3 ms or more brings the upload to 1.75-1.8 ms and the fill to 0.4-0.55 ms, however it is spent. A sleep and a spin behave alike, so it isn't the CPU clock or idle states.
- With no update at all, waits of 0, 0.5, 1, 2, 3 and 5 ms gave draw times of 5.03, 4.67, 4.83, 3.42, 2.79 and 2.34 ms (an earlier batch of 2 rounds, 10 values).
- `glFinish` at the end of each frame gives the same draw whether the update ran on 16 threads or on none.
- **The same in Kestrel**, JIT, 2 rounds of 7 values (the draw column includes the copy loop; with the wait the frame includes 3 ms of spinning, written in the Kestrel program with `window.time()`):

| Kestrel | update ms | draw ms | frame ms |
|---|---:|---:|---:|
| `KESTREL_THREADS=16` | 0.74 | 4.19 | 4.97 |
| `KESTREL_THREADS=8` | 0.86 | 4.13 | 5.04 |
| `KESTREL_THREADS=4` | 1.40 | 3.76 | 5.27 |
| `KESTREL_THREADS=2` | 2.62 | 3.00 | 5.67 |
| `KESTREL_THREADS=1` | 4.89 | 2.54 | 7.47 |
| 16 threads, then a 3 ms spin | 0.77 | 2.55 | 6.37 |
| 1 thread, then a 3 ms spin | 4.87 | 2.58 | 10.50 |

  With 16 threads and a 3 ms spin Kestrel's draw is 2.55 ms, the same as with one thread. It is the gap before the draw, not the thread count, that sets the draw time. And the frame is still shortest with 16 threads (4.97 ms against 7.47 for one).

**3. Where the main thread runs.** (Rust, 16-thread rayon update, 2 rounds of 5 values. The mask is the thread's affinity mask, set before the window opens; bit 0 is logical CPU 0.)

| Main thread's affinity | draw ms | fill ms | upload ms | frame ms |
|---|---:|---:|---:|---:|
| all CPUs (not set) | 4.94 | 1.42 | 3.54 | 5.59 |
| all 16 set explicitly (`FFFF`) | 4.71 | 1.52 | 3.20 | 5.37 |
| CPU 2 only (`4`) | 2.02 | 0.43 | 1.56 | 2.75 |
| CPU 0 only (`1`) | 2.26 | 0.51 | 1.73 | 3.03 |
| CPU 4 only (`10`) | 2.02 | 0.42 | 1.56 | 2.75 |
| CPU 8 only (`100`) | 2.21 | 0.52 | 1.66 | 2.95 |
| CPUs 0 and 1, one core's two threads (`3`) | 2.22 | 0.51 | 1.67 | 2.95 |
| CPUs 0 and 2, two cores (`5`) | 4.72 | 1.50 | 3.24 | 5.36 |
| CPUs 0-3, two cores (`F`) | 4.86 | 1.75 | 3.13 | 5.48 |
| every other CPU, 8 cores (`5555`) | 4.56 | 1.33 | 3.23 | 5.18 |

- Pinning the main thread to one CPU (or to one core's two hardware threads) **more than halves the frame**: 5.59 to 2.75 ms in this batch (in a later batch, whose unpinned baseline was 4.7 ms, the pinned frame was 2.5 ms with a draw of 1.8). Two cores do not help, an explicit full mask does not help, and pinning after the window was open did not help (draw 4.35 ms). So the effect depends on the affinity when the window's graphics context is made, and not on thread migration: the thread switched CPU about once a frame in the slow runs, but not in the `no update` runs, which are slow too (a counter in the Rust port's `moves` column).
- Process affinity on the Kestrel exe (`start /affinity 4`, so all its threads share one CPU) gave the same drop in Kestrel: draw 2.4 ms (the update was 5.0 ms, the frame 7.5) against 5.4 ms with all CPUs.

**4. What else runs in the process.** Per-thread CPU time over 4 seconds (`Get-Process`, Rust with no update, 200,000 dots):

| Run | main thread | the next busiest thread |
|---|---:|---:|
| unpinned | 4,016 ms | 3,000 ms |
| main thread pinned to CPU 2 | 4,000 ms | 0 ms |
| unpinned, `--upload skip` (no upload, no `DrawMesh`) | 4,000 ms | 1,203 ms |
| unpinned, `--upload gl-drawonly` | 3,984 ms | 1,500 ms |
| unpinned, `--upload gl-nodraw` (upload, no draw) | 3,953 ms | 3,875 ms |
| unpinned, upload and draw | 3,953 ms | 3,656 ms |
| Kestrel exe, unpinned (the 15 `par for` threads used about 400 ms each) | 4,016 ms | 2,969 ms |

A second thread that the program does not start is busy 74-97% of the time while each frame uploads (30-37% when a frame has no upload), in Kestrel and in Rust alike. The Rust run with no update has no pool; raylib and GLFW start no threads on Windows. So it is a thread of the graphics driver. With the main thread pinned to one CPU that thread is gone.

### What it is

My best explanation: **the NVIDIA OpenGL driver does an upload on a thread of its own.** `UpdateMeshBuffer` returns once the main thread has copied the 14.4 MB (1.6-1.8 ms, about 8-9 GB/s when nothing else is going on); the driver's thread then takes about 3.5-5 ms more to handle it (`glFinish` after the upload waits that long, which is 3-4 GB/s, and not the PCIe link, which showed Gen 4 x8 while busy, about 15 GB/s). If the next frame's upload comes while the driver's thread is still busy, the call waits for it (3.3-3.8 ms instead of 1.8), and the fill, which runs just before the upload, is slowed in some runs from 0.5 to 1.4 ms (memory and cache traffic of the driver's copies, I suppose; its size varied from run to run and batch to batch, 0.6 to 1.8 ms, and I do not know what sets it). A faster update only moves the next upload closer. It exposes the wait, it does not cause it. A one-thread update (3-5 ms), a sleep or a spin all give the driver time to finish. With the main thread pinned to one CPU the driver does not use its thread and does the work on the main thread, which takes 1.6 ms for the upload and gets the whole frame to 2.5-2.8 ms.

- **Confident (about 95%) that it is not `par for`**, nor Kestrel's pool, rayon, cache coherence between the cores that wrote the particles (0.15 ms of it at most), clock speed after an all-core burst, the vector's pages, or thread migration: the same slowdown happens with no update at all, it is absent without GL calls, a sleep and a spin cure it alike, and the thread count enters only through the time the update takes.
- **Fairly confident (about 75%) that it is the driver's upload thread.** The evidence is circumstantial: a second busy thread that is ours in neither program, the way it scales with the upload, the `glFinish` wait, and its disappearance when the main thread is pinned. I have not looked inside the driver, and I cannot change the driver's "threaded optimization" setting from here, which would test it directly.
- **Not known**: why the fill is slowed (and by how much, which varies), and which stage of the driver's work takes the 3.5-5 ms.

### What it means for Kestrel (nothing was changed in `kestrel/window` or `kestrel/runtime`)

- The slowdown costs Kestrel nothing it could take back by changing `par for`. The 16-thread update is the right choice: the frame is 4.97 ms with it and 7.47 ms with one thread.
- Only 2.3 ms of the 200,000-dot draw (fill 0.5, upload 1.8) is work the main thread does; the rest is waiting for the driver. So **sending fewer bytes** is the real fix, and it would cut both: 4 vertices with shared indices (48 bytes a dot), or a vertex shader that expands one position per dot (8 bytes: 14.4 MB becomes 1.6 MB), as the demo's results suggested.
- **Pinning the window thread** to one CPU before the graphics context is made gave a 2x faster frame in the Rust port (5.6 to 2.75 ms) on this NVIDIA machine. It is a hack that depends on this driver and may cost a program that wants the main thread to move; I have not tried it in the window crate.
- A visible window is capped at 60 fps (16.67 ms) by vsync, so at 200,000 dots none of this changes the frame rate; it changes how much of the frame is spare.

## Summary: how far Kestrel is from Rust here, and why

- **Bulk drawing at 200,000 dots**: Kestrel's frame is 2-7% longer than Rust's (5.58 ms against 5.46 with the same copy and 5.22 without it). That is within this machine's noise, but steady in direction. Nearly all of the frame is `draw.rects`, which is the same Rust code, so both programs are about equally fast there. What differs is Kestrel's own code: the update takes 0.75 ms against 0.57, and the copy loop 0.44 ms against 0.15-0.24, together about 0.4 ms of 5.5. Kestrel's loops run 1.3 to 3.4 times as long as the same Rust loops (the update 1.3 to 1.7 times as long as Rust's, the copy loop 2 to 3 times), but those loops are small here.
- **One `draw.rect` per dot**: Kestrel is 14-16% behind (28.3-29.3 ms against 24.7). Most of the time is raylib's, and the native-call boundary and the loop are the difference.
- **The update's compute** is where Kestrel is furthest behind: a one-thread step takes 1.5 times as long as Rust's with the same `pow` per particle, and 3.4 times as long with both programs' `pow` hoisted. Hoisting the `pow` is a change to the program, not the language, and it helps Kestrel less than Rust: the 16-thread update goes from 0.80 to 0.47 ms in Kestrel and from 0.61 to 0.27 in Rust, and the one-thread update from 5.41 to 2.95 and from 3.52 to 0.86. So after the hoist Kestrel's step costs about 3.4 times Rust's on one thread. That is the largest gap between the two here, and it is 0.2 ms of a 5 ms frame at 200,000 dots.
- **At 500,000 and 50,000 dots** it is the same picture: 7% and 3% behind.
- **The slowdown the demo's results could not explain is not Kestrel's** (see above): it is the cost of a 14.4 MB upload per frame on this driver, and what Kestrel can do about it is send less.
- **Caveat.** This is one machine with one GPU and driver. The Rust port copies the window crate's draw path, so "the draw call is the same code" is true by construction, and it is the reason Rust has so little room to be faster there.

## Reproducing it

From `kestrel/` (Windows, PowerShell or Git Bash; `my_copy.kes` is `demos/particles/particles.kes` with `START_COUNT` set and `let target = input.mouse()` replaced by `let target = vec2(640, 360)`):

```
cargo build --release -p kestrel-cli
cd bench/particles/rust && cargo build --release && cd ../../..
python bench/particles/measure.py --rounds 4 \
  "kestrel-jit=KESTREL_HIDDEN=1 target/release/kestrel.exe run --release my_copy.kes" \
  "rust-direct=bench/particles/rust/target/release/particles-rust --count 200000 --draw direct" \
  "rust-copy=bench/particles/rust/target/release/particles-rust --count 200000 --draw copy" \
  "rust-dots=bench/particles/rust/target/release/particles-rust --count 200000 --draw dots"
```

`measure.py` prints the medians and ranges and each run's median draw and frame, and kills each process it starts after 11 seconds. Every Kestrel command needs `KESTREL_HIDDEN=1`; the Rust port is always hidden. The switches of the Rust port (update modes, `--upload`, `--gap`, `--finish`, `--pin-main`) are listed at the top of `rust/src/main.rs`.

A caution about hidden windows: they are normally not held to the display's refresh rate on this machine (the first measurements that day ran at 5 ms frames, and 10,000 dots at 4,300 fps), but from about 13:00 on 2026-10-11, in the middle of the experiments, they were capped at 60 fps (a frame of 16.67 ms) for both Kestrel and the Rust port, whatever the particle count, and stayed so for the next 40 minutes with one short gap. It began while the machine was being used (input within the last seconds, the Claude desktop app in front), and the GPU was doing other work at 10-35%. All of the windows of the process report as not visible, so nothing was shown. I do not know what caused it; the NVIDIA driver's handling of vsync for a window with no output is my guess. Every table above is from before it began, except the update-alone table, whose column does not depend on it, and the `moves` and per-thread checks, which do not depend on the frame rate. `measure.py` prints a warning for a run with a 16.67 ms frame, and `--retry-capped N` repeats such runs; a run that shows one should be thrown away.
