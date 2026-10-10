# Particles demo: measured times

Measured on 2026-10-10. The demo is `particles.kes`: a `par for` moves the particles, then one `draw.rect` call per particle draws them. Drawing is the bottleneck here, not Kestrel's `par for`.

## What the numbers are

- **update** is the time of the `par for` that steps every particle, in ms per frame.
- **draw** is the time of `draw.clear` plus one `draw.rect` call per particle, in ms per frame. It is the time the program spends in those calls. The three text lines of the overlay are not in it.
- **frame** is the time of the whole loop iteration: update, draw, the text, and `window.frame()` (which shows the frame, waits for vsync and reads input).
- **fps** is frames divided by seconds.

Each cell is the **median of the per-second averages** the demo prints (it prints one line a second), over a run of about 11 seconds. The first second is dropped because it includes start-up, which leaves 9 or 10 values per run. The range of those averages is in brackets. These are not medians of single frames: the demo only sums each second, and a frame that stalls shows up in that second's average.

## Setup

- AMD Ryzen 7 5800X (8 cores, 16 threads), NVIDIA GeForce RTX 3070, Windows 11, a 60 Hz display. `par for` used all 16 threads.
- A 1280×720 window, vsync on (`window.open` always turns it on).
- Both `kestrel run --release` (the JIT) and a `kestrel build --release` exe, so the overflow and narrowing checks are off.
- The starting count is `START_COUNT` in the demo, changed per run. The mouse is not used: the measured copies set `target` to the window centre (640, 360), so every run pulls toward the same point.
- Most runs used a visible window. The "hidden" rows set `KESTREL_HIDDEN=1`.
- The machine is noisy (a desktop with other programs open), so read differences under about 10% as noise.

## Results

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

Hidden windows give the same numbers (for example 200,000: update 0.91, draw 30.2 and 29.9, fps 31.9 and 32.3, JIT and exe). A hidden window still waits for vsync, so `KESTREL_HIDDEN=1` doesn't uncap the frame rate.

### Where the time goes

- **vsync caps the frame at 16.67 ms (60 fps)** at 10,000, 50,000 and 100,000 particles. Below 100,000 the program is waiting for the display: at 10,000 the update and draw together use about 2 ms of the 16.67, at 50,000 about 9 ms, and at 100,000 about 16.4 ms, which is just inside the frame. So 60 fps holds up to about 100,000 particles on this machine, and no further: at 200,000 the frame is 30-33 ms (30-33 fps).
- **Drawing is the bottleneck, not the update.** At 200,000 particles the `par for` takes under 1 ms and the draw loop about 30 ms, so the update is 3% of the frame's work. Draw time grows with the count at about 150-190 ns per `draw.rect` call (1.9 ms for 10,000, 8.7 for 50,000, 15.9 for 100,000, 30 for 200,000).
- **That per-call cost is not Kestrel's.** I ran the same demo headless (`KESTREL_HEADLESS` with `KESTREL_HEADLESS_QUIET=1`), where each native call does its checks and reads its arguments but calls no raylib function. At 200,000 particles a whole frame took 2.9 ms (JIT and exe alike), measured as the slope of the run time between a 30-frame and a 330-frame run. If the update is the 0.9 ms measured above (I didn't time it headless), the Kestrel loop and the native-call boundary cost about 2 ms for 200,000 calls, about 10 ns each. The other 28 ms of the 30 ms draw time is spent inside raylib's rectangle path and the graphics driver (raylib batches the vertices and sends them to OpenGL when a batch fills). I did not profile inside raylib, so I can't say how that splits between raylib and the driver.
- **`par for` helps, but it isn't what sets the frame rate here.** With one thread (`KESTREL_THREADS=1`) the 200,000-particle update takes 5.4 ms (5.0-5.9), against about 0.9 ms with 16 threads: 5.9 times faster on 8 cores and 16 threads. That is not 16 times, because the loop moves a lot of memory for very little arithmetic (about 27 ns per particle on one thread) and each frame forks and joins. Over a whole frame the difference is 35.4 ms against 31.3 ms (28.3 fps against 31.9 fps; both are hidden JIT runs).
- **JIT and exe are the same speed** within the noise, as expected: both run the same machine code.

## What would make it faster

Draw calls, not computation. The way out is to draw many rectangles per call (a bulk `draw` function that takes a list of positions) or to use textured batches, which is part of the next spec (textures and 3D). Making each `draw.rect` call cheaper on the Kestrel side can save at most about 10 ns of the 150 ns.

## Reproducing it

From `kestrel/`, with `START_COUNT` edited in the demo:

```
cargo build --release
target/release/kestrel run --release demos/particles/particles.kes
```

The demo prints a line once a second, for example `particles 50000 update 0.41 ms draw 8.68 ms frame 16.66 ms fps 60.03`. Let it run for ten seconds or so, ignore the first line, and take the median of the rest. The window shows the same numbers, updated once a second. The mouse pulls the particles, so for repeatable numbers keep it parked (or set `target` to a fixed point, as the measured copies did).
