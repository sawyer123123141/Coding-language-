# Fast deterministic float math (design, 2026-10-28)

## Why

The determinism round (spec `2026-10-27-determinism-design.md`, decision 2) moved every transcendental function onto the `libm` crate, so a program computes the same bits on every x86-64 CPU. That cost speed: `pow` got 2.67 times slower per call, and the JIT kernel `step_pow` 2.02 times slower (`kestrel/bench/determinism/results.md`, followups entry 47). The user: "for games math is a huge part". A spike (`.superpowers/sdd/fastmath-spike-report.md`, patch beside it) measured the fix. Its "wide" method computes a `float` function in `f64` from `+ - * /` and exact operations and rounds once. That is deterministic by construction, at least as accurate as today, and fast, above all when inlined. This round builds the spike's recommended set; the choices are the controller's under the user's standing delegation, written here as decided.

## Goals

Numbers are the spike's, on the user's Ryzen 7 5800X, as ns per call (random inputs) or per item, today against the target.

- G1. **Accuracy:** every `float` transcendental is within **0.501 ulp** of the true value. For one-argument functions this holds on all 2^32 inputs; for two-argument functions on at least 1M random inputs plus every pair of edge values. Special inputs (NaN, infinities, zeros, huge arguments) give today's bits. Float `%` stays exact: identical values to today's on every input, NaN payloads aside.
- G2. **Determinism:** identical bits in the JIT, a checked exe and a `--release` exe, at any thread count. The runtime functions give identical bits whether built for the SSE2 baseline, with `-C target-cpu=native` (AVX2 and FMA allowed), or in debug. The inline code gives identical bits to the Rust function on every input tested.
- G3. **Speed per call:**

  | function | today | target |
  |---|---:|---:|
  | `sin` | 15.4 | ≤ 6 |
  | `cos` | 15.5 | ≤ 5.5 |
  | `exp` | 7.6 | ≤ 4 |
  | `log` | 6.1 | ≤ 4.5 |
  | `pow` | 36.1 | ≤ 12 |
  | `atan2` | 19.7 | ≤ 11 |
  | float `%` | 5.5 | ≤ 3 (inlined) |

  `tan`, `asin`, `acos` and `atan` must be no slower than today's.
- G4. **Speed in kernels:** the spike's `trig_step.kes` (`atan2`, `cos`, `sin` per mover) goes from 34.7 to **≤ 22 ns** per mover (the spike's inlined estimate was 19.8). `step_pow.kes` goes from 33.2 to **≤ 5 ns** per particle once `pow(KEEP, dt)` is hoisted out of the loop (`step.kes`, hoisted by hand, is 4.3). Programs without transcendental math stay within ±2%: the noise kernels, the frozen voxel world and the engine bench's p99.

## Non-goals

- `f64` math stays on `libm`. The spike could not measure CORE-MATH. A port of ARM's optimized-routines (`exp`, `log`, `pow`; license to check) is a later round (followups).
- `vec4` SIMD transcendentals (the spike's narrow x4: 0.9 to 2.5 ns per value) are a later round; Kestrel has no `sin(vec4)` yet.
- No correctly rounded `float` functions, no table-driven `powf`, no inline `pow` (inlining gained about 2%).
- No new syntax, names, flags or fast-math modes. No new Cargo dependency.
- No change to `f64` `%` or to `round`'s existing float `select`.

## Anti-goals (failures even if they "work")

- Any result that differs between the JIT and an exe, between machines, between runtime builds, or between thread counts.
- Inline code that disagrees with the Rust function on any input, or two sources for one constant.
- Accuracy above 0.501 ulp on any tested input, or a special input whose result differs from today's.
- A branch on the inline fast path other than the one special-input branch. That includes the jumps hidden inside Cranelift pseudo-instructions: a float `select`, `fmin`/`fmax`, and the saturating float-to-int conversions.
- Runtime code that calls the C runtime's math. On the SSE2 baseline Rust's `f64::trunc` and `floor` call it; use `libm::trunc` or exact arithmetic.
- Hoisting that changes any output, moves a check, reads a value the loop changes, or crosses into a `par for` body.
- Re-pinning bits to make a test pass without explaining why they changed.

## The design (decisions)

1. **Scope.** The `float` (`f32`) forms of `sin cos tan asin acos atan atan2 exp log pow` and float `%` become "wide" functions: take the `f32`, compute in `f64` using only `+ - * /`, `sqrt`, integer bit operations and exact rounding to an integer, then round to `f32` once.
   - Algorithms: the spike's for `sin`, `cos`, `exp`, `log`, `pow`, `atan2` and `%`.
   - `tan` is `s/c` or `-c/s` from the same `sin` and `cos` polynomials, with one division.
   - `atan(x)` is `atan2(x, 1)`'s core. `asin(x)` is `atan2(x, sqrt((1-x)(1+x)))` and `acos(x)` is `atan2(sqrt((1-x)(1+x)), x)`. For `|x| ≥ 0.5`, `(1-x)` and `(1+x)` are exact in `f64` and so is their product.
   - The accuracy gate decides: a function that cannot meet 0.501 ulp stays on today's `libm` path, and the round reports it.
   - Some outputs differ from today's on rare inputs (the spike measured 99.97 to 100% identical). That is allowed. Determinism means identical everywhere, not identical to the previous compiler. The results list the share of changed bits per function.
   - The 36 pinned `float` rows of `det_math.kes` keep their bits.

2. **The fast and slow paths.** Each function has a fast path and a special-input test. The slow path is today's code: `libm` in `f64` on the widened operands, narrowed once, or `libm::fmodf` for `%`. So special inputs keep today's bits.

   | function | fast path when | otherwise |
   |---|---|---|
   | `sin` `cos` `tan` | `|x| < 4e8` | NaN, ±inf, huge |
   | `exp` | `x` is not NaN | (±inf clamps on the fast path) |
   | `log` | `0 < x < inf` | |
   | `atan2(y, x)` | both finite and not both zero | |
   | `atan` | `|x| < inf` | |
   | `asin` `acos` | `|x| ≤ 1` | |
   | `%` | `q = trunc(a/b)` has `|q| < 2^29`, and `b` is finite | |
   | `pow` | `x > 0`, `x` finite, `y` finite | (always a call) |

   Each test is written so that NaN takes the slow path: "unordered or ≥", never "not <".

3. **One Rust reference per function**, in the runtime (`kestrel_runtime::math::wide`), exported as `kestrel_rt_<op>f(f32...) -> f32` (`kestrel_rt_sinf`, ..., `kestrel_rt_powf`). `kestrel_rt_fmod_f32` keeps its name and calls the wide `%`. Every polynomial coefficient and range constant is a `pub const` there. The inline emitter reads those constants, and its helper functions mirror the Rust helpers one for one, in the same operation order. Rounding to an integer uses the `1.5·2^52` add: `t = v + R`, `k = t - R`, and `k` as an integer is `bits(t) - bits(R)`. It needs no float-to-int conversion; Cranelift's trapping and saturating conversions are a trap and a jump sequence on x64.

4. **Inline as Cranelift IR** for every `float` function except `pow`: `sin cos tan atan2 atan asin acos exp log` and `%`. `pow` calls `kestrel_rt_powf`.
   - Shape: the whole fast computation is emitted in the current block. It is pure and cannot trap on any input: no float-to-int conversion, exceptions masked. Then one `brif special, slow, merge(fast)`. The cold slow block calls the full `kestrel_rt_<op>f`, whose own test sends the input to `libm`.
   - Because the fast code dominates, Cranelift's GVN shares one range reduction between `sin(a)` and `cos(a)`. The only extra cost for a special input is the unused fast computation.
   - **Selects are integer operations on bitcasts**: an integer `select` (a `cmov` on x64) or `band`/`bor` masks. Never a float `select` (on x64 it is `XmmCmove`, a jump), never `fmin`/`fmax` (a jump sequence on x64), and never an arithmetic blend (`a·(1-w) + b·w` loses `-0`).
   - A `--dump` test checks each function's code: one `brif`, and none of those pseudo-instructions. If Cranelift's optimiser turns a mask back into a branch, the builder stops and reports instead of working around it.

5. **Hoisting pure math out of loops**: a small KIR pass, `kir/src/hoist.rs`, run last in `kestrel_kir::optimize` (after `promote`, before range analysis).
   - Why: Cranelift's egraph never moves a call or anything with control flow, so neither a runtime call nor the inline code with its slow branch leaves a loop today.
   - What moves: the largest subexpression that is loop-invariant and contains a transcendental `Math` or a float `%`. It moves to a new `Value` local, `let`-bound just before the loop. Loops are `While` and `ForRange`, outermost first, so an expression leaves every loop it is invariant in.
   - Invariant means built only from float and int literals, whole-local `Load`s of `Value` locals the loop never writes, `Cast`s to a float type, float `Unary(Neg)` and `Binary(+ - * / %)`, and scalar `Math`.
   - "Never writes" means no `Let`, no written place and no `Bind` of that local, and it is not the loop variable. A `var` argument counts as a written place (`visit::places`).
   - Integer arithmetic and float-to-int casts are excluded, since they can hold checks. So is any place with a path, and `RefParam` or `Addressed` locals; `promote` already turns a `par for` capture like `dt` into a `Value` local.
   - Why it is safe: these expressions cannot fail (float `/` and `%` by zero give inf or NaN, never a trap; `arith.rs` lowers float `%` to a call with no check) and cannot change anything. So they may move out of a loop that runs zero times, or out of an `if` inside the loop. They are deterministic, so the value is the same bits.
   - Contracts and range analysis see no check and no event in them. A `par for` body is its own function and hoists within itself; nothing crosses `Stmt::ParFor`.

6. **Tests** (the plan has names and assertions).
   - **Accuracy:** a sweep of at least 1M inputs per function (the spike's mixed distributions plus edges) asserts ulps ≤ 0.501 against the `libm` `f64` result, which is within one `f64` ulp of exact. An ignored exhaustive test covers all 2^32 inputs of each one-argument function.
   - **Remedy for the trig functions:** if `sin`/`cos`/`tan` exceed the bound only above some magnitude `T` (the medium reduction's worst cases), the fast-path bound drops below `T` and the rest goes to the slow path. Any other excess stops the round for a report.
   - **Runtime builds:** a pinned hash of all wide outputs over the sweep passes in a baseline release build, a `target-cpu=native` build and a debug build.
   - **Pinned bits:** a pinned table of `float` rows (`math/bits_float.rs`), printed by the new rows of `det_math.kes`.
   - **The new program:** `det_math_float.kes` computes 1M inputs per function with dyadic literals and prints a `checksum` per function. A Rust test computes the same checksums from the runtime functions and compares them with all four builds.
   - **The inline code:** a codegen test compares JIT code against the Rust function on the 1M sweep inputs, checked and release. The `--dump` tests check the branches.
   - **Hoisting:** KIR tests for what moves and what stays, a program test whose output is recorded before the pass exists, and a dump test that `step_pow`'s par body calls `kestrel_rt_powf` once, before its loop.
   - The existing cross-build test runs the extended `det_math.kes` and the new program.

7. **Measurements** (at the end, PC free): per-call ns (wide against today's path, random and in order), the kernels (`step_pow`, `step`, the new `bench/kernels/trig_step.kes`, the noise rows), the frozen world, the engine bench's p99 and the compile time of `voxel/main.kes`. Before is 49ae9f0, after is the round's head; medians, ranges and load go into a new section of `kestrel/bench/determinism/results.md`, together with the accuracy and same-bits table per function. Where a goal is missed, the results say so.

## Constraints

- No new syntax, names or Cargo dependencies. Files under about 400 lines. Messages and comments are plain sentences.
- Builds at Idle priority, `CARGO_BUILD_JOBS=8` when the user is away and 4 if gaming. No benchmark while the user is gaming. Never a visible window, never a bare `link.exe`.
- Never stage `kestrel/check/src/body/enums.rs`, `kestrel/test.kes` or `voxels.exe`; never edit `kestrel/bench/voxels3d/` sources.
- The spike's patch is evidence, not code to paste: port from it on purpose, with the changes the plan lists.

## Left to the builder

- An integer `select` or `band`/`bor` masks per pick, whichever the dump shows branch-free and shorter; whether `sin`, `cos` and `tan` share one emitter; where the cold block sits.
- Horner or Estrin per polynomial, the same order in Rust and IR (the spike found switching changed no bit).
- The machine-code text the dump tests match (cranelift-codegen 0.137's printer); how the sweep inputs reach the codegen and CLI tests (a `#[doc(hidden)] pub` runtime module is expected); the hoisted local's name (it starts with `__`).
- An F64X2 select route that stays in XMM registers, only if measurement shows the general-register round trip matters; otherwise a followup. A polynomial refit only after stopping and reporting the failing gate.
