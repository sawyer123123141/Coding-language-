# Proposal: SIMD vector types (2026-10-21)

**Decided 2026-10-21: the user took every pick.** `vec4`/`ivec4` as 4-lane SIMD values; `ivec4` overflow follows `int`; wrapping operators `*%`, `+%`, `-%` on `int`, `i64` and `ivec4` (they never check); a one-value fill (`vec4(0.5)`) for every vector type, `vec2` and `vec3` included; `ivec2`/`ivec3` built this round as plain (non-SIMD) structs. Next: spec, then plan.

The user leaned toward explicit vector types over auto-vectorisation on 2026-10-21 ("a might be best") and asked for this proposal. It is written in GDScript terms. It shows a pick for each part, the alternatives rejected, and the questions the user has to decide. Nothing is built until the user approves it.

## What SIMD is, briefly

A normal CPU instruction works on one number. A SIMD instruction works on a small pack of numbers at once: four floats added to four floats in one instruction, about as fast as one add. In Godot terms, it is `Vector4` math where the CPU really does all four lanes in one step, instead of four steps.

Kestrel's backend (Cranelift 0.116) only compiles 128-bit vectors on x64. That is four `float`s or four `int`s per instruction (both are 32-bit in Kestrel). Its x64 backend has no rules for 256-bit (AVX2) vectors, so eight lanes at once isn't available. Kestrel already builds for the CPU it runs on (it turns on SSE4.1 and AVX when the machine has them), which the vector instructions need; an exe built here may not run on a much older CPU, which is already true of scalar code.

## The pick: `vec4` and `ivec4` become real SIMD values

The syntax spec already lists `vec4` ("float vectors, SIMD-backed") and `ivec4` ("int vectors"); neither is built. The pick is to build them as one-register SIMD values, and nothing else in this round.

```
let a = vec4(1.0, 2.0, 3.0, 4.0)
let b = vec4(0.5)                  // all four lanes 0.5
let c = a * b + a                  // one multiply and one add, all four lanes
print(c.x)                         // read one lane: 1.5
c.w = 9.0                          // write one lane
let r = c.wzyx                     // swizzles, as on vec3

let h = ivec4(1, 2, 3, 4)
let k = (h ^ (h >> 13)) & 0xFFFF   // bit operations on all four lanes, with a plain int on the right
let f = vec4(k) / 65535.0          // ivec4 to vec4: four conversions in one instruction
```

What they feel like to a GDScript programmer: `Vector4` and `Vector4i`, with lane names `x y z w`, the same operators, and the speed promise that each operator works on all four lanes at once (one instruction for most, a few for `min`, `max` and float-to-int conversion).

### What they can do (the first round)

- **Make one:** `vec4(x, y, z, w)`; `vec4(v)` with one value fills all four lanes; `vec4()` is all zeros. Mixed forms like `vec4(v3, 1.0)` (a `vec3` plus `w`) are allowed, as in shaders. Same for `ivec4`.
- **Lanes:** `.x .y .z .w` to read and write, and swizzles (`v.xy` gives a `vec2`, `v.xyz` a `vec3`, `v.wzyx` a `vec4`).
- **Operators, lane by lane:** `+ - *` on both and `/` on `vec4`, with a plain number on either side; `ivec4` also gets `& | ^ << >>` and `~`. Not `/` or `%` on `ivec4`: the CPU has no vector integer divide, so they would quietly run four scalar divides. For powers of two, `>>` and `&` do the same job at full speed.
- **Built-in functions, lane by lane:** `min max clamp abs floor ceil round sqrt lerp`, and `smooth`-style formulas just work because they are made of operators.
- **Across lanes:** `v.sum()`, `a.dot(b)`, `v.length()`, `v.normalized()` on `vec4`.
- **Conversions:** `vec4(iv)` (int lanes to float lanes) and `ivec4(v)` (float lanes to int lanes, truncating, like `int(x)`).
- **Storage:** in locals, struct fields, `[vec4]` arrays and `list[vec4]` lists, 16 bytes each. A `[vec4]` read is one load.
- **Comparisons:** `a == b` is true when all four lanes are equal. Lane-by-lane comparisons are left for a later round (see "Not in this round").

### Overflow on `ivec4`

`ivec4` follows the same rules as `int`: a checked build stops the program when a lane overflows, and `--release` wraps. Range analysis treats the four lanes as one range (the widest), so masked values like the noise hash's lose their checks, the same as scalar code does today. Checks on vectors cost much more than on scalars: the CPU sets no overflow flag for lanes, so a checked build has to redo the math in wider lanes (two halves of two 64-bit lanes) and compare. I'd expect checked builds of vector-heavy integer code to be several times slower than `--release` where range analysis can't remove the checks (not measured). Float vectors have no such checks. That is the price of keeping one rule for all integers.

## Why this pick

- **Predictable.** You write `vec4`, you get SIMD. The compiler never silently decides your loop isn't vectorisable.
- **Already in the spec**, with Godot names the user knows.
- **Small to build.** Cranelift has the 4-lane float and int instructions; the work is in the checker, KIR, codegen and range analysis, the same pipeline `vec2`/`vec3` went through.
- **Combines with `par for`.** Threads split the chunks across cores; vectors speed up each core.

## Rejected

1. **Auto-vectorising loops** (the compiler packs plain loops into SIMD on its own). It needs nothing new from the user, but it is unpredictable: a small change to a loop can silently turn it off. It is also much harder to build: Cranelift doesn't do it, so Kestrel would need its own loop analysis. It may come later on top of the vector types.
2. **Generic widths** (`simd[float, 8]`, as in Rust's portable SIMD or Mojo). Not GDScript-like, and the backend has no 8-wide vectors anyway.
3. **Making `vec3` SIMD too** (padding it to four lanes). `vec3` is already split into three plain floats, which runs close to Rust's speed (scalar replacement: the particle step went from 3.4x to 1.15x Rust). Padding wastes a lane, and adding up the lanes is slow. `vec3` stays as it is.
4. **`ivec4` that always wraps** (like shaders and Godot's `Vector4i`). It is faster in checked builds, but it would give vector integers different rules from plain `int`. A silent wrap in voxel coordinates is the kind of bug Kestrel's checks exist to catch.
5. **A `@vectorize` hint on loops.** It still needs an auto-vectoriser behind it (see 1).

## What it would buy, honestly

- **Noise and other math on packs of four** (four noise corners at once, four particles at once): up to about 4x on that code, less in practice once loads, stores and the rest of the work are counted. I'd guess 2-3x on the voxel noise; that is a guess, not measured.
- **But the voxel benchmark's hash doesn't fit as written.** It multiplies in `i64` to avoid overflow. Four `i64` lanes would take two registers, and without AVX-512 each 64-bit lane multiply is a sequence of several instructions, so there's little to win. A 4-lane hash has to work in 32-bit lanes and let the multiply wrap (see question 1). That is a different hash, so a different world and checksum. It belongs in the no-rules engine version, not the frozen benchmark.
- **Plain code gets nothing.** Only code written with `vec4`/`ivec4` speeds up.

## Questions for the user

1. **Wrapping math for hashes (the main one).** Hash functions *want* multiplication to wrap around. Today that needs `i64` and masks; in four 32-bit lanes it has no workaround, so with the overflow rule above, a vector hash would stop in a checked build. Options:
   - **(a, my pick)** Wrapping operators that work on `int`, `i64` and `ivec4`: `a *% b`, `a +% b`, `a -% b` (the `%` says "wraps around"). They never check, in any build. Example: `h = (h ^ (h >> 13)) *% 0x5BD1E995`.
   - (b) Methods: `a.wrapping_mul(b)`. Clearer to read, much longer to write in a hash.
   - (c) A wrapping block: `wrap { ... }` where every operator inside wraps. Short for long hashes, but easy to wrap too much by accident.
2. **One-value fill:** `vec4(0.5)` fills all four lanes (my pick, as in shaders), or a named form like `vec4.all(0.5)`? Today `vec3(1)` with one value is an error ("give every component, or none for a zero vector"). My pick also lets `vec2(0.5)` and `vec3(0.5)` fill, so all vectors follow one rule.
3. **Should `ivec2`/`ivec3` come in this round too**, as plain (non-SIMD) structs like `vec2`/`vec3`, for voxel coordinates? My pick: yes, they're cheap and the spec lists them.

## Not in this round

- Lane-by-lane comparisons and choosing per lane (`select(a < b, x, y)`), which branchy math needs. Later, once the basics are measured.
- 8-wide vectors (the backend doesn't have them), `f64`/`i64` vectors, auto-vectorisation.
- `/` and `%` on `ivec4` (no vector integer divide on the CPU).
- `vec4`/`ivec4` in the Godot add-on's exports (Godot's `Vector4`/`Vector4i`); a small follow-up.

## How it would be built (sketch, for the plan)

1. Types, making, lanes, swizzles, `+ - * /`, storage (checker, KIR, codegen with Cranelift's `F32X4`/`I32X4`).
2. `ivec4` bit operations, conversions, the built-in functions, across-lane operations.
3. Overflow checks per lane, range analysis for lanes, and the wrapping operators (question 1).
4. Measure: a kernel (four particles or four noise corners at once) against Rust's `std::simd` or hand-written 4-lane code, and use it in the no-rules voxel engine version.
