# Particles demo: measured times

Measured on 2026-10-10 (one `draw.rect` call per particle) and 2026-10-11 (one `draw.rects` call for all of them). The demo is `particles.kes`: a `par for` moves the particles, then they are drawn. With `BULK = true` (the committed setting) the positions are copied into a list and one `draw.rects` call draws them; with `BULK = false` it makes one `draw.rect` call per particle. The one-call-per-particle numbers below are the baseline. They show that drawing, not Kestrel's `par for`, was the bottleneck. The "Bulk drawing" section shows what `draw.rects` changed: at 200,000 particles drawing takes about 6 times less time, and a frame of 5.4 ms fits well inside the 16.67 ms of 60 fps (judged from the frame time, as every new run used a hidden window).

## What the numbers are

- **update** is the time of the `par for` that steps every particle, in ms per frame.
- **draw** is the time of `draw.clear` plus drawing the particles, in ms per frame. It is the time the program spends in those calls: one `draw.rect` call per particle, or with `BULK` the loop that copies the positions into the list plus the one `draw.rects` call. The three text lines of the overlay are not in it.
- **frame** is the time of the whole loop iteration: update, draw, the text, and `window.frame()` (which shows the frame, waits for vsync and reads input; a hidden window doesn't wait for vsync, see below).
- **fps** is frames divided by seconds.

Each cell is the **median of the per-second averages** the demo prints (it prints one line a second), over a run of about 11 seconds. The first second is dropped because it includes start-up, which leaves 9 or 10 values per run. The range of those averages is in brackets. These are not medians of single frames: the demo only sums each second, and a frame that stalls shows up in that second's average.

## Setup

- AMD Ryzen 7 5800X (8 cores, 16 threads), NVIDIA GeForce RTX 3070, Windows 11, a 60 Hz display. `par for` used all 16 threads.
- A 1280×720 window, vsync on (`window.open` always turns it on).
- Both `kestrel run --release` (the JIT) and a `kestrel build --release` exe, so the overflow and narrowing checks are off.
- The starting count is `START_COUNT` in the demo, and `BULK` picks the drawing, both changed per run. The mouse is not used: the measured copies set `target` to the window centre (640, 360), so every run pulls toward the same point.
- The 2026-10-10 runs (the baseline table) mostly used a visible window. The hidden runs of that day set `KESTREL_HIDDEN=1`; their numbers are in the text under the baseline table, not in it. **Every run of 2026-10-11 used a hidden window**, so the numbers in "Bulk drawing" are all hidden.
- The machine is noisy (a desktop with other programs open), so read differences under about 10% as noise.

## Results: one `draw.rect` per particle (the baseline, 2026-10-10)

| Particles | Run | update ms | draw ms | frame ms | fps |
|---:|---|---:|---:|---:|---:|
| 10,000 | JIT | 0.19 (0.19-0.21) | 1.95 (1.90-2.19) | 16.67 | 60 |
| 10,000 | exe | 0.20 (0.18-0.22) | 1.90 (1.81-1.98) | 16.67 | 60 |
| 50,000 | JIT | 0.41 (0.40-0.47) | 8.68 (7.98-10.10) | 16.66 | 60 |
| 50,000 | exe | 0.47 (0.44-0.56) | 8.75 (8.60-9.09) | 16.67 | 60 |
| 100,000 | JIT | 0.53 (0.52-0.56) | 15.89 (15.86-15.96) | 16.67 | 60 |
| 100,000 | exe | 0.50 (0.50-0.52) | 15.95 (15.88-15.98) | 16.65 | 60 |
| 200,000 | JIT | 0.95 (0.86-1.06) | 31.65 (29.92-33.38) | 32.80 (30.98-34.67) | 30.5 (28.9-32.3) |
| 200,000 | exe | 0.89 (0.86-0.97) | 29.55 (29.23-32.04) | 30.61 (30.25-33.36) | 32.7 (30.0-33.1) |

The 100,000 rows are an extra, to show where the 60 fps ends. The frame and fps ranges are left out where the frame is pinned at 16.67 ms: every median is within 0.1 fps of 60, and the per-second averages run from 59.7 to 60.9 fps.

Hidden windows gave the same numbers at 200,000 (update 0.91, draw 30.2 and 29.9, fps 31.9 and 32.3, JIT and exe). I wrote here on 2026-10-10 that a hidden window still waits for vsync, so that `KESTREL_HIDDEN=1` doesn't uncap the frame rate. **That was wrong.** At 200,000 the frame is 30 ms either way, so those numbers can't tell, but the hidden runs of 2026-10-11 show that a hidden window is not held to 60 fps: 10,000 particles ran at about 4,300 fps. So the 60 fps rows above (10,000 to 100,000) are from visible windows, and a hidden run's frame time is how long the program works on a frame, with no waiting for the display.

### Where the time goes (one `draw.rect` per particle)

- **vsync caps the frame at 16.67 ms (60 fps)** at 10,000, 50,000 and 100,000 particles. Below 100,000 the program is waiting for the display: at 10,000 the update and draw together use about 2 ms of the 16.67, at 50,000 about 9 ms, and at 100,000 about 16.4 ms, which is just inside the frame. So 60 fps holds up to about 100,000 particles on this machine, and no further: at 200,000 the frame is 30-33 ms (30-33 fps).
- **Drawing is the bottleneck, not the update.** At 200,000 particles the `par for` takes under 1 ms and the draw loop about 30 ms, so the update is 3% of the frame's work. Draw time grows with the count at about 150-190 ns per `draw.rect` call (1.9 ms for 10,000, 8.7 for 50,000, 15.9 for 100,000, 30 for 200,000).
- **That per-call cost is not Kestrel's.** I ran the same demo headless (`KESTREL_HEADLESS` with `KESTREL_HEADLESS_QUIET=1`), where each native call does its checks and reads its arguments but calls no raylib function. At 200,000 particles a whole frame took 2.9 ms (JIT and exe alike), measured as the slope of the run time between a 30-frame and a 330-frame run. If the update is the 0.9 ms measured above (I didn't time it headless), the Kestrel loop and the native-call boundary cost about 2 ms for 200,000 calls, about 10 ns each. The other 28 ms of the 30 ms draw time is spent inside raylib's rectangle path and the graphics driver (raylib batches the vertices and sends them to OpenGL when a batch fills). I did not profile inside raylib, so I can't say how that splits between raylib and the driver.
- **`par for` helps, but it isn't what sets the frame rate here.** With one thread (`KESTREL_THREADS=1`) the 200,000-particle update takes 5.4 ms (5.0-5.9), against about 0.9 ms with 16 threads: 5.9 times faster on 8 cores and 16 threads. That is not 16 times. I did not investigate why. Candidates: the machine has 8 cores, so the other 8 threads are SMT siblings that share a core's execution units; each frame forks and joins the pool; and each step does real work per particle (about 27 ns on one thread: a `pow(KEEP, dt)` call to the runtime that gives the same value for every particle in a frame, and a `normalized`, which is a square root and a divide), which may contend for shared resources. Hoisting the `pow` out of the step would cut the per-particle cost; I didn't try it, so the update numbers here are for the demo as written. Over a whole frame the difference is 35.4 ms against 31.3 ms (28.3 fps against 31.9 fps; both are hidden JIT runs).
- **JIT and exe are the same speed** within the noise, as expected: both run the same machine code.

## Bulk drawing (`draw.rects`, 2026-10-11)

**The change.** After the `par for`, a loop copies each particle's position into a `list[vec2]` that lives across frames, and one `draw.rects(positions, DOT_SIZE, DOT)` draws them all. A `list` has `add` but no `clear`, so the list is topped up with `add` to the particle count (once at the start, and again after Space adds more), and every frame the loop overwrites its entries by index. The copy sits between `draw_start` and `draw_end`, so it is counted in **draw**.

**How it was measured.** The same way as the baseline (medians of the per-second averages, first second dropped, 10 values per run, ranges in brackets), with the same machine and copies of the demo with `target` fixed at (640, 360). **Every run was a hidden window** (`KESTREL_HIDDEN=1`), because the user must not see one. The baseline's `BULK = false` at 200,000 was run again, hidden, as today's comparison. A hidden window is not held to 60 fps (see above), so **frame** and **fps** here are how fast the program could go with no display to wait for. A visible window would show at most 60 fps.

| Particles | Drawing | Run | update ms | draw ms | frame ms | fps (hidden, not capped) |
|---:|---|---|---:|---:|---:|---:|
| 10,000 | `draw.rects` | JIT | 0.07 (0.07-0.07) | 0.09 (0.09-0.10) | 0.23 (0.23-0.24) | 4,300 (4,145-4,348) |
| 10,000 | `draw.rects` | exe | 0.07 (0.07-0.07) | 0.09 (0.09-0.10) | 0.23 (0.23-0.24) | 4,261 (4,159-4,387) |
| 50,000 | `draw.rects` | JIT | 0.21 (0.20-0.21) | 0.97 (0.95-1.00) | 1.21 (1.20-1.25) | 824 (802-830) |
| 50,000 | `draw.rects` | exe | 0.21 (0.20-0.21) | 0.97 (0.96-0.98) | 1.22 (1.21-1.24) | 821 (804-826) |
| 100,000 | `draw.rects` | JIT | 0.40 (0.40-0.41) | 2.01 (2.00-2.09) | 2.46 (2.45-2.54) | 407 (394-408) |
| 100,000 | `draw.rects` | exe | 0.40 (0.40-0.40) | 2.08 (2.07-2.15) | 2.51 (2.51-2.60) | 397 (385-399) |
| 200,000 | `draw.rects` | JIT | 0.77 (0.76-0.79) | 4.62 (4.55-5.22) | 5.44 (5.37-6.07) | 184 (165-186) |
| 200,000 | `draw.rects` | exe | 0.77 (0.77-0.79) | 4.59 (4.55-5.22) | 5.42 (5.37-6.08) | 185 (165-186) |
| 500,000 | `draw.rects` | JIT | 1.81 (1.79-1.89) | 16.95 (16.71-21.09) | 18.84 (18.61-23.01) | 53 (44-54) |
| 500,000 | `draw.rects` | exe | 1.80 (1.72-1.91) | 17.25 (15.71-20.95) | 19.12 (17.51-22.84) | 52 (44-57) |
| 1,000,000 | `draw.rects` | JIT | 3.31 (3.28-3.44) | 38.20 (36.58-43.73) | 41.72 (39.96-47.15) | 24.0 (21.2-25.0) |
| 1,000,000 | `draw.rects` | exe | 3.31 (3.29-3.40) | 37.30 (36.42-43.38) | 40.74 (39.80-47.26) | 24.5 (21.2-25.1) |
| 200,000 | `draw.rect` per particle | JIT | 0.75 (0.74-0.76) | 28.12 (28.07-28.16) | 28.95 (28.90-28.99) | 34.5 (34.5-34.6) |
| 200,000 | `draw.rect` per particle | exe | 0.76 (0.74-0.76) | 28.66 (28.61-28.79) | 29.50 (29.44-29.64) | 33.9 (33.7-34.0) |

The last two rows are the baseline run again today. They are within about 11% of the 2026-10-10 rows (draw 31.65 and 29.55 visible; 30.2 and 29.9 hidden). The 11% is the JIT row's 28.12 against 31.65, a little over the 10% I called noise; the other three comparisons are within 7%. Nothing here depends on the difference.

At 500,000 and 1,000,000 the first few seconds are slower than the rest (for example 500,000 JIT: draw 21.7 and 21.1, then 16.7-17.9; 1,000,000 JIT: 43.6-43.9 for four seconds, then 36.6-39.6), which is why the ranges are wide. I don't know why; the medians are over all the values.

### Does 200,000 reach 60 fps?

**Yes, with room to spare, but I did not see it.** At 200,000 particles one frame takes 5.4 ms of work (update 0.77, draw 4.6, and under 0.1 ms for the text and `window.frame()`), against the 16.67 ms a 60 fps frame has: about 3 times the room. With vsync on, a visible window would then wait for the display and show 60 fps. That is an inference from the frame time. I did not run a visible window (the user must not see one), so I can't give a visible fps. It is the same inference that held for 10,000 to 100,000 in the baseline, where the visible runs did show 60.

- **200,000 drawn with `draw.rects`: 4.6 ms, against 28.1 and 28.7 ms with one `draw.rect` per particle** (hidden JIT and exe, same day): about 6 times less draw time, and a frame of 5.4 ms against 29-29.5 (about 5.4 times shorter). The 2026-10-10 baseline had 200,000 at 30-33 fps.
- **500,000 misses 60 fps.** The frame is 18.8-19.1 ms (52-53 fps uncapped), over the 16.67 ms. A straight line between my 200,000 and 500,000 runs crosses 16.67 ms at about 440,000, but I did not measure anything between them.
- **1,000,000 misses by a lot:** 41 ms a frame, 24 fps.
- Draw time per particle is 9 ns at 10,000, 19 ns at 50,000, 20 ns at 100,000, 23 ns at 200,000, 34 ns at 500,000 and 38 ns at 1,000,000, against about 140 ns with `draw.rect` at 200,000. So drawing a particle is about 6 times cheaper at 200,000 and still about 4 times cheaper at 1,000,000 (against the baseline's 150 ns), but it gets dearer per particle as the count grows.

### Where the time goes now

The draw time was split with `window.ticks_usec()` around the copy loop and the `draw.rects` call in a scratch copy of the demo, and the `draw.rects` call was split further with a scratch copy of the window crate (outside the repo, never committed) that timed `fill_into`, `UpdateMeshBuffer` and `DrawMesh` with `Instant` and printed their averages over 60 calls. All of these are separate hidden JIT runs of about 12 seconds (medians of the per-second or per-60-calls averages, first dropped), so their draw totals differ a little from the table above. The 1,000,000 row of the second split has only 3 values.

| Particles | draw ms | copy loop | `draw.rects` call | vertex fill | upload to the GPU buffer | `DrawMesh` call |
|---:|---:|---:|---:|---:|---:|---:|
| 10,000 | 0.09 | 0.02 | 0.07 | 0.02 | 0.05 | under 0.01 |
| 50,000 | 0.96 | 0.10 | 0.86 | 0.09 | 0.77 | under 0.01 |
| 100,000 | 1.96 | 0.19 | 1.77 | 0.18 | 1.58 | under 0.01 |
| 200,000 | 4.22 | 0.45 | 3.78 | 0.92 (0.82-2.12) | 2.84 (2.74-3.40) | under 0.01 |
| 500,000 | 15.72 | 1.98 | 13.77 | 7.56 (7.47-8.63) | 6.17 (6.00-8.78) | under 0.01 |
| 1,000,000 | 38.25 | 4.47 | 32.98 | 20.80 (19.37-21.36) | 11.32 (11.13-16.90) | 0.01 |

- **The `draw.rects` call is about 90% of the draw time from 50,000 particles up, and nearly all of it is writing the vertices and copying them to the GPU.** The `DrawMesh` call that issues the draw takes microseconds. The vertices are 72 bytes a particle (two triangles of three `x, y, z` floats): 14.4 MB at 200,000. `fill_into` writes them into a buffer on the CPU, then `UpdateMeshBuffer` (a `glBufferSubData`) copies them to the GPU buffer. At 200,000 that is 2.8 ms for the copy (about 5 GB/s) and about 0.9 ms for the fill. The copy is the biggest piece up to 200,000; at 500,000 and 1,000,000 the fill is.
- **The copy loop in Kestrel is about 10% of the draw time** (2.2 ns a particle: 0.45 ms of the 4.2 at 200,000). It reads each particle's position and writes it into the list by index.
- **`window.frame()` doesn't wait for the GPU in these runs**: frame minus update minus draw is under 0.1 ms at 200,000 (0.05) and 0.2 ms at 1,000,000. That only rules out a wait in `window.frame()`: a wait inside `UpdateMeshBuffer` (a `glBufferSubData` into the buffer the previous frame drew from) would be counted as draw time. Nor does it say how long the GPU takes to draw 400,000 tiny triangles, because the hidden window never shows its frames.
- **The update is now the smaller part but not nothing:** 0.77 ms of the 5.4 ms frame at 200,000 (14%), and 3.3 of 41.7 ms at 1,000,000 (8%). Drawing is still 85-90% of the frame.

### Concern: the vertex fill and the upload cost more in the demo than they should

`fill_into` in `window/src/bulk.rs` is a plain loop. I compiled a copy of it by itself (in the scratch folder, `rustc -O`, 100 repetitions each) and it took 1.7 ns a particle up to 200,000 (0.35 ms at 200,000), 4.0 ns at 500,000 and 6.7 ns at 1,000,000 (2.0 and 6.7 ms): fast while the buffer fits the CPU's cache, slower once it doesn't. In the demo the same function takes 0.9 ms at 200,000 (0.8-2.1), 7.6 ms at 500,000 and 20.8 ms at 1,000,000: 2.6 times the standalone time at 200,000, 3.8 times at 500,000 and 3.1 times at 1,000,000. The upload is also faster with fewer threads. The difference tracks the size of the `par for` pool (`KESTREL_THREADS`):

| 200,000 particles, `KESTREL_THREADS` | update ms | `draw.rects` call ms | frame ms |
|---:|---:|---:|---:|
| 1 | 4.87 | 2.1 | 7.4 |
| 4 | 1.39 | 3.3 | 5.2 |
| 16 | 0.74 | 3.8 | 5.0 |

With one thread the instrumented fill takes 0.37-0.53 ms at 200,000 (like the standalone loop) and the upload 1.75 ms, against 0.9 and 2.8 ms with 16. At 500,000 with one thread the fill is 2.6-2.9 ms and the upload 4.8-4.9 ms, against 7.6 and 6.2. The pool's threads block while they wait for work (`runtime/src/par.rs` uses a channel, not spinning), so I don't know why the main thread's memory work is slower after a 16-thread `par for`. I did not investigate (I didn't try pinning, a different thread count for the pool, or timing a cold run of the loop). It is still a win to use 16 threads: the whole frame is the shortest there. But it means up to 1.7 ms of the 4.2 ms of draw time at 200,000 (the `draw.rects` call takes 2.1 ms with one thread and 3.8 with 16) may be a cost of the pool that could be taken back, and I haven't shown that.

I didn't change the window crate. Both the fill and the upload are bounded by the 72 bytes a particle. Writing four vertices and a shared index buffer would be 48 bytes, and a shader that builds each rectangle from one position would be 8 bytes, so the upload at 200,000 would drop from 14.4 MB to 1.6 MB. Neither is tried.

## What would make it faster

Before `draw.rects`, the way out was fewer draw calls. Now the cost is the vertex data: 72 bytes a particle, written once on the CPU and copied once to the GPU, which together are about 90% of the draw time at 200,000. Sending less data per particle (see the concern above) would help most. The Kestrel side is now 0.45 ms at 200,000 for the copy loop, plus 0.77 for the update; making either cheaper saves at most about 1.2 of the 5.4 ms.

## Reproducing it

From `kestrel/`, with `START_COUNT` and `BULK` edited in the demo:

```
cargo build --release
KESTREL_HIDDEN=1 target/release/kestrel run --release demos/particles/particles.kes
```

`KESTREL_HIDDEN=1` keeps the window from showing at all, and the frame rate is then not capped at 60 fps; leave it off to see the window (capped at the display's rate). The demo prints a line once a second, for example `particles 200000 update 0.77 ms draw 4.59 ms frame 5.41 ms fps 184.81` (the bulk drawing at 200,000, hidden). Let it run for ten seconds or so, ignore the first line, and take the median of the rest. A visible window shows the same numbers, updated once a second. The mouse pulls the particles, so for repeatable numbers keep it parked (or set `target` to a fixed point, as the measured copies did).
