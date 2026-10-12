# Kernels: Kestrel's inner loops against Rust

These are small loops from the particles demo, timed alone so that a change to the compiler's code for a loop shows up without a window, a GPU or a draw call in the way. Each kernel is a Kestrel program with a `pub bench(n: int, iters: int) -> float` that makes `n` particles, runs the kernel over them `iters` times and returns a checksum. The numbers are nanoseconds per particle (per item), the median of 9 repetitions, with the cost of making the particles taken out by also timing a run with 0 iterations.

- `step.kes`: the demo's `Particle.step` (pull towards a target, damping, move, bounce off the edges), in a `par for` over a `list[Particle]`. `pow(KEEP, dt)` is worked out once and passed in.
- `step_scalar.kes`: the same step written by hand with `float` locals: the four fields are read once, the maths uses no `vec2` values, and the fields are written once. This is the speed the compiler should reach on `step.kes` by itself.
- `step_pow.kes`: the step exactly as the demo writes it, with `pow(KEEP, dt)` for every particle.
- `copy.kes`: the demo's draw copy, every particle's position into a `list[vec2]`.
- `src/bin/rust.rs`: the same three kernels in Rust (`step`, `pow` and `copy`), one thread, same arithmetic.

## How to run it

From `kestrel/bench/kernels` (it is its own Cargo workspace, listed in `exclude` in `kestrel/Cargo.toml`, so the compiler's tests do not build it):

```
cargo build --release
py measure.py --rounds 3 --threads 1
py measure.py --rounds 3 --threads 16
```

`measure.py` runs every kernel for a few rounds, one after the other so they share the machine's conditions, and prints the median of the rounds' medians with their range. `--threads` sets `KESTREL_THREADS` for the Kestrel kernels, which is the thread count of their `par for`; the Rust kernels always use one thread. A single kernel can be run with `target/release/kernels.exe step.kes --release`, and takes `--n`, `--iters`, `--reps` and, without `--release`, checks integer overflow as `kestrel run` does.

To see what the compiler made of a loop, `target/release/kernels.exe step.kes --release --dump par` prints the optimised Cranelift IR and then the machine code of every `par for` body. `--dump NAME` does the same for the function with that name. The `explicit_slot` lines at the top of the IR are stack slots. For `step.kes` there are 8 of them, each holding a `vec2` temporary of the step (the vector to the target, its direction, the pull, and so on) that is stored to the stack and loaded back.

## Before scalar replacement

Measured on 2026-10-11, with the compiler at commit abc4d6a, on an AMD Ryzen 7 5800X (8 cores, 16 threads), Windows 11. The machine is noisy, so compare rows inside a column, and compare columns only for the same run. Each cell is the median of 3 rounds with the range of the rounds in brackets, in ns per particle.

| Kernel | One thread | 16 threads |
|---|---:|---:|
| `step.kes` (the demo's step, `pow` hoisted) | 13.06 (12.93-13.19) | 1.86 (1.84-1.86) |
| `step_scalar.kes` (written with float locals) | 4.42 (4.36-4.44) | 0.78 (0.76-0.78) |
| `step_pow.kes` (`pow` per particle, the demo) | 24.65 (24.59-24.83) | 3.38 (3.37-3.40) |
| `copy.kes` (the draw copy) | 0.70 (0.68-0.70) | 0.68 (0.68-0.69) |
| Rust step | 3.85 (3.82-3.86) | 3.84 (3.84-3.86) |
| Rust pow | 16.10 (16.00-16.15) | 16.06 (16.03-16.09) |
| Rust copy | 0.20 (0.20-0.21) | 0.21 (0.20-0.22) |

The Rust kernels are one thread in both columns, so their two columns are two measurements of the same program: they differ by under 1%.

So at one thread `step.kes` takes 3.4 times as long as Rust's step, while `step_scalar.kes` takes 1.15 times as long: the difference between the two is the `vec2` values the compiler keeps in memory. `copy.kes` takes 3.5 times as long as Rust's copy. The `copy.kes` kernel does not use `par for`, so its two columns agree.

The "after" numbers come with the last task of the scalar replacement plan, measured the same way.
