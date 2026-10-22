# SIMD vector types: design (2026-10-21)

**Source:** the proposal `docs/superpowers/plans/2026-10-21-simd-proposal.md`, decided 2026-10-21: the user took every pick (`vec4`/`ivec4` as 4-lane SIMD values; `ivec4` overflow follows `int`; wrapping operators `*%`, `+%`, `-%` that never check; a one-value fill for every vector type; `ivec2`/`ivec3` this round as plain structs). This spec settles what the proposal left open. **Plan:** `docs/superpowers/plans/2026-10-21-simd-vectors.md`.

Where the proposal is settled this spec only restates it. Each open detail has a ruling and one line of why ("Rulings", R1-R24). Four things users would see that the proposal doesn't show are **syntax picks for the user to confirm**; they are built as written, each in one place, so a change is cheap.

## Syntax picks for the user to confirm

1. **Compound wrapping assignment: `h *%= K`, `h +%= x`, `h -%= x`.** Like `*=` for `*`. They lex as one token each and are built in `syntax/src/lexer.rs` (the three-character table) and `check/src/body/wrap.rs`.
2. **A hex literal written directly next to a wrapping operator may use the type's whole bit width:** `h *% 0x9E3779B1` is `h *% -1640531535` in an `int` (and up to `0xFFFFFFFFFFFFFFFF` in an `i64`). Hash constants are usually written this way; without it they must be written as negative decimals. Only hex, only as a direct operand of `*% +% -%` (or the right side of `*%=` and the others); everywhere else `0x9E3779B1` is still "doesn't fit in int". Built in one function, `wrap_literal_ok` in `check/src/body/wrap.rs`.
3. **Mixed constructors follow the shader rule for every vector size:** the values' components must add up to the vector's, in order: `vec4(v3, w)`, `vec4(v2, z, w)`, `vec4(a.xy, b.zw)`, `vec3(v2, z)`, `ivec4(iv2, 0, 1)`. Labels work only when every value is a number (`vec4(x: 1, y: 2, z: 3, w: 4)`, as `vec3(x: ...)` today). Built in `check/src/body/vec_make.rs`.
4. **One rule for every vector type.** What the proposal lists for `vec4` also works on `vec2`, `vec3`, `ivec2`, `ivec3` where it makes sense: the lane-by-lane built-ins (`min max clamp abs` on all; `floor ceil round sqrt lerp` on float vectors), with a plain number filling all lanes in any argument (`clamp(v, 0.0, 1.0)`); `v.sum()`; `a == b` and `a != b`; conversions between float and int vectors of one size (`vec3(iv)`, `ivec2(v)`); 4-letter swizzles of `vec2`/`vec3` giving a `vec4` (`v.xyzz`). And `ivec2`/`ivec3` follow `ivec4` in having **no `/` or `%`** (the same error), so moving code between int vector sizes never changes what compiles. Built in `check/src/vectors.rs` (the table of what each type has).

## The types

| Type | Lanes | What it is | Size, alignment |
|---|---|---|---|
| `vec2`, `vec3` | 2, 3 `float` | built-in structs, unchanged (scalar replacement splits them into floats) | 8/4, 12/4 |
| `ivec2`, `ivec3` | 2, 3 `int` | new built-in structs, like `vec2`/`vec3`; scalar replacement splits them into ints | 8/4, 12/4 |
| `vec4` | 4 `float` | one SIMD value (Cranelift `F32X4`), never split into lanes | 16/16 |
| `ivec4` | 4 `int` | one SIMD value (Cranelift `I32X4`), never split into lanes | 16/16 |

All six print as `vec4(x: 1.5, y: 3, z: 4.5, w: 6)` (R13), are plain copyable values (no ownership, allowed in `pure` functions and `par for` bodies), and work in locals, parameters, results, struct fields, `[T]`, `list[T]`, `T?`, map values, constants and field defaults. They are not map keys.

## Making them

- `vec4(1, 2, 3, 4)` (numbers; ints convert to floats as today), `vec4(0.5)` fills, `vec4()` is zeros, the same for every vector type.
- Mixed: syntax pick 3.
- One value of the same type copies it (`vec4(v)`); one vector of the other kind and the same size converts it (`vec4(iv)`, `ivec4(v)`; R10).
- `ivec*` take ints and int vectors only: `ivec4(1.5)` is an error (V2), as `int` needs `int(x)`.

## Lanes and swizzles

`.x .y .z .w` read and write one lane (`.w` only on 4-lane types). `v.x += 1` and `p.vel.w = 0` and `vs[i].y = 7` work; a lane of a `var` parameter is written in place. Swizzles of 2 to 4 letters, repeats allowed, give a vector of that many lanes of the same kind (`v.xy` is `vec2`, `iv.zyx` is `ivec3`, `v.wzyx` and `v3.xyzz` are `vec4`). Swizzles are read-only, as today.

## Operators

| | `vec2 vec3 vec4` | `ivec2 ivec3 ivec4` |
|---|---|---|
| `+ - *` | lane by lane | lane by lane, checked like `int` (R6) |
| `/` | lane by lane | error V5 |
| `%` | error (today's "`%` doesn't work on vectors") | error V6 |
| `-v` | yes | yes, checked (`-` of the lowest `int` overflows) |
| `& \| ^`, `~v` | error ("needs integers") | lane by lane |
| `<< >>` | error | every lane by one plain `int` amount, checked like `int` (R8) |
| `*% +% -%` | error W1 | lane by lane, never checked |
| `== !=` | all lanes equal / some lane differs (R12) | same |
| `< > <= >=` | error V8 | error V8 |

A plain number on either side of a binary operator fills every lane (`2 * v`, `h & 0xFFFF`); an int vector with a `float` is error V7. Compound forms (`+=`, `&=`, `>>=`, `*%=` ...) follow their operators. Precedence: `*%` with `* / %`, `+% -%` with `+ -` (R3).

## Built-in functions

- Lane by lane: `min max clamp abs` on every vector; `floor ceil round sqrt` on float vectors (`round` halves away from zero, as the scalar one); `lerp(a, b, t)` with two float vectors and a number `t`. Any argument may be a plain number, which fills the lanes (pick 4). `abs` of an int vector is checked like `-v`.
- Across lanes: `v.sum()` on every vector (lanes add left to right, `((x + y) + z) + w`, R11; an int vector's sum is checked like `+`); `a.dot(b)`, `v.length()`, `v.normalized()` (a zero vector stays zero), `a.distance_to(b)` on float vectors; `cross` on `vec3` only.
- Everything else (`sin`, `pow`, ...) takes numbers only, as today.

## Wrapping operators

`a *% b`, `a +% b`, `a -% b` on `int`, `i64`, `u8`, `ivec2`, `ivec3` and `ivec4` (R4): the result wraps around the type's width, in every build, with no check. Both sides meet at one type as `*` does (`int *% i64` is `i64`). A float is error W1. They make hashes possible without `i64` and masks: `h = (h ^ (h >> 13)) *% 0x5BD1E995`.

## Overflow on int vectors

`ivec4` follows `int`: a checked build stops with "integer overflow on line N of f.kes" when any lane of `+ - *`, `-v` or `abs` overflows, and "shift amount out of range" for a bad shift amount; `--release` wraps. `ivec2`/`ivec3` are structs of ints, so each component gets an ordinary `int` check. **Cost** (x64 has no overflow flag for lanes; R6): `+`/`-` about 5 more instructions than the bare one, `*` about 12, `-v` and `abs` about 3, a shift one compare of its scalar amount. Range analysis removes them where it can (R16); `--release` has none.

## Storage, layout and calls (R1, R2)

A `vec4`/`ivec4` is 16 bytes aligned to 16 everywhere it is laid out: struct fields (`struct P { id: u8, pos: vec4 }` is 32 bytes, `pos` at 16), arrays and lists (16-byte stride, 16-aligned blocks), optional payloads (at 16), map entries, closure environments, interface boxes, globals and stack slots. Codegen loads and stores them **without** Cranelift's `aligned` flag, so a layout that misses the alignment is slower, not a crash. A copy is one 16-byte load and store. Parameters and results use Cranelift's own Windows x64 rules: a vector argument goes by an implicit pointer to a 16-byte copy, a result comes back in XMM0. Natives can't take them (today's error); exporting a function that takes or returns `vec4`, `ivec4`, `ivec2` or `ivec3` to Godot is today's "can't export" error until the follow-up.

## Range analysis (R16-R19)

An `ivec4` local is tracked with **one range for all four lanes** (the widest): a make takes the hull of its lanes, a fill the number's range, a swizzle or a lane read the vector's range; `+ - * & | ^ << >> ~ -v` and the wrapping operators use the scalar rules on those ranges, which hold lane by lane. Writing one lane widens the range to cover the old one and the new lane (one lane changes, three stay). A vector check is removed only when the widest range proves it for every lane: one check per vector operation, removed whole or kept whole. A vector is never a term of a fact (no `a - b <= c` between vectors, no linear forms), and **a condition on a lane (`if v.x < 10`, `@assume(v.x < 10)`) teaches nothing about the vector**. Memory loads of vectors (fields, list items) are any `int`, as other loads.

## Report and `@prove` (R20-R22)

A vector operation's check is one item: "`a * b` may exceed `int` in a lane (the lanes of `a` can be any `int`; the lanes of `b` are 0 to 65,535)". `@prove(no_checks)` on vector code holds when every vector operation's check was removed (or it has none: float vectors and wrapping operators never check). `no_panic` counts them the same way; `no_alloc` is unaffected (vectors never allocate; a `[vec4]` literal does, as any array). Nothing else in Round B's contracts changes.

## Errors

Plain sentences that say what to do (exact text in the plan's Global Constraints):

- W1: "`*%` wraps integers; for floats use `*`" (and `+%`/`+`, `-%`/`-`, `*%=`/`*=` ...).
- V1: "`vec4` has 4 components, but these values give 3; give 4, one value for all of them, or none for zeros".
- V2: "`ivec4` holds ints, found float; convert it with `int(...)` first".
- V3: "`ivec3` takes ints and int vectors, found vec2; convert it with `ivec2(...)` first", and the other way "`vec3` takes floats and float vectors, found ivec2; convert it with `vec2(...)` first" (a plain `f64` keeps today's conversion error).
- V4: "labels only work when every component is a number, like `vec4(x: 1, y: 2, z: 3, w: 4)`".
- V5: "int vectors have no `/`; for a power of two use `>>`, or divide each component, like `v.x / n`".
- V6: "int vectors have no `%`; for a power of two use `&`, or take each component, like `v.x % n`".
- V7: "`*` can't mix ivec4 and float; convert with `vec4(...)` first".
- V8: "`<` doesn't compare vectors; compare components, like `a.x < b.x`".
- V9: "`>>` shifts every component by one `int`; shift each component on its own for different amounts".
- V10: "`floor` needs floats; convert with `vec4(...)` first". V11: "`length` needs a float vector; convert with `vec4(...)` first". V12: "`cross` needs a vec3; vec4 has no cross product".
- C1 (codegen): "`vec4` and `ivec4` need a CPU with SSE4.1 (every x64 CPU since about 2011); this one doesn't have it".

## How it is built

- **Checker:** `Ty::Vec4` and `Ty::IVec4` (plain values, not aggregates); `IVEC2 = StructId(2)`, `IVEC3 = StructId(3)`, `BUILTIN_STRUCTS = 4`; `check/src/vectors.rs` is the one table of what each vector type has (`VecKind { dims, int, simd }`, `Tables::vector(ty)`); `vec4`, `ivec2`, `ivec3`, `ivec4` become reserved names. Lanes of `vec4`/`ivec4` are recorded in `Tables::lanes` and are places like struct fields.
- **KIR:** `KTy::F32X4`, `KTy::I32X4` (register values: `is_aggregate` false, `is_vector`, `lane()`), one leaf each for scalar replacement and promotion. New expressions `VecLit` (four lane values), `Splat`, `Shuffle(e, [u8; 4])`, `LaneOf(e, lane)` (a lane of a value that isn't a place); a place may end in `Proj::Lane(lane)`, and only there. `BinOp::{AddWrap, SubWrap, MulWrap}` on any integer type and `I32X4`. A vector binary operation has two operands of its type (lowering wraps a plain number in `Splat`), except `Shl`/`Shr`, whose amount is an `I32`. `Eq`/`Ne` on vectors give one `Bool`. `Cast` converts between `F32X4` and `I32X4`. `Math` takes `F32X4` for `Sqrt Floor Ceil Round Abs Min Max` and `I32X4` for `Min Max`; an int vector's `abs` lowers to `max(v, -v)`, so its check is the `Neg`'s. `ivec2`/`ivec3` lower like `vec2`/`vec3`, component by component.
- **Codegen:** `cl_type` maps them to `F32X4`/`I32X4`; Cranelift handles the calling convention. Lanes: `extractlane`/`insertlane` on registers, a 4-byte load or store at `4 * lane` in memory. Swizzles: `bitcast` to `I8X16` (little-endian), `shuffle`, `bitcast` back. Casts: `fcvt_from_sint`, `fcvt_to_sint_sat`. `==`: `icmp`/`fcmp` then `vall_true`. The checks are in `func/simd_check.rs` (R6). Printing extracts the lanes and prints them with the type's pieces; no new runtime function.

## Rulings

1. **Alignment 16 in every layout, and no `aligned` flag on vector loads and stores.** Why: aligned blocks keep loads fast and let Cranelift fold them, and dropping the flag means a spot that misses the alignment (an environment, a `par for` block) costs speed, never a fault (`addps xmm, m128` faults on a misaligned address).
2. **Calls use Cranelift's Windows x64 vector convention (implicit pointer in, XMM0 out); natives and Godot exports refuse them.** Why: Cranelift already implements it for every Kestrel-to-Kestrel call, direct, indirect and through interfaces, and natives would need Rust-side layouts this round doesn't need.
3. **`*%` binds like `*`, `+%` and `-%` like `+`, all left to right.** Why: they are the same operations, so `a +% b *% c` must read like `a + b * c`. The tokens can't break a program: `* %`, `+ %` and `- %` never parsed (`%` can't start an expression).
4. **Wrapping operators work on every integer type and every int vector (`int i64 u8 ivec2 ivec3 ivec4`), not on floats.** Why: wrapping is natural on each of them (`u8` wraps at 256), and one rule is easier than a list; floats don't wrap.
5. **Wrapping operators never check, in any build, and range analysis gives their result the wrapped range** (exact when the true result fits, else the whole type). Why: that is what they promise; analysing them as `--release` analyses `+` is already sound code.
6. **Per-lane checks are built from lane tricks, not from Cranelift's overflow instructions (which x64 doesn't lower for vectors):** `+` fails when `(a ^ r) & (b ^ r)` has a sign bit set in some lane (`vhigh_bits` not 0), `-` when `(a ^ b) & (a ^ r)` does; `*` takes the 64-bit products of the low and high lane pairs (`imul` of `swiden_low`/`swiden_high`, which Cranelift lowers to `pmuldq`), adds 2^31 to each and fails when any is 2^32 or more after (`ushr` by 32 not all zero, `vany_true`); `-v` fails when some lane is the lowest `int`. The result itself is the plain `iadd`/`isub`/`imul`. Why: each is a few instructions with no branch per lane, and the value stays hoistable, as the scalar checks' design wants.
7. **The check's message is the scalar one ("integer overflow on line N"), with no lane number.** Why: one rule for all integers; the line points at the operation and `kestrel check --report` explains it.
8. **A vector shift takes one plain `int` amount, checked once like a scalar shift.** Why: x64 shifts all lanes by one amount in one instruction; per-lane amounts need AVX2 rules Cranelift 0.116 doesn't have (error V9).
9. **No `/` or `%` on any int vector (V5, V6), and no `%` on float vectors.** Why: the CPU has no integer lane divide (it would silently be four scalar divides), and float `%` was never on vectors.
10. **`ivec4(v)` truncates toward zero and saturates, NaN giving 0, exactly as `int(x)` does today (`fcvt_to_sint_sat`); `vec4(iv)` rounds to the nearest float. Neither is checked.** Why: one rule with the scalar conversion; `int(x)` of a float has never been a checked narrowing.
11. **Across-lane sums add left to right, `((x + y) + z) + w`.** Why: float results then equal the written-out sum, which tests and the scalar versions of kernels rely on.
12. **`a == b` is true when every lane is equal (a NaN lane is unequal, as for floats); `a != b` is its opposite.** Why: the proposal's rule, and `!=` must be `not ==`.
13. **Vectors print like the struct vectors do today: `vec4(x: 1, y: 2, z: 3, w: 4)`, `ivec2(x: 1, y: -2)`, in interpolation too.** Why: `vec3` already prints this way; one format.
14. **`ivec2`/`ivec3` are `StructId(2)` and `StructId(3)`; `vec2`/`vec3` stay 0 and 1.** Why: the Godot bridge and `export.rs` key on 0 and 1.
15. **SSE4.1 is required for a program that uses `vec4` or `ivec4` (error C1 at build).** Why: `pmulld`, `pminsd`, `roundps` and `ptest` are SSE4.1, and without it Cranelift lowers the vector rounding functions to libcalls Kestrel neither provides nor tests; every x64 CPU from 2011 on has it.
16. **Range analysis tracks an `ivec4` local with one interval (the hull of its lanes); writing one lane joins the old range with the new value's.** Why: the proposal's rule ("the widest"), and the join is what makes a one-lane write sound.
17. **A vector is never a fact term and has no linear form; conditions on lanes teach nothing.** Why: `v.x < 10` says nothing about `v.y`, and facts between vectors would need per-lane reasoning for little gain.
18. **One check per vector operation, removed only when all four lanes are proven.** Why: it is one instruction sequence; there is nothing per lane to remove.
19. **Float vectors are not analysed** (as floats today). Why: they have no checks.
20. **`@prove(no_checks)` means no vector check is left in the region, counted as one item per operation.** Why: contracts read the same sites as the report; a vector check is a site like any other.
21. **`@assume` may name a lane (`@assume(v.x < 10)`), but it teaches nothing, so it gets the "removes no check" warning.** Why: the checker's form rule already accepts fields, and rejecting lanes would need a special message for no gain.
22. **The report names vector checks "in a lane" and describes ranges as "the lanes of `a` are 0 to 99".** Why: the reader must see that one range stands for four lanes.
23. **Closure environment blocks become 16-aligned (were 8), for every closure.** Why: simpler than per-environment alignment; an arena allocation's cost doesn't change.
24. **The frozen voxel benchmark doesn't change.** Why: a 32-bit wrapping hash is a different world and checksum; the measurement uses new kernels (plan, Task 6), and the no-rules engine is where a vector hash belongs.

## Not this round

Lane-by-lane comparisons and `select`; 8-wide vectors; `f64`/`i64` vectors; auto-vectorisation; `/` and `%` on int vectors; per-lane shift amounts; Godot exports of `vec4`, `ivec4`, `ivec2`, `ivec3` (`Vector4`, `Vector4i`, `Vector2i`, `Vector3i`); natives taking `vec4`/`ivec4`; hex bit patterns for `& | ^` (pick 2 covers only the wrapping operators).
