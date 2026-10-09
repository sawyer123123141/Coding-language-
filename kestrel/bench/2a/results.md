# Plan 2a benchmarks: Kestrel vs Rust vs C

2026-10-09, Windows 11, x86-64. Median of 5 wall-clock runs from PowerShell (`run.ps1`); process start is about 15 ms and is included. All variants print identical output (checked by the script).

| Workload | Kestrel exe (checked) | Kestrel exe (`--release`) | `kestrel run` (JIT, incl. compile) | Rust -O3 (overflow checks on) | Rust -O3 (release default) | C gcc -O2 |
|---|---:|---:|---:|---:|---:|---:|
| fib(38), recursive i64 | 241 ms | 229 ms | 242 ms | 160 ms | 98 ms | 60 ms |
| loop, 200M iterations of i64 multiply + mod | 674 ms | 673 ms | 675 ms | 541 ms | 562 ms | 537 ms |
| collatz, longest chain under 1M | 525 ms | 518 ms | 542 ms | 152 ms | 99 ms | 111 ms |
| mandelbrot 2000², 200 iterations, f32 | 407 ms | 410 ms | 420 ms | 341 ms | 362 ms | 344 ms |

Kestrel `--release` ÷ Rust release: fib 2.3x, loop 1.2x, collatz 5.2x, mandelbrot 1.1x.

**Update (commit 59dd0f2, division by a constant strength-reduced):** collatz `--release` 518 ms → **237 ms** (2.4x Rust, was 5.2x). Output unchanged.

## Reading it

- **Plain loops and float maths are close to Rust and C** (loop 1.2x, mandelbrot 1.1x). That is Cranelift vs LLVM codegen on tight scalar code, the expected gap.
- **Collatz: division by a constant.** Rust and C turn `n / 2` and `n % 2` into shifts; Kestrel emits a real `idiv` behind its zero and `MIN / -1` checks. Evidence: the Rust version with the divisor hidden behind `black_box` (`collatz_vardiv.rs`) runs in **407 ms**, close to Kestrel's 518 ms. Fixed for powers of two (shifts) and check elision for other constants in 59dd0f2. The remaining gap is likely the bias chain on signed `% 2` and the checks around `3 * n + 1`; `x % 2^k == 0` as a mask test and magic-number division for other constants are the next steps.
- **fib: call overhead.** Cranelift doesn't inline or transform recursion; GCC turns one of fib's two recursive calls into a loop, and LLVM similarly. A later release backend (sub-project 5) closes this.
- **Overflow checks are nearly free in Kestrel** (checked vs `--release` within 5%). Rust's checks cost 60% on fib.
- **JIT compile time is invisible:** `kestrel run` is within about 10-20 ms of the prebuilt exe, which includes compiling.
- Compared with the old `kestrelc` on the same machine (benchmarks/results.md): its integer loop took 0.748 s, the new compiler takes 0.673 s. Its fib "win" came from automatic memoization, which the new compiler deliberately doesn't do.

## Reproduce

```powershell
cd kestrel; cargo build --release -p kestrel-cli; cd bench\2a; .\run.ps1
```
Needs `rustc` and `gcc` on PATH.

## After plan 2b-i (structs), 2026-10-10

Same suite plus `particles`: one struct of four floats updated by a `var self` method, 100M steps. This run was about 15% noisier across the board (Rust's own fib went from 98 to 111 ms), so compare ratios rather than raw times.

| Workload | Kestrel exe (checked) | Kestrel exe (`--release`) | `kestrel run` | Rust -O3 (checks on) | Rust -O3 | C gcc -O2 |
|---|---:|---:|---:|---:|---:|---:|
| fib | 290 ms | 265 ms | 287 ms | 192 ms | 111 ms | 84 ms |
| loop | 744 ms | 750 ms | 754 ms | 599 ms | 608 ms | 580 ms |
| collatz | 304 ms | 289 ms | 363 ms | 188 ms | 121 ms | 133 ms |
| mandelbrot | 501 ms | 492 ms | 524 ms | 400 ms | 377 ms | 378 ms |
| particles | 459 ms | 453 ms | 484 ms | 366 ms | 359 ms | 143 ms |

Kestrel `--release` ÷ Rust release: fib 2.4x, loop 1.2x, collatz 2.4x, mandelbrot 1.3x, **particles 1.26x**.

- Structs cost little: the method call passes a pointer and the fields are loaded and stored in memory, and it lands within 1.3x of Rust (which inlines `step` and keeps the fields in registers).
- C is far ahead on particles (143 ms); GCC keeps the particle in registers across the whole loop. Inlining small methods and promoting struct fields to registers (KIR scalar replacement) would close most of the gap; both are candidates for sub-project 5.
