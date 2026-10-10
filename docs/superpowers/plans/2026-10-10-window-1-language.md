# Window plan 1: math, random numbers, strings anywhere

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** the language pieces the window needs that don't need a window: global math (`sqrt`, `sin`, `min`, `lerp`, `PI`...), GDScript's vector methods, random numbers, and string interpolation anywhere (`let s = "hp {hp}"`).

**Architecture:**
- **Checker:** each built-in function becomes `Res::Builtin(Builtin)`, typed in a new `check/src/body/builtins.rs`. Interpolated strings are `string` values.
- **KIR:** float operations become `ExprKind::Math`, random numbers `RandFloat` / `RandInt` / `Stmt::RandSeed`, and interpolated strings `ExprKind::Format`. Everything else (integer `abs`/`min`/`max`/`sign`, `clamp`, `lerp`, the vector methods) lowers to existing nodes.
- **Codegen:** `sqrt floor ceil abs min max` (floats) and `round` use Cranelift instructions. The trigonometric and exponential functions call one runtime function, `kestrel_rt_math(op, a, b)`. A `Format` writes its parts with the print code into a runtime string buffer, then copies the bytes into the current arena.
- **Strings become heap values**, like arrays. Copying a string place copies its bytes into the current arena, and the arena escape check covers strings. String literals stay in read-only data, belonging to no arena.

**Tech Stack:** Rust workspace `kestrel/` (syntax, check, kir, codegen on Cranelift 0.116, runtime staticlib, cli). No new crates.

**Spec:** `docs/superpowers/specs/2026-10-10-window-design.md`, sections "Global math and random numbers" and "String interpolation anywhere". Read them first.

## Global Constraints

- Source files stay under about 400 lines. New code goes in:
  - `check/src/builtins.rs` (the `Builtin` enum) and `check/src/body/builtins.rs` (typing);
  - `kir/src/lower/builtins.rs`;
  - `codegen/src/func/math.rs`;
  - `runtime/src/math.rs` and `runtime/src/print.rs`.
  - `kir/src/ir.rs` is at 381 lines, so Task 2 first moves the four layout structs into `kir/src/layout.rs`.
- Error messages are plain sentences that say what to do; use the messages in this plan verbatim.
- From `kestrel/`, run `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`. Also run clippy in `kestrel/godot` at the end of each task that touches codegen or the runtime.
- Program tests: `tests/programs/*.kes` with `// expect:` lines (JIT and exe). Error tests: `tests/errors/*.kes` with `// error <line>: <message>`; put the expectations at the end of the file so they don't shift line numbers.
- Write files containing `\n` or `\\` with the Write/Edit tools, not shell heredocs.
- Floats print with Rust's shortest form: `2.0` prints `2`, `0.6` prints `0.6`, f32 `PI` prints `3.1415927`.
- The flaky `relocated` CLI test sometimes fails with "file in use" (OneDrive). Re-run it.

---

### Task 1: Runtime: math, random numbers, string building

**Files:**
- Create: `runtime/src/math.rs`
- Modify: `runtime/src/lib.rs` (`pub mod math;`), `runtime/src/print.rs` (string building), `codegen/src/rt.rs` (declare the six new functions in `RtFuncs` and list them in `jit_symbols`)

**Interfaces (Produces):**
- `kestrel_runtime::math` op codes, `pub const`: `SIN = 1, COS = 2, TAN = 3, ASIN = 4, ACOS = 5, ATAN = 6, ATAN2 = 7, EXP = 8, LOG = 9, POW = 10` (all `i32`).
- `#[no_mangle] pub extern "C" fn kestrel_rt_math(op: i32, a: f64, b: f64) -> f64`: `b` is used only by `ATAN2` (`a.atan2(b)`, so `a` is y) and `POW` (`a.powf(b)`). An unknown op gives NaN.
- `#[no_mangle] pub extern "C" fn kestrel_rt_rand_float() -> f64`: in `[0, 1)`.
- `#[no_mangle] pub extern "C" fn kestrel_rt_rand_int(lo: i64, hi: i64) -> i64`: between `lo` and `hi`, both included. Swaps them if `lo > hi`.
- `#[no_mangle] pub extern "C" fn kestrel_rt_rand_seed(seed: i64)`.
- `#[no_mangle] pub extern "C" fn kestrel_rt_str_begin()`: from now on, this thread's print output goes into a new string buffer (buffers nest).
- `#[no_mangle] pub unsafe extern "C" fn kestrel_rt_str_end(out: *mut u8)`: ends the innermost buffer. It copies the bytes into the current arena and writes the pointer at `out` and the length (`i64`) at `out + 8`. An empty buffer writes a null pointer and 0.
- `RtFuncs` fields:
  - `math: decl("kestrel_rt_math", &[I32, F64, F64], Some(F64))`
  - `rand_float: decl("kestrel_rt_rand_float", &[], Some(F64))`
  - `rand_int: decl("kestrel_rt_rand_int", &[I64, I64], Some(I64))`
  - `rand_seed: decl("kestrel_rt_rand_seed", &[I64], None)`
  - `str_begin: decl("kestrel_rt_str_begin", &[], None)`
  - `str_end: decl("kestrel_rt_str_end", &[ptr], None)`

**The generator:** SplitMix64 in a `thread_local! { static STATE: Cell<u64> }`, seeded on first use from the clock: `SystemTime::now()` nanoseconds, mixed with the address of a local variable. Godot 4 seeds from the clock the same way. `rand_seed(n)` sets the state to `n as u64`.

```rust
fn next(state: &Cell<u64>) -> u64 {
    let s = state.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
    state.set(s);
    let mut z = s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
```

- `rand_float`: `(next >> 11) as f64 * (1.0 / (1u64 << 53) as f64)`.
- `rand_int`: `span = hi.wrapping_sub(lo) as u64 .wrapping_add(1)`. When `span == 0` (the full `i64` range), return `next as i64`. Otherwise return `lo.wrapping_add(((next as u128 * span as u128) >> 64) as i64)`.

**String building** in `print.rs`:
- `thread_local! { static BUILD: RefCell<Vec<Vec<u8>>> }`.
- `write_fmt` and `write_bytes` append to the top buffer when the stack isn't empty, and go to `OUT` otherwise.
- `str_end` pops the top buffer and copies it with `crate::arena::kestrel_rt_alloc(len, 1, 1, 0)`.

- [ ] **Step 1: Failing unit tests.** In `math.rs`:
  - `math_ops`: `kestrel_rt_math(SIN, 0.0, 0.0) == 0.0`, `COS` of 0 is 1, `ATAN2` of `(0.0, 1.0)` is 0, `POW` of `(2.0, 10.0)` is 1024, `EXP` of 0 is 1, `LOG` of 1 is 0, op 99 is NaN.
  - `seeded_sequences_repeat`: seed 7, take 5 ints in `1..=6` and 5 floats; seed 7 again and get the same values.
  - `ints_cover_their_range`: seed 1; 1000 calls of `rand_int(1, 6)` stay in range and hit all six values. `rand_int(6, 1)` is in range too. `rand_int(i64::MIN, i64::MAX)` doesn't panic.
  - `floats_stay_below_one`: 10_000 calls are in `[0, 1)`.

  In `print.rs`, `strings_nest_and_land_in_the_arena`, on a spawned thread:
  1. `capture_output()`.
  2. `str_begin`; `print_i64(4)`; `str_begin`; `print_str` of `"x"`; `str_end(inner)`; `print_i64(2)`; `str_end(outer)`.
  3. Read back both `(ptr, len)` pairs: the outer string is `"42"` and the inner is `"x"`.
  4. `take_output()` is empty: nothing leaked to the capture.
  5. `str_begin` then `str_end` gives a null pointer and length 0.
- [ ] **Step 2: Run** `cargo test -p kestrel-runtime` and see them fail to compile.
- [ ] **Step 3: Implement** `math.rs` and the `print.rs` changes, and add the six functions to `RtFuncs::declare` and `jit_symbols` in `codegen/src/rt.rs`.
- [ ] **Step 4: Run** `cargo test --workspace` and clippy (both workspaces). Commit "Add runtime math, random numbers and string building".

### Task 2: Interpolated strings anywhere (strings become heap values)

**Files:**
- Modify:
  - `kir/src/ir.rs`: `ExprKind::Format`. First move `EnumLayout`, `VariantLayout`, `StructLayout` and `FieldLayout` into `kir/src/layout.rs`, re-exported from `lib.rs` exactly as before, so `ir.rs` stays under 400 lines.
  - `check/src/body/expr.rs`: the `Interp` arm.
  - `check/src/tables.rs`: `has_heap(Str)` is true.
  - `check/src/body/regions.rs`: a string literal has region 0.
  - `kir/src/lower/expr.rs`: `Interp` lowers to `Format`.
  - `kir/src/lower/mod.rs`: `print_arg` flattens nested interpolations.
  - `kir/src/lower/helpers.rs`: `has_heap(Str)` is true; `fixup` of a `Str` place is `Stmt::Dup(place)`.
  - `kir/src/visit.rs`, `kir/src/rename.rs`, `kir/src/inline.rs`: `Format` arms, reusing their `PrintArg` helpers.
  - `codegen/src/func/print.rs`: split `print` into `pieces` / `eval_pieces` / `write_pieces`.
  - `codegen/src/func/aggregate.rs`: `Format` in `lower_into`.
  - `codegen/src/func/collections.rs`: `elem_size_align(Str)` is `(1, 1)`.
- Test: `tests/programs/string_values.kes` (new), `tests/errors/strings.kes` (drop the old error), `tests/errors/string_escape.kes` (new)

**Interfaces (Produces):**
- `kir::ExprKind::Format(Vec<PrintArg>)`, with `ty` `KTy::Str`. Its parts are only `PrintArg::Value` and `PrintArg::Str`: lowering flattens a nested interpolation into its parent. It is built in the current arena.
- `codegen` `FuncLowerer::eval_pieces(&[(&PrintArg, bool)]) -> Result<Vec<Option<Value>>, String>` and `write_pieces(pieces, vals) -> Result<(), String>`. `print` becomes `pieces` + `eval_pieces` + `write_pieces` + the newline.

**Rules:**
- **Checker:** `ExprKind::Interp` outside `print` types as `Ty::Str`, and each part is checked with `check_printable` (the same as inside `print`). The old error "string interpolation only works directly inside `print(...)` for now" goes away.
- **Strings are heap values.** In both `has_heap` functions, `Str` is true. The comment "Strings are immutable, so copies share them and they don't count" changes to: "Strings may live in an arena (interpolation builds them there), so copies copy their bytes, like arrays."
- **Regions:** in `region()`, add `ExprKind::Str(_) => 0` before the default arm. Literals live in read-only data and belong to no arena.
- **Codegen of `Format`:**
  1. Evaluate the pieces first, exactly like `print`, so that output from a call inside a part doesn't land in the string.
  2. `str_begin`.
  3. Write the pieces.
  4. `str_end(dest)`.

- [ ] **Step 1: Failing tests.** `tests/programs/string_values.kes`:
```
// expect: hp 3/10
// expect: ["a0", "a1", "a2"]
// expect: Ann 2: 7 hp
// expect: true
// expect: 5
// expect: temp 3
// expect: v = vec2(x: 1.5, y: 2)
// expect: n7
// expect: start
struct Player {
    name: string
    hp: int
}

label(p: Player) -> string {
    "{p.name}: {p.hp} hp"
}

pure tag(n: int) -> string {
    "n{n}"
}

main() {
    let hp = 3
    let s = "hp {hp}/10"
    print(s)
    var names = list[string]()
    for i from 0 to 3 {
        names.add("a{i}")
    }
    print(names)
    let p = Player(name: "Ann {1 + 1}", hp: 7)
    print(label(p))
    print("{s}!" == "hp 3/10!")
    var scores = map[string, int]()
    scores["p{hp}"] = 5
    print(scores.get("p3", 0))
    arena {
        let tmp = "temp {hp}"
        print(tmp)
    }
    let v = vec2(1.5, 2)
    print("v = {v}")
    print(tag(7))
    var kept = "start"
    let copy = kept
    kept = "changed"
    print(copy)
}
```
`tests/errors/string_escape.kes`:
```
main() {
    var s = "start"
    let n = 1
    arena {
        s = "n {n}"
    }
    print(s)
}
// error 5: this memory is freed at the end of the `arena { }` block, but `s` lives longer
```
In `tests/errors/strings.kes`, delete the `// error 8: string interpolation ...` line; line 8 (`let s = "hp {hp}"`) is now valid.
- [ ] **Step 2: Run** `cargo test -p kestrel-cli` and see both fail.
- [ ] **Step 3: Implement** the layout move, then the rules above.
- [ ] **Step 4: Run** `cargo test --workspace` and clippy (both workspaces).
  - Existing string, map and struct tests must still pass: copying a string now copies its bytes, and nothing else changes.
  - Then measure `kestrel/bench/maps` (string keys) before and after, median of 5, and note the change in its `results.md`. String keys may now copy on insert.
- [ ] **Step 5: Commit** "Build interpolated strings anywhere".

### Task 3: Float math builtins and `PI` / `TAU`

**Files:**
- Create:
  - `check/src/builtins.rs`: `pub enum Builtin` and its tables.
  - `check/src/body/builtins.rs`: typing.
  - `kir/src/lower/builtins.rs`
  - `codegen/src/func/math.rs`
- Modify:
  - `check/src/lib.rs` (export `Builtin`)
  - `check/src/tables.rs` (`Res::Builtin(Builtin)`)
  - `check/src/collect.rs`: add every builtin name to `BUILTINS`, so they are reserved for items, locals and parameters with the existing message "`min` is a built-in name; pick another".
  - `check/src/body/call.rs`: dispatch to builtins before the user-function lookup; `check_discarded_call`.
  - `check/src/body/expr.rs`: `PI` and `TAU` in `ident`.
  - `check/src/body/mod.rs`: `is_const_expr` allows calls to pure builtins with constant arguments.
  - `kir/src/ir.rs`: `MathOp`, `ExprKind::Math`.
  - `kir/src/lower/expr.rs`: dispatch.
  - `kir/src/visit.rs`, `rename.rs`, `inline.rs`.
  - `codegen/src/func/expr.rs`: dispatch.
- Test: `tests/programs/math.kes`, `tests/errors/math.kes`, unit tests in `check/tests/builtins.rs`

**Interfaces (Produces):**
```rust
// check/src/builtins.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    Sqrt, Sin, Cos, Tan, Asin, Acos, Atan, Atan2, Exp, Log, Pow, Floor, Ceil, Round,
    Abs, Sign, Min, Max, Clamp, Lerp,
    Randf, RandfRange, RandiRange, RandSeed,
    Pi, Tau,
    Length, Normalized, Dot, DistanceTo,
}
impl Builtin {
    /// A global function or constant by name (not the vector methods).
    pub fn global(name: &str) -> Option<Builtin>;
    /// A `vec2` / `vec3` method by name.
    pub fn vector_method(name: &str) -> Option<Builtin>;
    pub fn name(self) -> &'static str;          // "sqrt", "distance_to", "PI"...
    pub fn is_pure(self) -> bool;               // false only for the four random ones
    /// Every global name, for `BUILTINS`.
    pub const GLOBAL_NAMES: &'static [&'static str];
}
```
```rust
// kir/src/ir.rs
/// Float operations: the operands and the result all have the expression's type (F32 or F64).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathOp { Sqrt, Floor, Ceil, Round, Abs, Min, Max, Sin, Cos, Tan, Asin, Acos, Atan, Atan2, Exp, Log, Pow }
// in ExprKind:
    /// A float operation (see `MathOp`); integer `abs`/`min`/`max` lower to `If`.
    Math(MathOp, Vec<Expr>),
```

**Typing (this task: the float functions and constants):**
- **Arity:** `sqrt sin cos tan asin acos atan exp log floor ceil round` take one value; `atan2 pow` take two.
  - A wrong count: "`sqrt` takes one value", "`atan2` takes two values" (later tasks: "`clamp` takes three values", "`randf` takes no values").
  - Labelled or `var` arguments: "built-in functions take plain values, not labels or `var`".
- **Argument types:** each argument is checked with no hint.
  - An integer type is coerced to `float` (`self.coerce(arg, ty, Ty::Float)`).
  - `float` and `f64` stay as they are.
  - With two arguments, both meet at `Ty::common` after that, so `pow(2, x64)` is `f64`.
  - Anything else: "`sqrt` needs a number here, found string" (on the argument's span).
- **Result:** the (common) argument type.
- **`PI` and `TAU`** in `ident`, after locals and constants: `Res::Builtin(Pi|Tau)`. The type is `F64` when `want == Some(Ty::F64)`, else `Float`. Lowering emits `ExprKind::Float(std::f64::consts::PI)` (or `TAU`) at that type.
- **Purity:** these are pure. `check_discarded_call` reports a pure builtin's unused result: "the result of `sqrt` is unused, so this line does nothing".

**Codegen** (`math.rs`): evaluate the arguments in order.
- `Sqrt Floor Ceil Abs Min Max` are `sqrt floor ceil fabs fmin fmax`.
- `Round` rounds halves away from zero:
  ```rust
  let t = ins.trunc(x); let d = ins.fsub(x, t); let ad = ins.fabs(d);
  let half = fconst(ty, 0.5); let ge = ins.fcmp(FloatCC::GreaterThanOrEqual, ad, half);
  let one = ins.fcopysign(fconst(ty, 1.0), x); let up = ins.fadd(t, one);
  ins.select(ge, up, t)
  ```
- The rest promote an `F32` operand to `f64` (`fpromote`) and call `rt.math` with the op code from `kestrel_runtime::math` (a missing second operand is `f64const(0.0)`). The result is demoted (`fdemote`) back to `F32`.

- [ ] **Step 1: Failing tests.** `tests/programs/math.kes`:
```
// expect: 4 2 3 3 -3 0
// expect: 1 0 0 1024 1 0
// expect: 3.1415927 6.2831855 3.141592653589793
// expect: 1.4142135 2
// expect: 5
pure hyp(a: float, b: float) -> float {
    sqrt(a * a + b * b)
}

let HALF_PI = PI / 2
let ROOT2 = sqrt(2.0)

main() {
    print(sqrt(16), floor(2.7), ceil(2.1), round(2.5), round(-2.5), round(0.49999997))
    print(cos(0), sin(0), atan2(0, 1), pow(2, 10), exp(0), log(1))
    let wide: f64 = PI
    print(PI, TAU, wide)
    print(ROOT2, round(HALF_PI))
    print(hyp(3, 4))
}
```
`tests/errors/math.kes`:
```
main() {
    let min = 3
    print(sqrt("x"))
    print(sqrt(1, 2))
    sqrt(4)
    print(atan2(1))
}
abs(x: int) -> int { x }
// error 2: `min` is a built-in name; pick another
// error 3: `sqrt` needs a number here, found string
// error 4: `sqrt` takes one value
// error 5: the result of `sqrt` is unused, so this line does nothing
// error 6: `atan2` takes two values
// error 8: `abs` is a built-in name; pick another
```
- [ ] **Step 2: Run** `cargo test -p kestrel-cli` and see them fail.
- [ ] **Step 3: Implement** the enum, typing, lowering and codegen above. Search the existing tests and benchmarks for any function, constant or variable that now collides with a reserved name; there were none on 2026-10-10.
- [ ] **Step 4: Run** `cargo test --workspace` and clippy (both workspaces). Commit "Add float math builtins".

### Task 4: `abs sign min max clamp lerp` and the vector methods

**Files:** `check/src/body/builtins.rs`, `check/src/body/structs.rs` (`check_method_call` dispatches `length normalized dot distance_to` on `vec2`/`vec3` before the struct method lookup), `kir/src/lower/builtins.rs`, `kir/src/lower/expr.rs` (method-call dispatch)
- Test: `tests/programs/math_generic.kes`, more cases in `tests/errors/math.kes`

**Typing:**
- `abs(x)`, `sign(x)`: any number; the result is its type.
- `min(a, b)`, `max(a, b)`: two numbers meeting at `Ty::common` (coerce both), like `+`.
- `clamp(x, lo, hi)`: the same over three values (meet `x` with `lo`, then with `hi`).
- `lerp(a, b, t)`:
  - If `a` is a vector, `b` must be the same vector type and `t` a number coerced to `float`; the result is the vector.
  - Otherwise `a` and `b` are numbers meeting at their common type, and an integer result becomes `float`. `t` is coerced to that type. So `lerp(0, 10, 0.5)` is `float` 5.
  - A mismatch: "`lerp` needs two numbers or two vectors of the same size, found vec2 and float".
- Vector methods on a `vec2`/`vec3` receiver:
  - `length()` and `normalized()` take no values;
  - `dot(b)` and `distance_to(b)` take one value of the receiver's type (coerce, so a mismatch says "expected vec2, found vec3").
  - `length`, `dot` and `distance_to` give `float`; `normalized` gives the receiver's type.
  - A wrong count says "`dot` takes one value" or "`length` takes no values".
  - Other names keep the existing "`vec2` has no method `foo`".

**Lowering** (`kir/src/lower/builtins.rs`). Every operand is evaluated once, in source order, through `spill`. Results are wrapped as `ExprKind::Seq(pre, value)` (see `with_pre`).

| Builtin | Integer types | Float types |
|---|---|---|
| `abs(x)` | `If(a < 0, Neg a, a)`; for `U8` just `a`. `Neg` is checked, so `abs` of the smallest `int` panics with "integer overflow" when checks are on | `Math(Abs, [x])` |
| `sign(x)` | `If(a > 0, 1, If(a < 0, -1, 0))`; for `U8`, `If(a > 0, 1, 0)` | same shape, with float constants |
| `min(a, b)` | `If(x <= y, x, y)` | `Math(Min, [a, b])` |
| `max(a, b)` | `If(x >= y, x, y)` | `Math(Max, [a, b])` |
| `clamp(x, lo, hi)` | `m = If(a > l, a, l)`, then `If(m < h, m, h)` | `Math(Min, [Math(Max, [x, lo]), hi])` |
| `lerp(a, b, t)` | (the result is never an integer) | `x + (y - x) * s` |

Vector `lerp`: `vec_combine(Add, x, vec_combine(Mul, vec_combine(Sub, y, x), s))`.

The vector methods, with the receiver `v` spilled (an `Operand`-style place read; see `kir/src/lower/vector.rs`):
- `length()`: `Math(Sqrt, [v.x*v.x + v.y*v.y (+ v.z*v.z)])`.
- `dot(w)`: the sum of products.
- `distance_to(w)`: spill `d = v - w` (`vec_combine(Sub)`), then the length of `d`.
- `normalized()`: spill `len` (the length), then `If(len > 0, StructLit(v.x / len, ...), StructLit(0, ...))`. A zero vector stays zero, as in Godot.

- [ ] **Step 1: Failing tests.** `tests/programs/math_generic.kes`:
```
// expect: 3 2.5 0 1 -1 1 0
// expect: 1.5 9 10 0 7
// expect: 2.5 5 vec2(x: 5, y: 10)
// expect: 5 vec2(x: 0.6, y: 0.8) 11 5 vec2(x: 0, y: 0)
// expect: 13 vec3(x: 0, y: 1, z: 0)
pure speed(v: vec2) -> float {
    v.length()
}

main() {
    let small: u8 = 7
    print(abs(-3), abs(-2.5), abs(0), sign(small), sign(-7), sign(2.5), sign(0.0))
    print(min(3, 1.5), max(2, 9), clamp(15, 0, 10), clamp(-4, 0, 10), clamp(7, 0, 10))
    print(lerp(0, 10, 0.25), lerp(0.0, 10.0, 0.5), lerp(vec2(0, 0), vec2(10, 20), 0.5))
    let v = vec2(3, 4)
    print(speed(v), v.normalized(), v.dot(vec2(1, 2)), v.distance_to(vec2(0, 0)), vec2(0, 0).normalized())
    let w = vec3(3, 4, 12)
    print(w.length(), vec3(0, 5, 0).normalized())
}
```
And a program `tests/programs/abs_overflow.kes`:
```
// expect-exit: 101
// expect-stderr: integer overflow on line 5
main() {
    let x = -2147483647 - 1
    print(abs(x))
}
```
(The panic names the line of the `abs` call.)
Append to `tests/errors/math.kes`, before the expectation lines (adjust its line numbers):
```
other() {
    print(lerp(vec2(1, 1), 2.0, 0.5))
    print(vec2(1, 1).dot(vec3(1, 1, 1)))
    print(vec2(1, 1).length(2))
}
```
with "`lerp` needs two numbers or two vectors of the same size, found vec2 and float", "expected vec2, found vec3", "`length` takes no values".
- [ ] **Step 2: Run** and see them fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace` and clippy (both workspaces). Commit "Add abs, sign, min, max, clamp, lerp and vector methods".

### Task 5: Random numbers

**Files:** `check/src/body/builtins.rs`, `kir/src/ir.rs` (`ExprKind::RandFloat`, `ExprKind::RandInt(Box<Expr>, Box<Expr>)`, `Stmt::RandSeed(Expr)`), `kir/src/lower/builtins.rs`, `kir/src/lower/` statement lowering for a discarded `rand_seed` call, `kir/src/visit.rs`, `rename.rs`, `inline.rs`, `codegen/src/func/math.rs`, `codegen/src/func/mod.rs` (the statement)
- Test: `tests/programs/random.kes`, more in `tests/errors/math.kes`

**Typing:**
- `randf()` takes no values and gives `float`.
- `randf_range(lo, hi)`: numbers. An integer becomes `float`; the two meet at their common type (`float` or `f64`), and that's the result.
- `randi_range(lo, hi)`: integers meeting at their common type, which is the result. A float argument: "`randi_range` needs integers, found float".
- `rand_seed(n)`: any integer. It gives nothing, so it is used as a statement.
- **None of these are pure.**
  - In a `pure` function: "pure functions can't use random numbers: each call changes the generator".
  - In a `par for` body: "`par for` bodies can't use random numbers: iterations run at the same time".
  - Either way, set the enclosing function's `io = true` (as `print` does).

**Lowering and codegen:**
- `randf()`: `RandFloat` at `F32`. Codegen calls `rt.rand_float` and demotes the result to `F32`.
- `randf_range(lo, hi)`: spill `a = lo` and `b = hi` (in that order), then `a + (b - a) * RandFloat` at the result type.
- `randi_range(lo, hi)`: `RandInt(lo, hi)`. Codegen extends both operands to `i64` (`sextend` for signed, `uextend` for `U8`), calls `rt.rand_int`, then `ireduce`s to the result type. The result is within the operands, so narrowing is safe.
- `rand_seed(n)`: `Stmt::RandSeed(n)`. Codegen extends the operand to `i64` and calls `rt.rand_seed`.

- [ ] **Step 1: Failing tests.** `tests/programs/random.kes`:
```
// expect: true
// expect: true
// expect: true
// expect: true
main() {
    rand_seed(7)
    let a = randi_range(1, 6)
    let x = randf()
    let r = randf_range(-1, 1)
    rand_seed(7)
    print(a == randi_range(1, 6) and x == randf() and r == randf_range(-1, 1))
    var seen = array[bool](7)
    var in_range = true
    for i from 0 to 1000 {
        let d = randi_range(1, 6)
        if d < 1 or d > 6 {
            in_range = false
        } else {
            seen[d] = true
        }
    }
    print(in_range)
    print(seen[1] and seen[2] and seen[3] and seen[4] and seen[5] and seen[6])
    var ok = true
    for i from 0 to 1000 {
        let f = randf_range(2, 3)
        if f < 2 or f > 3 {
            ok = false
        }
    }
    print(ok)
}
```
Append to `tests/errors/math.kes`:
```
pure roll() -> int {
    randi_range(1, 6)
}
dice() {
    var xs = [1, 2, 3]
    par for x in xs {
        x = randi_range(1, 6)
    }
    print(randi_range(1.5, 2))
}
```
with "pure functions can't use random numbers: each call changes the generator", "`par for` bodies can't use random numbers: iterations run at the same time", "`randi_range` needs integers, found float".
- [ ] **Step 2: Run** and see them fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace` and clippy (both workspaces). Commit "Add random numbers".

### Task 6: Docs, Godot check, wrap-up

**Files:** `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md`, `docs/superpowers/specs/2026-10-10-window-design.md`, `docs/superpowers/plans/2026-10-09-core-2a-followups.md`, `docs/superpowers/plans/2026-10-10-handoff.md`

- [ ] **Step 1: Syntax spec.**
  - The Interpolation bullet: "works anywhere; the string is built in the current arena".
  - A short "Built-in functions" section listing the math, vector and random names (point to the window spec for the table).
  - The Strings row: copies copy bytes.
- [ ] **Step 2: Window spec.**
  - The reserved-name message is the existing "`min` is a built-in name; pick another".
  - The string buffer is a separate per-thread stack, not the print sink.
- [ ] **Step 3: Follow-ups.**
  - Remove the "Strings (2b-ii scope)" bullet's claim that interpolation works only in `print`.
  - Add three items:
    - copies of literal-backed strings still copy their bytes (a static-data check would skip them);
    - `round` and the trigonometric functions go through a call (`kestrel_rt_math`) or a few instructions;
    - the random generator is per thread.
- [ ] **Step 4: Godot.** Build the add-on (`kestrel/godot/build.ps1`, with Godot closed) and run the headless test (`demo/tests/test_bridge.gd`); all 26 checks must still pass (there are 24). Run clippy in `kestrel/godot`.
- [ ] **Step 5: Handoff.** Plan 1 is done; next is plan 2 (the window). Update the test count.
- [ ] **Step 6: Commit and push** "Document math, random numbers and strings anywhere".
