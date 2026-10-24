# SIMD vector types: design (2026-10-21)

**Source:** the proposal `docs/superpowers/plans/2026-10-21-simd-proposal.md`, decided 2026-10-21: the user took every pick (`vec4`/`ivec4` as 4-lane SIMD values; `ivec4` overflow follows `int`; wrapping operators `*%`, `+%`, `-%` that never check; a one-value fill for every vector type; `ivec2`/`ivec3` this round as plain structs). This spec settles what the proposal left open. **Plan:** `docs/superpowers/plans/2026-10-21-simd-vectors.md`.

**Revised 2026-10-21 after the Opus review** (1 Critical, 9 Important, 15 Minor) and the controller's rulings: lane-type semantics pinned (R25-R27), `/` and `%` allowed on `ivec2`/`ivec3`, pick 2 widened to every hex literal, `u8` wrapping made a pick, labelled `vec4` makes built, closure copies 16-aligned, the `*` check's cost with a constant stated, a test for C1, and a list of what to recheck after a Cranelift upgrade.

Where the proposal is settled this spec only restates it. Each open detail has a ruling and one line of why ("Rulings"). Five things users would see that the proposal doesn't show are **syntax picks for the user to confirm**; they are built as written, each in one place, so a change is cheap.

## Syntax picks for the user to confirm

**Confirmed 2026-10-22:** the user's "lets continue" was taken as yes to all five; see "As built". They stay listed here as the record of what was picked.

1. **Compound wrapping assignment: `h *%= K`, `h +%= x`, `h -%= x`.** Like `*=` for `*`. They lex as one token each (`syntax/src/lexer.rs`, the three-character table) and are checked in `check/src/body/wrap.rs`.
2. **A hex literal may use its type's whole bit width, anywhere:** `let PRIME = 0x9E3779B1` is the `int` -1640531535, so `h *% PRIME` works; `0xFFFFFFFFFFFFFFFF` is an `i64` -1; with an int vector the literal is measured against the lane type (`int`) and then fills the lanes (`h *% 0x9E3779B1` on an `ivec4`). The literal first takes its type from where it is, as today, and is positive when it fits that type's positive range: `let x: i64 = 0xFFFFFFFF` and `i64(0xFFFFFFFF)` are 4294967295, but `let x = 0xFFFFFFFF` is an `int`, -1 (and stays -1 when later widened to `i64`). Decimal literals keep today's rule ("doesn't fit in int"). Hash constants are usually written in hex; without this they must be negative decimals. Built in one function, `hex_literal` in `check/src/body/expr.rs`'s literal check, with the parser marking hex literals.
3. **Mixed constructors follow the shader rule for every vector size:** the values' components must add up to the vector's, in order: `vec4(v3, w)`, `vec4(v2, z, w)`, `vec4(a.xy, b.zw)`, `vec3(v2, z)`, `ivec4(iv2, 0, 1)`. Labels work only when every value is a number, and they work on every vector, `vec4` and `ivec4` included (`vec4(x: 1, y: 2, z: 3, w: 4)`, `ivec4(1, y: 2, z: 3, w: 4)`, `vec4(w: 4, z: 3, y: 2, x: 1)`), binding like a struct's fields, evaluated in source order. Built in `check/src/body/vec_make.rs`.
4. **One rule for every vector type.** What the proposal lists for `vec4` also works on `vec2`, `vec3`, `ivec2`, `ivec3` where it makes sense: the lane-by-lane built-ins (`min max clamp abs` on all; `floor ceil round sqrt lerp` on float vectors), with a plain number filling all lanes in any argument (`clamp(v, 0.0, 1.0)`); `v.sum()`; `a == b` and `a != b`; conversions between float and int vectors of one size (`vec3(iv)`, `ivec2(v)`); 4-letter swizzles of `vec2`/`vec3` giving a `vec4` (`v.xyzz`). **And `ivec2`/`ivec3` have `/` and `%`** (each component divides as `int` does, checked the same way, as Godot's `Vector2i`/`Vector3i` do), while `ivec4` has none (no integer lane divide on the CPU). Built in `check/src/vectors.rs` (the table of what each type has).
5. **The wrapping operators also work on `u8`** (`a +% 100` wraps at 256), besides `int`, `i64` and the int vectors. Built in `check/src/body/wrap.rs` (one entry of its type list).

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
- Mixed and labelled: syntax pick 3.
- One value of the same type copies it (`vec4(v)`); one vector of the other kind and the same size converts it (`vec4(iv)`, `ivec4(v)`; R10).
- `ivec*` take `int` and `u8` values and int vectors only: `ivec4(1.5)` and `ivec4(n)` with `n: i64` are error V2, as `int` needs `int(x)`.

## Lanes and swizzles

`.x .y .z .w` read and write one lane (`.w` only on 4-lane types). `v.x += 1` and `p.vel.w = 0` and `vs[i].y = 7` work; a lane of a `var` parameter is written in place. Swizzles of 2 to 4 letters, repeats allowed, give a vector of that many lanes of the same kind (`v.xy` is `vec2`, `iv.zyx` is `ivec3`, `v.wzyx` and `v3.xyzz` are `vec4`). Swizzles are read-only, as today.

## Operators

| | `vec2 vec3 vec4` | `ivec2 ivec3` | `ivec4` |
|---|---|---|---|
| `+ - *` | lane by lane | lane by lane, each an `int` check | lane by lane, checked per lane (R6) |
| `/` | lane by lane | lane by lane, each an `int` division check (pick 4) | error V5 |
| `%` | error (today's "`%` doesn't work on vectors") | lane by lane, as `/` (pick 4) | error V6 |
| `-v` | yes | yes, each an `int` check | yes, checked |
| `& \| ^`, `~v` | error ("needs integers") | lane by lane | lane by lane |
| `<< >>` | error | every lane by one plain `int` amount, checked like `int` (R8); `>>` is arithmetic | same |
| `*% +% -%` | error W1 | lane by lane, never checked | same |
| `== !=` | all lanes equal / some lane differs (R12) | same | same |
| `< > <= >=` | error V8 | error V8 | error V8 |

A plain number on either side of a binary operator fills every lane (`2 * v`, `h & 0xFFFF`); an int vector with a `float` is error V7. Compound forms (`+=`, `/=`, `&=`, `>>=`, `*%=` ...) follow their operators, a plain number on the right included (`v *= 2.0`, `h >>= 1`). Precedence: `*%` with `* / %`, `+% -%` with `+ -` (R3).

## Built-in functions

- Lane by lane: `min max clamp abs` on every vector; `floor ceil round sqrt` on float vectors (`round` halves away from zero, as the scalar one); `lerp(a, b, t)` with two float vectors and a number `t`. Any argument may be a plain number, which fills the lanes (pick 4). `abs` of an int vector is checked like `-v`. `sign` takes numbers only.
- Across lanes: `v.sum()` on every vector (lanes add left to right, `((x + y) + z) + w`, R11; an int vector's sum is checked like `+`); `a.dot(b)`, `v.length()`, `v.normalized()` (a zero vector stays zero), `a.distance_to(b)` on float vectors; `cross` on `vec3` only.
- Everything else (`sin`, `pow`, ...) takes numbers only, as today.

## Wrapping operators

`a *% b`, `a +% b`, `a -% b` on `int`, `i64`, `ivec2`, `ivec3`, `ivec4` (R4) and `u8` (pick 5): the result wraps around the type's width, in every build, with no check. Both sides meet at one type as `*` does (`int *% i64` is `i64`). A float is error W1. They make hashes possible without `i64` and masks: `h = (h ^ (h >> 13)) *% 0x5BD1E995`.

## Overflow on int vectors

`ivec4` follows `int`: a checked build stops with "integer overflow on line N of f.kes" when any lane of `+ - *`, `-v` or `abs` overflows, and "shift amount out of range" for a bad shift amount; `--release` wraps. `ivec2`/`ivec3` are structs of ints, so each component gets an ordinary `int` check (division and remainder included). **Cost** (x64 has no overflow flag for lanes; R6): `+`/`-` about 5 more instructions than the bare one; `*` about 12 for two vectors, and about 30 (measured in Task 3b; the first estimate was 16) when one side is a filled constant (`h * 3`: Cranelift's mid-end turns the widened splat into a splat of the widened number, which misses the `pmuldq` rule, R28); `-v` and `abs` about 3; a shift one compare of its scalar amount. Range analysis removes them where it can (R16); `--release` has none.

## Storage, layout and calls (R1, R2)

A `vec4`/`ivec4` is 16 bytes aligned to 16 everywhere it is laid out: struct fields (`struct P { id: u8, pos: vec4 }` is 32 bytes, `pos` at 16), arrays and lists (16-byte stride, 16-aligned blocks), optional payloads (at 16), map entries, closure environments (made, copied and on the stack, R23), interface boxes, globals and stack slots. Codegen loads and stores them **without** Cranelift's `aligned` flag (stack slots are the exception: R29), so a layout that misses the alignment is slower, not a crash. A copy is one 16-byte load and store. Parameters and results use Cranelift's own Windows x64 rules: a vector argument goes by an implicit pointer to a 16-byte copy, a result comes back in XMM0. Natives can't take them (today's error); exporting a function that takes or returns `vec4`, `ivec4`, `ivec2` or `ivec3` to Godot is today's "can't export" error until the follow-up.

## Range analysis (R16-R19)

An `ivec4` local is tracked with **one range for all four lanes** (the widest): a make takes the hull of its lanes, a fill the number's range, a swizzle or a lane read the vector's range; `+ - * & | ^ << >> ~ -v` and the wrapping operators use the scalar rules **for `int`** (R26) on those ranges, which hold lane by lane. Writing one lane widens the range to cover the old one and the new lane (one lane changes, three stay). A vector check is removed only when the widest range proves it for every lane: one check per vector operation, removed whole or kept whole. A vector is never a term of a fact (no `a - b <= c` between vectors, no linear forms), and **a condition on a lane of an `ivec4` (`if v.x < 10`, `@assume(v.x < 10)`) teaches nothing about the vector**. (A lane of an `ivec2`/`ivec3` is a plain `int` after scalar replacement, so a condition on it does teach, as on any `int` field.) Memory loads of vectors (fields, list items) are any `int`, as other loads.

## Report and `@prove` (R20-R22)

A vector operation's check is one item, worded with the lane type: "`a * b` may exceed `int` in a lane (the lanes of `a` can be any `int`; the lanes of `b` are 0 to 65,535)"; a vector shift "may shift by less than 0 or more than 31". `@prove(no_checks)` on vector code holds when every vector operation's check was removed (or it has none: float vectors and wrapping operators never check). `no_panic` counts them the same way; `no_alloc` is unaffected (vectors never allocate; a `[vec4]` literal does, as any array). Nothing else in Round B's contracts changes.

## Errors

Plain sentences that say what to do (exact text in the plan's Global Constraints):

- W1: "`*%` wraps integers; for floats use `*`" (and `+%`/`+`, `-%`/`-`, `*%=`/`*=` ...).
- V1: "`vec4` has 4 components, but these values give 3; give 4, one value for all of them, or none for zeros".
- V2: "`ivec4` holds ints, found float; convert it with `int(...)` first".
- V3: "`ivec3` takes ints and int vectors, found vec2; convert it with `ivec2(...)` first", and the other way "`vec3` takes floats and float vectors, found ivec2; convert it with `vec2(...)` first" (a plain `f64` keeps today's conversion error).
- V4: "labels only work when every component is a number, like `vec4(x: 1, y: 2, z: 3, w: 4)`".
- V5: "`ivec4` has no `/`: the CPU can't divide integer lanes at once; for a power of two use `>>`, or divide each lane, like `v.x / n`".
- V6: "`ivec4` has no `%`: the CPU can't divide integer lanes at once; for a power of two use `&`, or take each lane, like `v.x % n`".
- V7: "`*` can't mix ivec4 and float; convert with `vec4(...)` first".
- V8: "`<` doesn't compare vectors; compare components, like `a.x < b.x`".
- V9: "`>>` shifts every component by one `int`; shift each component on its own for different amounts".
- V10: "`floor` needs floats; convert with `vec4(...)` first". V11: "`length` needs a float vector; convert with `vec4(...)` first". V12: "`cross` needs a vec3; vec4 has no cross product".
- C1 (codegen): "`vec4` and `ivec4` need a CPU with SSE4.1 (every x64 CPU since about 2011); this one doesn't have it, so use `vec2`, `vec3` or plain numbers instead".

## How it is built

- **Checker:** `Ty::Vec4` and `Ty::IVec4` (plain values, not aggregates); `IVEC2 = StructId(2)`, `IVEC3 = StructId(3)`, `BUILTIN_STRUCTS = 4`; `check/src/vectors.rs` is the one table of what each vector type has (`VecKind { dims, int, simd }`, `Tables::vector(ty)`); `vec4`, `ivec2`, `ivec3`, `ivec4` become reserved names. Lanes of `vec4`/`ivec4` are recorded in `Tables::lanes` and are places like struct fields. A hex literal's wrapped value is recorded in `Tables::wrapped_literals: HashMap<ExprId, i64>`, which lowering reads (the parser marks hex literals in a side set of `ExprId`s; the AST's `Int(u64)` is unchanged).
- **KIR types (R25):** `KTy::F32X4`, `KTy::I32X4` answer `false` to `is_int`, `is_float`, `is_signed` and `is_aggregate`; `is_vector()` is true and `lane()` gives `I32`/`F32` (a scalar's `lane()` is itself). Each is one leaf for scalar replacement and promotion; its zero value is `Splat` of a zero number (`leaves::zero`, `lower/remove.rs`'s `zero`).
- **KIR forms:** `VecLit` (four lane values), `Splat`, `Shuffle(e, [u8; 4])`, `LaneOf(e, lane)` (a lane of a value that isn't a place); a place may end in `Proj::Lane(lane)`, and only there. `BinOp::{AddWrap, SubWrap, MulWrap}` on any integer type and `I32X4`. A vector binary operation has two operands of its type (lowering wraps a plain number in `Splat`, compound assignments included), except `Shl`/`Shr`, whose amount is the lane type `I32` (`shift_amount` converts to `ty.lane()`). `Eq`/`Ne` on vectors give one `Bool`. `Cast` converts between `F32X4` and `I32X4`. `Math` takes `F32X4` for `Sqrt Floor Ceil Round Abs Min Max` and `I32X4` for `Min Max`; an int vector's `abs` lowers to `max(v, -v)`, so its check is the `Neg`'s. `ivec2`/`ivec3` lower like `vec2`/`vec3`, component by component.
- **Codegen:** `cl_type` maps them to `F32X4`/`I32X4`; Cranelift handles the calling convention. The vector dispatch is in `arith_expr`/`binary`, which plain and `Proven` nodes both reach (R27). `>>` on `I32X4` is `sshr`. Lanes: `extractlane`/`insertlane` on registers, a 4-byte load or store at `4 * lane` in memory. Swizzles: `bitcast` to `I8X16` (little-endian), `shuffle`, `bitcast` back. Casts: `fcvt_from_sint`, `fcvt_to_sint_sat`. `==`: `icmp`/`fcmp` then `vall_true`. The checks are in `func/simd_check.rs` (R6). Printing extracts the lanes and prints them with the type's pieces; no new runtime function.
- **Range analysis:** `range/vector.rs` evaluates vector forms and calls every interval helper (`width`, `fits`, `settle`, `bit_not`, `shift_safe`, `shift_amount`, `clip`, `of_type`) with `KTy::I32`; `show.rs` names the lane type in check texts (R22, R26).

## Rulings

1. **Alignment 16 in every layout, and no `aligned` flag on vector loads and stores.** Why: in Cranelift 0.116 only a flagged address may become the memory operand of a non-VEX SSE instruction (`XmmMemAligned`), which faults when misaligned; without the flag such a load stays a separate `movups` (with AVX, VEX forms fold it anyway), so a spot that misses the alignment costs a little speed, never a fault, while the 16-byte layout keeps loads on one cache line.
2. **Calls use Cranelift's Windows x64 vector convention (implicit pointer in, XMM0 out); natives and Godot exports refuse them.** Why: Cranelift already implements it for every Kestrel-to-Kestrel call, direct, indirect and through interfaces, and natives would need Rust-side layouts this round doesn't need.
3. **`*%` binds like `*`, `+%` and `-%` like `+`, all left to right.** Why: they are the same operations, so `a +% b *% c` must read like `a + b * c`. The tokens can't break a program: `* %`, `+ %` and `- %` never parsed (`%` can't start an expression), and the lexer tries three-character forms first.
4. **Wrapping operators work on every integer type and every int vector, not on floats** (`u8` is pick 5). Why: wrapping is natural on each of them, and one rule is easier than a list; floats don't wrap.
5. **Wrapping operators never check, in any build, and range analysis gives their result the wrapped range** (exact when the true result fits, else the whole type). Why: that is what they promise; analysing them as `--release` analyses `+` is already sound code.
6. **Per-lane checks are built from lane tricks, not from Cranelift's overflow instructions (which x64 doesn't lower for vectors):** `+` fails when `(a ^ r) & (b ^ r)` has a sign bit set in some lane (`vhigh_bits` not 0), `-` when `(a ^ b) & (a ^ r)` does; `*` takes the 64-bit products of the low and high lane pairs (`imul` of `swiden_low`/`swiden_high`), adds 2^31 to each and fails when any is 2^32 or more after (`ushr` by 32 not all zero, `vany_true`); `-v` fails when some lane is the lowest `int`. The result itself is the plain `iadd`/`isub`/`imul`. Why: each is a few instructions with no branch per lane, and the value stays hoistable, as the scalar checks' design wants.
7. **The check's message is the scalar one ("integer overflow on line N"), with no lane number.** Why: one rule for all integers; the line points at the operation and `kestrel check --report` explains it.
8. **A vector shift takes one plain `int` amount, checked once like a scalar shift; `>>` is arithmetic.** Why: x64 shifts all lanes by one amount in one instruction; per-lane amounts need AVX2 rules Cranelift 0.116 doesn't have (error V9); `>>` on `int` is arithmetic and range analysis assumes it.
9. **No `/` or `%` on `ivec4` (V5, V6), and no `%` on float vectors; `ivec2`/`ivec3` have both (pick 4).** Why: the CPU has no integer lane divide, so on `ivec4` they would silently be four scalar divides, while `ivec2`/`ivec3` are scalar structs already; float `%` was never on vectors.
10. **`ivec4(v)` truncates toward zero and saturates, NaN giving 0, exactly as `int(x)` does today (`fcvt_to_sint_sat`); `vec4(iv)` rounds to the nearest float. Neither is checked.** Why: one rule with the scalar conversion; `int(x)` of a float has never been a checked narrowing.
11. **Across-lane sums add left to right, `((x + y) + z) + w`.** Why: float results then equal the written-out sum, which tests and the scalar versions of kernels rely on.
12. **`a == b` is true when every lane is equal (a NaN lane is unequal, as for floats); `a != b` is its opposite.** Why: the proposal's rule, and `!=` must be `not ==`.
13. **Vectors print like the struct vectors do today: `vec4(x: 1, y: 2, z: 3, w: 4)`, `ivec2(x: 1, y: -2)`, in interpolation too.** Why: `vec3` already prints this way; one format.
14. **`ivec2`/`ivec3` are `StructId(2)` and `StructId(3)`; `vec2`/`vec3` stay 0 and 1.** Why: the Godot bridge and `export.rs` key on 0 and 1.
15. **SSE4.1 is required for a program that uses `vec4` or `ivec4` (error C1 at build, tested with an ISA that has it turned off).** Why: `pmulld`, `pminsd`, `roundps` and `ptest` are SSE4.1, and without it Cranelift lowers the vector rounding functions to libcalls Kestrel neither provides nor tests; Windows 11 24H2 needs SSE4.2, so it never fires on a supported machine.
16. **Range analysis tracks an `ivec4` local with one interval (the hull of its lanes); writing one lane joins the old range with the new value's.** Why: the proposal's rule ("the widest"), and the join is what makes a one-lane write sound.
17. **A vector is never a fact term and has no linear form; conditions on `ivec4` lanes teach nothing.** Why: `v.x < 10` says nothing about `v.y`, and facts between vectors would need per-lane reasoning for little gain. (`ivec2`/`ivec3` lanes are plain `int`s after scalar replacement and do teach.)
18. **One check per vector operation, removed only when all four lanes are proven.** Why: it is one instruction sequence; there is nothing per lane to remove.
19. **Float vectors are not analysed** (as floats today). Why: they have no checks.
20. **`@prove(no_checks)` means no vector check is left in the region, counted as one item per operation.** Why: contracts read the same sites as the report; a vector check is a site like any other.
21. **`@assume` may name an `ivec4` lane (`@assume(v.x < 10)`), but it teaches nothing, so it gets the "removes no check" warning; on an `ivec2`/`ivec3` lane it teaches as on an `int`.** Why: the checker's form rule already accepts fields, and rejecting lanes would need a special message for no gain.
22. **Report texts for vector checks use the lane type: "may exceed `int` in a lane", "the lanes of `a` are 0 to 99", a shift "more than 31"; `type_name` gives the vector's own name only where a type is named as a type.** Why: the reader must see that one range stands for four lanes, and the limits are the lane's.
23. **Closure environment blocks become 16-aligned (were 8), for every closure, at all three places that make one: `heap_fn_into`, `stack_fn_into` and the copy `env_dup`; `EnvLayout`'s doc says so.** Why: simpler than per-environment alignment; at most 8 bytes of arena padding per environment.
24. **The frozen voxel benchmark doesn't change.** Why: a 32-bit wrapping hash is a different world and checksum; the measurement uses new kernels (plan, Task 6), and the no-rules engine is where a vector hash belongs.
25. **`F32X4`/`I32X4` answer `false` to `is_int`, `is_float`, `is_signed` and `is_aggregate`, and every question about lanes goes through `lane()`.** Why: the scalar paths (linear forms in `eval.rs`, refinement in `refine.rs`/`terms.rs`, `width`, `bit_not`, codegen's `shift`/`bits`) branch on those answers, and a vector that slipped through them as an `int` would get a 64-bit width, an unsigned `~`, a logical `>>` or a linear form, each removing a check that can fail.
26. **Range analysis calls every interval helper with `I32` for an `ivec4`.** Why: the helpers default unknown types to 64 bits and unsigned; with `I32` they give exactly the scalar `int` answers, which hold lane by lane.
27. **Codegen dispatches vectors inside `arith_expr`/`binary`, which `Proven` nodes also reach, and the checks go through `fail_if` (so `Emit`, `Skip` and `Verify` all work).** Why: a dispatch only in `expr()` would send a proven vector `-v` or `>>` down the scalar path (an `iconst` of a vector type, or a logical shift).
28. **The `*` check costs about 30 instructions when one side is a filled constant (measured in Task 3b; the estimate was 16), and that is accepted this round** (a follow-up may check `a * c` by comparing `a`'s lanes with `MIN / c` and `MAX / c`). Why: range analysis removes most constant products (masked hash values), and a second check shape is more to get wrong.
29. **Stack slots are exempt from the no-`aligned` rule:** `stack_store`/`stack_load` keep Cranelift's own flags. Why: a slot of `size_align(ty)` is 16-aligned (Cranelift honours `align_shift`, and the slot area is 16-aligned).

## Recheck after a Cranelift upgrade

Another branch is moving Cranelift to the wasmtime 50+ line (about 0.137). After it lands, recheck each:

- The `pmuldq` rule for `imul(swiden_low/high, swiden_low/high)` used by the `*` check (R6), and the mid-end rewrite of `swiden_low(splat)` behind R28: dump an `ivec4` product.
- Windows fastcall's implicit pointer for vector arguments (R2): rerun the ABI test and its closure and interface variants.
- "Slower, not a fault" without `aligned` (R1): `XmmMemAligned` still accepting only flagged addresses under the new assembler; `MemFlags::trusted()` still notrap plus aligned.
- The `fcvt_to_sint_sat` `F32X4` lowering's NaN-to-0 and saturation (R10).
- The vector rounding libcall fallback behind C1 (R15), and the `has_sse41` flag through the ISA-flags API.
- The verifier's rule for a lane-count-changing `bitcast` (needs little-endian `MemFlags`) in swizzles.
- Hoisting of check values out of loops (the LICM workarounds of d21201f).
- Text-matching dump tests: `trap user2`, `explicit_slot`, `f32x4`, and mnemonics like `pmulld`/`psrad`.
- Spill-slot size and alignment for XMM vectors (Cranelift-internal; watch frame layout).

## Not this round

Lane-by-lane comparisons and `select`; 8-wide vectors; `f64`/`i64` vectors; auto-vectorisation; `/` and `%` on `ivec4`; per-lane shift amounts; Godot exports of `vec4`, `ivec4`, `ivec2`, `ivec3` (`Vector4`, `Vector4i`, `Vector2i`, `Vector3i`); natives taking `vec4`/`ivec4`; `sign` on vectors; a cheaper `*` check for a constant (R28).

## As built (2026-10-24)

Built on `main` as the plan `docs/superpowers/plans/2026-10-21-simd-vectors.md`, subagent-driven in eight tasks and these commits: Task 1, the wrapping operators and hex literals, 1a741a1..57b980a (db4dc14, then two fix rounds 2dcf7a5 and 57b980a); Task 2, one rule for every vector and `ivec2`/`ivec3`, 22c8ce9 and its fix round 8854d15; Task 3a, `vec4`, 79a6f67; Task 3b, `ivec4` and its per-lane checks, f5b0652; Task 4, range analysis, 7656dbe; Task 5, the built-ins, b2b4535; Task 6, the measurement, 64d7d6e; Task 7, this documentation. Every task ran only focused tests in light mode (`CARGO_BUILD_JOBS=2`, `--test-threads=2`, `-p <crate>` and `KESTREL_TEST_ONLY` filters); **the full `cargo test --workspace --no-fail-fast`, clippy in all three workspaces, and Task 4's every-program proof sweep are owed** until the user says the machine is free. The final whole-round review (Opus) and its fix wave are the next step.

### The five syntax picks are confirmed

The controller took the user's "lets continue", said in answer to "take my five syntax picks and start the build?", as a yes to all five. They are built as written, so they are not waiting for anyone. Each is in one place, so a change is cheap: pick 1 (`*%=` and the others) is three entries in the lexer's operator table (`syntax/src/lexer.rs`) and `wrap_compound` in `check/src/body/wrap.rs`; pick 2 (hex over the whole width, anywhere) is `hex_literal` and `retype_hex` (`check/src/body/expr.rs`, `ops.rs`) with `Tables::wrapped_literals`; pick 3 (shader-style mixed and labelled makes) is `check/src/body/vec_make.rs`; pick 4 (one rule for every vector, `/` and `%` on `ivec2`/`ivec3`) is the table in `check/src/vectors.rs`; pick 5 (`u8` wrapping) is one entry of `wraps()` in `check/src/body/wrap.rs`.

### What differs from the spec

1. **Hex literals are marked by their own token, not a flag.** `Tok::Hex(u64)` replaces `Tok::Int` for a hex literal, and the parser lists the `ExprId`s in `Program::hex_literals`. A `hex` flag on `Token` made it 8 bytes bigger and pushed the 1000-level nesting test over the 2 MiB test-thread stack (the compiler needed about 2.03 MB, the flag version 2.10 MB). `ExprKind::Int(u64)` is unchanged.
2. **"A hex literal takes its type from where it is" means the operation's type.** The spec's wording left `0xFFFFFFFF + w` and `w + 0xFFFFFFFF` free to differ (`w: i64`: 2 and 4294967298, an order-dependent result for a commutative operator). The rule built is one sentence, **a hex literal is read at the type it meets**: a bare hex literal next to a wider integer is re-checked at that type, in both orders (`retype_hex`); next to a float it is the float it spells (`0xFFFFFFFF * 1.5` is 6442451000 and `0xFFFFFFFF * d`, `d: f64`, is 6442450942.5 in both orders), so **`0xFFFFFFFF < d` with `d = 1.5` is false** (it compares 4294967295.0, not -1), like `float(0xFFFFFFFF)` and `let f: float = 0xFFFFFFFF`. A *variable* holding a wrapped hex stays an `int` (`let x = 0xFFFFFFFF; x * 1.5` is -1.5). `-0x80000000` (and `-0x8000000000000000` for `i64`) is a compile error ("write `0x80000000`") like the decimal form, not a program that traps.
3. **`vector_ty` returns an `Option<Ty>`** (Task 2), since before `vec4` existed a 4-letter swizzle had to be able to find no type. Once the `vec4` and `ivec4` rows were in it always finds one.
4. **Bit-operator compound forms on float vectors are now refused** (`&=`, `|=`, `<<=` on a `vec2` used to pass the checker and reach KIR as float bit operations); the error is "`&=` needs integers, found vec2".
5. **A vector make's values are checked with the lane type as their hint**, as `check_construct` always did, so a vector-valued `if` or `match` written directly inside a mixed make (`vec3(if c { a } else { b }, 1)`) is refused ("expected float, found vec2"); a `let` first works. Kept to leave old behaviour alone.
6. **Lanes of a captured `vec4`.** A closure that reads `k.x` captures all of `k` (16 bytes); `note_capture` only extends a read through struct fields, so the lane is read from the capture's place. Simpler than lane steps in capture paths, and just as correct.
7. **V1 wins over V3 when the counts don't add up** (Task 3b). `ivec3(vec4(1))` used to say "convert it with `ivec4(...)`", which doesn't help (`ivec3(ivec4(..))` is still 4 components); it now says "`ivec3` has 3 components, but these values give 4". V3 still fires when the count is right.
8. **The report writes a number that fills a vector as the number**: `a * 3`, not `a * ivec4(3)`. A make on its own (`vec4(2.5)`) still reads as a make. The evidence for a hull reads "the lanes of `a` are 0 to 99" (one range, four lanes).
9. **The `*` check costs about 30 instructions with a filled constant, not about 16** (R6, R28 corrected above). The `pmuldq` rule fires for two vectors (14 instructions); with a constant the mid-end folds the widened splat and the check falls back to `pmuludq` sequences. Accepted as R28 says; the cheaper check is in the follow-ups file.
10. **Range analysis decides a vector shift by a known amount** (`h >> 1` is removed and passes `@prove(no_checks)`), and tracks a `var v: ivec4` parameter as a local that starts at any `int` (sound: its lane writes join). `min`, `max`, `clamp` and `abs` carry lane ranges (`abs` lowers to `max(v, -v)`, so its only check is the `Neg`'s; the range of `abs(h)` has a low end of -2,147,483,647 rather than 0, which loses precision, not soundness).
11. **Evaluation order of a vector's two sides** (Task 2's fix round): `vec_combine` copies the left operand into a temporary when the right side may write (a call or a write to a place), so `n - g(var n)` on `ivec2` reads `n` first, as the scalar does. This was a miscompile already present for `vec2`/`vec3`. `vec4`/`ivec4` need no spill: codegen loads the left operand into an SSA value before the right side runs, and `vec4_make_order.kes` covers it.
12. **Constants.** Whether `<const>.<name>` is a constant is decided by the base's type (a vector swizzle or lane), not by the name, so a user struct field called `xy` is refused in a constant as every user field always was.

### Rulings made while building

Each with what it costs if wrong (the ledger kept the same lines).

- **The five picks are confirmed** (above). If wrong: each is in one named place, so changing one is small.
- **Build on Cranelift 0.116.1 now**, and rebase onto the upgrade (branch `claude/xenodochial-ardinghelli-291b23`, 0.137 line) later using the "Recheck after a Cranelift upgrade" list; waiting would have stalled the build on a session with no commits yet. If wrong: some rework of vector lowering after the upgrade.
- **The Opus plan review served as the preflight conflict scan** (it checked task seams and executability), with no separate table. If wrong: a seam conflict would surface in a task review (none did).
- **Read a hex literal operand at the operation's type, so both orders agree** (Task 1, review minor 1), instead of an error. If wrong: the user may prefer an error; cheap to switch.
- **`-0x80000000` is a compile error**, as the decimal form is. If wrong: nothing, it is stricter.
- **A bare hex literal next to a float is the float it spells, in both orders** (Task 1, second fix round): `0xFFFFFFFF * 1.5` is 6442451000, like `float(0xFFFFFFFF)`. If wrong: the literal next to a float would give the unsigned reading; the user could choose -1.
- **Fix the vector evaluation-order miscompile now** (Task 2's second Important), not just log it, since `vec4` and `ivec4` reuse `vec_combine`. If wrong: small extra copies in rare programs.
- **`<const>.<name>` is a constant by the base's type, not the name** (Task 2's first Important). If wrong: none.
- **No `may_write` spill for `vec4`/`ivec4`.** If wrong: an order bug, covered by `vec4_make_order.kes`.

### What the reviews found

- **The Opus plan review** found 1 Critical (its C1: the lane-type semantics, now R25-R27: `F32X4`/`I32X4` must answer `false` to `is_int` and friends, and range analysis must call every interval helper with `I32`), 9 Important and 15 Minor; all but M14 (a cheaper `*` check, now R28) were folded in.
- **Task 1** (Opus review): 1 Important (`literal_as` ignored `wrapped_literals`, so `0xFFFFFFFF * 1.5` gave 6442451000 instead of -1.5) and 4 Minor; the fix for the Minor on operand order found a new order dependence next to a float, fixed in round 2. Clean after 2 rounds.
- **Task 2** (Opus review): 2 Important and 5 Minor. I1: a stack overflow in the checker on a default or constant that loops through a swizzle-named field (`default_cycles::builds` had no `Field` arm). I2: the evaluation-order miscompile of item 11. Clean after 1 round.
- **Task 3a** (Opus): approved, no Critical or Important; the reviewer argued the alignment rule (every vector load and store but a stack slot's goes through `mem_flags` with no `aligned` flag, so no SSE instruction can take a misaligned memory operand and fault) and the Windows fastcall implicit pointer.
- **Task 3b** (Opus): approved; about 3,500 differential runs of each `ivec4` operation against `int` lane by lane, checked and `--release`, found no mismatch. The implementer's 15 mutations of the checks were all caught.
- **Task 4** (Opus): approved; 31 counterexample programs run under `KESTREL_CHECK_PROOFS` found no unsound removal; the implementer's 11 mutations of the range rules were caught (one, a vector made a fact term, changes nothing because no vector is ever picked: the node guard is a second line of defence).
- **Task 5** (Sonnet, resumed after a usage-limit cut): approved; the built-ins match the scalar ones lane by lane (NaN, infinities, -0, round ties) in JIT, `--release` and exe.
- **Task 6** (Sonnet): approved; the checksums agree to the last `f64` digit across all eight runs, and the Rust SSE version is a fair yardstick (SSE only, non-VEX).

### What was measured (Task 6)

On a 2D value noise, `--release` (checked is within 0.98-1.03 of it), ns per sample, one thread, on a busy machine (a Python job and a game kept the CPU at 60-90%, so absolute times are probably 20-40% above a quiet one): scalar Kestrel 11.57, Rust scalar 10.29 (Kestrel 1.12 times Rust's time); four samples at once in `vec4`/`ivec4` 3.67 against hand-written Rust SSE4.1 3.58, which is **3.15-3.2 times the scalar Kestrel and level with Rust's SSE (within the noise; the Rust version is SSE only, not AVX)**, and 2.8 times Rust's scalar; four corners of one sample at once 7.89, about 1.4-1.7 times (noisy; it waits on one hash chain where the scalar loop overlaps four). A checked build costs nothing measurable here because range analysis removes every lane check (`--report`: 46 checks, 45 removed in the wide kernel; the one left is a scalar in the loop head). With every check kept the wide kernel takes 1.34 times as long, which overstates the lane checks (most of the extra is constant-false branches Cranelift keeps). The small gaps to Rust are listed in the follow-ups file. **A quiet rerun is owed:** `py measure.py --rounds 9 --only noise --threads 1` in `kestrel/bench/kernels`. Tables and the dump: `kestrel/bench/kernels/results.md`, "SIMD vectors".

### What is owed

- **The final whole-round review (Opus) and its fix wave.** Known items for the wave, from the ledger: a program `ivec4_sum_overflow.kes` and a `range_simd` test (`h.sum()` has 3 sites; `(h & 255).sum() * 2000000` has none left); an order-sensitive float `sum`/`dot` test (`vec4(100000000.0, 1, -100000000.0, 1)` sums to 1); `simd_method`'s `once(v)` spills plain locals, so the report prints `(...).x` (skip the spill for a local read); the vacuous round case in `vector_math_makes_no_calls` (it asserts only `f32x4`). Deferred minors, small and listed in the follow-ups file: `s +%= t` on a string reports two errors; a hex literal wider than 32 bits on the left of an `i64` operation errors with no hint (as a decimal does); `c + 0xFFFFFFFF` with `c: u8` errors but `0xFFFFFFFF + c` gives 2; V7's advice for an `f64` operand still fails; abs's range precision.
- **The full suite, clippy and Task 4's proof sweep**: `cargo test --workspace --no-fail-fast`, `cargo clippy --workspace --all-targets -- -D warnings` (and in `kestrel/godot` and `kestrel/bench/kernels`), and the every-program proof check, plus the frozen voxel world's `kestrel check --report` (checked and `--release`) compared byte for byte with the one before Task 1 (it contains no vector code). Until the user says the machine is free.
- **The quiet benchmark rerun** above.
- **The Cranelift upgrade** (branch `claude/xenodochial-ardinghelli-291b23`, commit 8ab1f60) and the "Recheck after a Cranelift upgrade" list for the vector code.
