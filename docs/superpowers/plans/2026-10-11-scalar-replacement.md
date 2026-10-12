# Scalar Replacement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep small structs (`vec2`, `vec3`, `Particle`, ...) and the fields of `var` parameters and loop elements in registers, so the particle step runs within 1.2x of Rust instead of 3.4x.

**Architecture:** Two KIR passes run after inlining. `scalarize` splits struct locals into one local per field and stores struct values one field at a time. `promote` caches the fields of pointer parameters and loop elements in locals for a region and writes them back at every way out. A shared rewriter (`split.rs`) does the place rewriting for both. Codegen changes only for a dead-local kind, a debugging dump, and one runtime function per math operation.

**Tech Stack:** Rust, Cranelift 0.116, the Kestrel workspace in `kestrel/`.

**Spec:** `docs/superpowers/specs/2026-10-11-scalar-replacement-design.md`

**Prototype:** every piece of code below was built and tested in a scratch copy of the workspace on 2026-10-11. All 529 existing tests plus the 14 new ones passed, clippy was clean, and the step took 4.32 ns per particle (12.81 before; Rust 3.78). Copy the code as written; if something doesn't compile, the repo has moved since then, so fix the call site, not the design.

## Global Constraints

- **No syntax change, and no program's output changes.** Every existing program test keeps its expected output.
- **A plain struct** has fields that are `Bool`, `U8`, `I32`, `I64`, `F32`, `F64` or plain structs, with at most 16 leaves (`MAX_LEAVES = 16`). A struct with no leaves isn't plain.
- **Math keeps `f64`** with the same Rust calls, so results are bit-identical.
- **Small files:** keep each source file under about 400 lines.
- **Comments and messages** are plain sentences that say what to do.
- **Never call a bare `link.exe`:** Git's coreutils `link.exe` shadows the MSVC linker on PATH.
- **No visible windows, ever.** Window programs run with `KESTREL_HEADLESS=<frames>` or `KESTREL_HIDDEN=1`. Never set `KESTREL_SMOKE_VISIBLE`.
- **Tests:** `cargo test --workspace --no-fail-fast` in `kestrel/`.
- **Clippy:** `cargo clippy --workspace --all-targets -- -D warnings` in `kestrel/`, and in `kestrel/godot` in the last task.
- **Commits:** commit to `main` and push after each task (the user approved this). End every commit message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Benchmarks:** this machine is noisy. Use medians over interleaved rounds and report ranges. Report slowdowns as plainly as speed-ups.
- **Shell escaping:** shell heredocs sometimes mangle `\n` and `\\` (a command hook rewrites commands). Write files containing escapes with the Write or Edit tools, not `cat <<EOF`.

## Review Focus

These are cases the spec implies but the obvious tests don't reach. Each now has a test in the task that owns the code:

1. **A plain struct stored inside an enum variant or an optional** (`d.at = d.at + by` in a `match`, `h.spot = v.yx` into a `vec2?`) must store and print as before. Covered by `scalarize_payloads.kes` (Task 2).
2. **A plain struct mixing `bool`, `u8`, `int`, `i64` and `f64` fields** must round-trip exactly through calls, prints, interpolation and lists. Covered by `scalarize_calls.kes` (Task 2).
3. **A `var self` method whose loop condition reads `self` and which returns from inside the loop** must leave the final values in the caller's variable. Covered by `climb` in `promote_var_self.kes` (Task 3).
4. **A returned value whose computation changes `self`** (`return self.next() * 10` with `next` inlined) must be computed before the write-back, so the change isn't lost. Covered by `tick` in `promote_var_self.kes` (Task 3).
5. **A struct with more than 16 leaves** must stay in memory and behave as before. Covered by `Wide` in `scalarize_calls.kes` and a KIR unit test (Task 2).

---

### Task 1: The kernel benchmark and `kestrel_codegen::dump`

The baseline is measured before any optimisation lands, so this task comes first.

**Files:**
- Create: `kestrel/codegen/src/dump.rs`
- Modify: `kestrel/codegen/src/lib.rs` (module list, re-export, `define_module`)
- Modify: `kestrel/codegen/src/jit.rs:42`, `kestrel/codegen/src/object.rs:14`
- Modify: `kestrel/codegen/tests/common/mod.rs` (new helper `optimized`)
- Create: `kestrel/codegen/tests/dump.rs`
- Create: `kestrel/bench/kernels/Cargo.toml`, `src/main.rs`, `src/bin/rust.rs`, `step.kes`, `step_pow.kes`, `step_scalar.kes`, `copy.kes`, `measure.py`, `results.md`
- Modify: `kestrel/Cargo.toml` (`exclude`)

**Interfaces:**
- Produces: `#[doc(hidden)] pub fn kestrel_codegen::dump(m: &kestrel_kir::Module, opts: Options, name: &str) -> Result<String, String>`. It returns the named function's optimised Cranelift IR followed by its machine code, or `Err("no function named `<name>`")`.
- Produces: `pub(crate) fn define_module<M: Module>(module: &mut M, m: &kir::Module, opts: Options, dump: Option<&mut dump::Dump>) -> Result<Vec<FuncId>, String>` (one new last parameter).
- Produces: `codegen/tests/common/mod.rs::optimized(src: &str) -> kestrel_kir::Module` (parse, check, lower, then `inline_small` for now; Task 2 switches it to `optimize`).
- Produces: `bench/kernels`: binaries `kernels` (`kernels <file.kes> [--release] [--n N] [--iters K] [--reps R] [--dump NAME|par]`) and `rust` (`rust <step|pow|copy> [--n N] [--iters K] [--reps R]`), plus `measure.py [--rounds 3] [--threads 1]`.

- [ ] **Step 1: Write the failing tests**

Add this helper at the end of `kestrel/codegen/tests/common/mod.rs`:

```rust
/// `src` compiled to KIR and optimised, as the driver does.
pub fn optimized(src: &str) -> kestrel_kir::Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = kestrel_kir::lower(&p, &t.unwrap(), &LineIndex::new(src));
    kestrel_kir::inline::inline_small(&mut m);
    m
}
```

Create `kestrel/codegen/tests/dump.rs`:

```rust
mod common;

use kestrel_codegen::{dump, Options};

const RELEASE: Options = Options { checked_arith: false };

#[test]
fn dump_shows_the_ir_and_the_machine_code() {
    let m = common::optimized("pure sq(x: int) -> int { x * x }\nmain() { print(sq(3)) }\n");
    let code = dump(&m, RELEASE, "sq").unwrap();
    assert!(code.contains("function") && code.contains("imul") && code.contains("ret"), "{code}");
}

#[test]
fn dump_names_a_missing_function() {
    let m = common::optimized("main() {}\n");
    assert_eq!(dump(&m, RELEASE, "nope").unwrap_err(), "no function named `nope`");
}
```

- [ ] **Step 2: Run them to see them fail**

Run (in `kestrel/`): `cargo test -p kestrel-codegen --test dump`
Expected: does not compile: "unresolved import `kestrel_codegen::dump`".

- [ ] **Step 3: Add `dump`**

Create `kestrel/codegen/src/dump.rs`:

```rust
//! A function's code as Cranelift leaves it, for tests and benchmarks that check what the
//! compiler made of a loop: its IR after optimisation, then its machine code.

use cranelift_codegen::Context;
use cranelift_object::{ObjectBuilder, ObjectModule};
use kestrel_kir as kir;

use crate::{define_module, isa, Options};

pub(crate) struct Dump<'a> {
    pub name: &'a str,
    pub text: String,
}

impl Dump<'_> {
    /// Adds the optimised IR and the machine code of the function `ctx` just compiled.
    pub(crate) fn record(&mut self, ctx: &Context) {
        self.text.push_str(&ctx.func.display().to_string());
        if let Some(asm) = ctx.compiled_code().and_then(|c| c.vcode.as_deref()) {
            self.text.push('\n');
            self.text.push_str(asm);
        }
    }
}

/// The optimised Cranelift IR and the machine code of the function named `name` in `m`.
pub fn dump(m: &kir::Module, opts: Options, name: &str) -> Result<String, String> {
    let builder = ObjectBuilder::new(isa::host_isa()?, "kestrel", cranelift_module::default_libcall_names()).map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);
    let mut d = Dump { name, text: String::new() };
    define_module(&mut module, m, opts, Some(&mut d))?;
    if d.text.is_empty() {
        return Err(format!("no function named `{name}`"));
    }
    Ok(d.text)
}
```

In `kestrel/codegen/src/lib.rs`:
- Add `mod dump;` before `pub mod export;`.
- Add `#[doc(hidden)]` and `pub use dump::dump;` before `pub use export::{ExportInfo, ExportTy};`.
- Replace the doc comment and signature of `define_module` with:

```rust
/// Declares every function of `m` in `module` and defines the ones with bodies
/// (natives are imports under their own symbols), and defines its globals. Returns
/// the functions' ids, indexed by `FuncIdx`. With `dump`, the named function's code is
/// recorded in it as it is compiled.
pub(crate) fn define_module<M: Module>(
    module: &mut M,
    m: &kir::Module,
    opts: Options,
    mut dump: Option<&mut dump::Dump>,
) -> Result<Vec<FuncId>, String> {
```

- In its loop, replace

```rust
        func::lower_func(module, &mut strings, &shared, &mut ctx.func, &mut fb_ctx, f)?;
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("codegen failed for `{}`: {e:?}", f.name))?;
        module.clear_context(&mut ctx);
```

with

```rust
        func::lower_func(module, &mut strings, &shared, &mut ctx.func, &mut fb_ctx, f)?;
        let dumping = dump.as_ref().is_some_and(|d| d.name == f.name);
        ctx.set_disasm(dumping);
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("codegen failed for `{}`: {e:?}", f.name))?;
        if let Some(d) = dump.as_deref_mut().filter(|_| dumping) {
            d.record(&ctx);
        }
        module.clear_context(&mut ctx);
```

In `jit.rs` and `object.rs`, the call becomes `define_module(&mut module, m, opts, None)?`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p kestrel-codegen --test dump`
Expected: 2 passed.

- [ ] **Step 5: Create the benchmark workspace**

Add `"bench/kernels"` to `exclude` in `kestrel/Cargo.toml`:
```toml
exclude = ["godot", "bench/particles/rust", "bench/kernels"]
```

Create `kestrel/bench/kernels/Cargo.toml`:

```toml
[package]
name = "kernels"
version = "0.1.0"
edition = "2021"

# Its own workspace (and listed in the `kestrel/` workspace's `exclude`), like `bench/particles/rust`:
# the compiler's tests shouldn't build it.
[workspace]

[dependencies]
kestrel-driver = { path = "../../driver" }
kestrel-codegen = { path = "../../codegen" }

[profile.release]
debug = false
```

Create `kestrel/bench/kernels/src/main.rs`:

```rust
//! Times a Kestrel kernel: compiles a `.kes` file the way `kestrel run` does, then calls its
//! exported `pub bench(n: int, iters: int) -> float` through the JIT, once with `iters` and once
//! with 0 (to take out the set-up), and prints the median time per item over `reps` pairs.
//!
//!   kernels <file.kes> [--release] [--n N] [--iters K] [--reps R] [--dump NAME]
//!
//! `--dump NAME` prints a function's optimised Cranelift IR and machine code instead (`par` for
//! every `par for` body). `KESTREL_THREADS` sets the `par for` thread count as usual.

use std::time::Instant;

use kestrel_codegen::{dump, ExportTy, JitProgram, Options};
use kestrel_driver::DiagStyle;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first().cloned() else {
        eprintln!("usage: kernels <file.kes> [--release] [--n N] [--iters K] [--reps R] [--dump NAME]");
        std::process::exit(2);
    };
    let flag = |name: &str| args.iter().position(|a| a == name).map(|i| args[i + 1].clone());
    let number = |name: &str, default: i64| flag(name).map_or(default, |v| v.parse().expect("a number"));
    let (n, iters, reps) = (number("--n", 200_000), number("--iters", 30), number("--reps", 9) as usize);
    let opts = Options { checked_arith: !args.iter().any(|a| a == "--release") };

    let src = std::fs::read_to_string(&path).expect("read the program");
    let (compiled, diagnostics) = kestrel_driver::compile(&path, &src, DiagStyle::Short);
    eprint!("{diagnostics}");
    let module = compiled.expect("the program compiles").module;
    if let Some(name) = flag("--dump") {
        let names: Vec<String> = if name == "par" {
            module.par_bodies.iter().map(|f| module.funcs[f.0 as usize].name.clone()).collect()
        } else {
            vec![name]
        };
        for name in names {
            println!("===== {name} =====\n{}", dump(&module, opts, &name).expect("dump"));
        }
        return;
    }

    let program = JitProgram::compile(&module, opts).expect("codegen");
    program.run_init();
    let (info, thunk) = program.exports().into_iter().find(|(i, _)| i.name == "bench").expect("a `pub bench(n: int, iters: int) -> float`");
    assert_eq!((info.params.as_slice(), info.ret.as_ref()), ([ExportTy::Int, ExportTy::Int].as_slice(), Some(&ExportTy::Float)));
    let call = |n: i64, k: i64| -> (f64, f32) {
        let mut buf = vec![0u8; info.args_size as usize];
        buf[info.arg_offsets[0] as usize..][..4].copy_from_slice(&(n as i32).to_le_bytes());
        buf[info.arg_offsets[1] as usize..][..4].copy_from_slice(&(k as i32).to_le_bytes());
        let mut ret = [0u8; 8];
        let start = Instant::now();
        // SAFETY: the buffer is laid out as `info` says, and `ret` holds the `float` result.
        unsafe { thunk(buf.as_ptr(), ret.as_mut_ptr()) };
        (start.elapsed().as_secs_f64(), f32::from_le_bytes(ret[..4].try_into().unwrap()))
    };
    call(n, 2);
    let mut per = Vec::new();
    let mut checksum = 0.0;
    for _ in 0..reps {
        let (base, _) = call(n, 0);
        let (t, sum) = call(n, iters);
        checksum = sum;
        per.push((t - base) / (n * iters) as f64 * 1e9);
    }
    per.sort_by(f64::total_cmp);
    println!(
        "{path}: {:.2} ns per item (median; {:.2}-{:.2}), n {n}, iters {iters}, reps {reps}, checksum {checksum}",
        per[per.len() / 2],
        per[0],
        per[per.len() - 1]
    );
}
```

Create `kestrel/bench/kernels/src/bin/rust.rs`:

```rust
//! The kernels in Rust, for comparison: the particle step of `step.kes` (`step`), the same with
//! `pow` per particle as `step_pow.kes` (`pow`), and the copy of `copy.kes` (`copy`). One thread,
//! release build, timed like `kernels` (median ns per item).
//!
//!   rust <step|pow|copy> [--n N] [--iters K] [--reps R]

use std::hint::black_box;
use std::time::Instant;

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 720.0;
const PULL: f32 = 900.0;
const KEEP: f64 = 0.5;
const BOUNCE: f32 = 0.7;

#[derive(Clone, Copy)]
struct V2 {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
struct Particle {
    pos: V2,
    vel: V2,
}

#[inline(always)]
fn step(p: &mut Particle, dt: f32, target: V2, keep: f32) {
    let (dx, dy) = (target.x - p.pos.x, target.y - p.pos.y);
    let len = (dx * dx + dy * dy).sqrt();
    let (nx, ny) = if len > 0.0 { (dx / len, dy / len) } else { (0.0, 0.0) };
    p.vel.x = (p.vel.x + nx * PULL * dt) * keep;
    p.vel.y = (p.vel.y + ny * PULL * dt) * keep;
    p.pos.x += p.vel.x * dt;
    p.pos.y += p.vel.y * dt;
    if p.pos.x < 0.0 {
        p.pos.x = 0.0;
        p.vel.x = p.vel.x.abs() * BOUNCE;
    } else if p.pos.x > WIDTH {
        p.pos.x = WIDTH;
        p.vel.x = -p.vel.x.abs() * BOUNCE;
    }
    if p.pos.y < 0.0 {
        p.pos.y = 0.0;
        p.vel.y = p.vel.y.abs() * BOUNCE;
    } else if p.pos.y > HEIGHT {
        p.pos.y = HEIGHT;
        p.vel.y = -p.vel.y.abs() * BOUNCE;
    }
}

#[inline(never)]
fn run_step(parts: &mut [Particle], dt: f32, target: V2, keep: f32) {
    for p in parts.iter_mut() {
        step(p, dt, target, keep);
    }
}

#[inline(never)]
fn run_pow(parts: &mut [Particle], dt: f32, target: V2, _keep: f32) {
    for p in parts.iter_mut() {
        // `black_box` stops LLVM from noticing every particle gets the same value.
        let keep = black_box(KEEP).powf(dt as f64) as f32;
        step(p, dt, target, keep);
    }
}

#[inline(never)]
fn run_copy(dst: &mut [V2], parts: &[Particle]) {
    for (d, p) in dst.iter_mut().zip(parts) {
        *d = p.pos;
    }
}

/// `count` particles from a fixed seed (splitmix64).
fn spawn(count: usize) -> Vec<Particle> {
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut range = |lo: f32, hi: f32| {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        lo + (hi - lo) * (((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32)
    };
    (0..count)
        .map(|_| Particle {
            pos: V2 { x: range(0.0, WIDTH), y: range(0.0, HEIGHT) },
            vel: V2 { x: range(-200.0, 200.0), y: range(-200.0, 200.0) },
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().cloned().unwrap_or_else(|| "step".into());
    let number = |name: &str, default: usize| args.iter().position(|a| a == name).map_or(default, |i| args[i + 1].parse().expect("a number"));
    let (n, iters, reps) = (number("--n", 200_000), number("--iters", 30), number("--reps", 9));
    let dt = black_box(1.0f32 / 60.0);
    let target = black_box(V2 { x: 640.0, y: 360.0 });
    let keep = black_box(KEEP).powf(dt as f64) as f32;
    let mut per = Vec::new();
    for rep in 0..=reps {
        let mut parts = spawn(n);
        let mut dst = vec![V2 { x: 0.0, y: 0.0 }; n];
        let start = Instant::now();
        for _ in 0..iters {
            match mode.as_str() {
                "step" => run_step(black_box(&mut parts), dt, target, keep),
                "pow" => run_pow(black_box(&mut parts), dt, target, keep),
                "copy" => run_copy(black_box(&mut dst), black_box(&parts)),
                other => panic!("no kernel named {other}"),
            }
        }
        let t = start.elapsed().as_secs_f64();
        black_box((&parts, &dst));
        // The first pass warms up.
        if rep > 0 {
            per.push(t / (n * iters) as f64 * 1e9);
        }
    }
    per.sort_by(f64::total_cmp);
    println!("rust {mode}: {:.2} ns per item (median; {:.2}-{:.2}), n {n}, iters {iters}, reps {reps}", per[per.len() / 2], per[0], per[per.len() - 1]);
}
```

Create `kestrel/bench/kernels/step.kes`:

```
// The particles demo's step (`demos/particles/particles.kes`) with `pow(KEEP, dt)` worked out
// once and passed in, over a list in a `par for`. `bench(n, iters)` steps `n` particles `iters`
// times and returns a checksum; `bench/kernels` times it.
let WIDTH = 1280
let HEIGHT = 720
let PULL = 900.0
let KEEP = 0.5
let BOUNCE = 0.7

struct Particle {
    pos: vec2
    vel: vec2

    pure step(var self, dt: float, target: vec2, keep: float) {
        self.vel += (target - self.pos).normalized() * PULL * dt
        self.vel *= keep
        self.pos += self.vel * dt
        if self.pos.x < 0 {
            self.pos.x = 0
            self.vel.x = abs(self.vel.x) * BOUNCE
        } else if self.pos.x > WIDTH {
            self.pos.x = WIDTH
            self.vel.x = -abs(self.vel.x) * BOUNCE
        }
        if self.pos.y < 0 {
            self.pos.y = 0
            self.vel.y = abs(self.vel.y) * BOUNCE
        } else if self.pos.y > HEIGHT {
            self.pos.y = HEIGHT
            self.vel.y = -abs(self.vel.y) * BOUNCE
        }
    }
}

spawn(var parts: list[Particle], count: int) {
    for i from 0 to count {
        let pos = vec2(randf_range(0, WIDTH), randf_range(0, HEIGHT))
        let vel = vec2(randf_range(-200, 200), randf_range(-200, 200))
        parts.add(Particle(pos: pos, vel: vel))
    }
}

pub bench(n: int, iters: int) -> float {
    var sum = 0.0
    arena {
        var parts = list[Particle]()
        spawn(var parts, n)
        let dt = 1.0 / 60.0
        let target = vec2(640, 360)
        let keep = pow(KEEP, dt)
        for k from 0 to iters {
            par for p in parts {
                p.step(dt, target, keep)
            }
        }
        for p in parts {
            sum += p.pos.x + p.pos.y
        }
    }
    sum
}

main() {
    print(bench(1000, 10))
}
```

Create `kestrel/bench/kernels/step_pow.kes`: the same as `step.kes` except as follows.
- The header comment is:
  ```
  // The particles demo's step as the demo writes it: `pow(KEEP, dt)` for every particle.
  // `bench(n, iters)` steps `n` particles `iters` times and returns a checksum.
  ```
- `step` is `pure step(var self, dt: float, target: vec2)` and its second line is `self.vel *= pow(KEEP, dt)`.
- `bench` has no `let keep` line and calls `p.step(dt, target)`.

Create `kestrel/bench/kernels/step_scalar.kes`: the same as `step.kes` except as follows.
- The header comment is:
  ```
  // `step.kes` written by hand with float locals: the fields are read once, the maths uses no
  // `vec2` values, and the fields are written once. The speed the compiler should reach on
  // `step.kes` by itself. `bench(n, iters)` steps `n` particles `iters` times.
  ```
- `step`'s body is:

```
        var px = self.pos.x
        var py = self.pos.y
        var vx = self.vel.x
        var vy = self.vel.y
        let dx = target.x - px
        let dy = target.y - py
        let len = sqrt(dx * dx + dy * dy)
        var nx = 0.0
        var ny = 0.0
        if len > 0 {
            nx = dx / len
            ny = dy / len
        }
        vx = (vx + nx * PULL * dt) * keep
        vy = (vy + ny * PULL * dt) * keep
        px += vx * dt
        py += vy * dt
        if px < 0 {
            px = 0
            vx = abs(vx) * BOUNCE
        } else if px > WIDTH {
            px = WIDTH
            vx = -abs(vx) * BOUNCE
        }
        if py < 0 {
            py = 0
            vy = abs(vy) * BOUNCE
        } else if py > HEIGHT {
            py = HEIGHT
            vy = -abs(vy) * BOUNCE
        }
        self.pos.x = px
        self.pos.y = py
        self.vel.x = vx
        self.vel.y = vy
```

Create `kestrel/bench/kernels/copy.kes`:

```
// The particles demo's draw copy: every particle's position into a list of `vec2`s.
// `bench(n, iters)` copies `n` positions `iters` times and returns a checksum.
let WIDTH = 1280
let HEIGHT = 720

struct Particle {
    pos: vec2
    vel: vec2
}

spawn(var parts: list[Particle], count: int) {
    for i from 0 to count {
        let pos = vec2(randf_range(0, WIDTH), randf_range(0, HEIGHT))
        let vel = vec2(randf_range(-200, 200), randf_range(-200, 200))
        parts.add(Particle(pos: pos, vel: vel))
    }
}

pub bench(n: int, iters: int) -> float {
    var sum = 0.0
    arena {
        var parts = list[Particle]()
        spawn(var parts, n)
        var positions = list[vec2]()
        while positions.len < parts.len {
            positions.add(vec2(0, 0))
        }
        for k from 0 to iters {
            for i, p in parts {
                positions[i] = p.pos
            }
        }
        for q in positions {
            sum += q.x + q.y
        }
    }
    sum
}

main() {
    print(bench(1000, 10))
}
```

Create `kestrel/bench/kernels/measure.py`:

```python
"""Runs every kernel, Kestrel and Rust, for a few interleaved rounds and prints the median of the
rounds' medians, with their range, in ns per item.

    py measure.py [--rounds 3] [--threads 1]

Run from kestrel/bench/kernels after `cargo build --release` there. `--threads` sets
KESTREL_THREADS for the Kestrel kernels (their `par for`); the Rust kernels are one thread.
"""
import argparse, os, re, statistics, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, "target", "release")

# (label, command, iterations): more iterations for the cheap kernels.
KERNELS = [
    ("step.kes (the demo's step, pow hoisted)", ["kernels", "step.kes", "--release"], 30),
    ("step_scalar.kes (written with float locals)", ["kernels", "step_scalar.kes", "--release"], 30),
    ("step_pow.kes (pow per particle, the demo)", ["kernels", "step_pow.kes", "--release"], 15),
    ("copy.kes (the draw copy)", ["kernels", "copy.kes", "--release"], 100),
    ("Rust step", ["rust", "step"], 30),
    ("Rust pow", ["rust", "pow"], 15),
    ("Rust copy", ["rust", "copy"], 100),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--threads", type=int, default=1)
    a = ap.parse_args()
    env = dict(os.environ, KESTREL_THREADS=str(a.threads))
    results = {label: [] for label, _, _ in KERNELS}
    for _ in range(a.rounds):
        for label, cmd, iters in KERNELS:
            exe = os.path.join(BIN, cmd[0] + (".exe" if os.name == "nt" else ""))
            out = subprocess.run([exe, *cmd[1:], "--iters", str(iters)], cwd=HERE, env=env, capture_output=True, text=True)
            m = re.search(r"([\d.]+) ns per item", out.stdout)
            if not m:
                raise SystemExit(f"{label} failed:\n{out.stdout}{out.stderr}")
            results[label].append(float(m.group(1)))
    print(f"{a.threads} thread(s) for Kestrel, {a.rounds} rounds")
    for label, vals in results.items():
        print(f"{label:46s} {statistics.median(vals):6.2f} ns  ({min(vals):.2f}-{max(vals):.2f})")


if __name__ == "__main__":
    main()
```

- [ ] **Step 6: Build it, check the kernels, and measure the baseline**

In `kestrel/bench/kernels`:
- Run `cargo build --release`. It writes `Cargo.lock` (commit it, as `bench/particles/rust` does).
- Run `target/release/kernels.exe step.kes --release --dump par`. Expected: one `par for` body with `explicit_slot` lines: the `vec2` temporaries this plan removes.
- Run `py measure.py --rounds 3 --threads 1` and `py measure.py --rounds 3 --threads 16`. Expected on this machine at one thread: about `step.kes` 12.8, `step_scalar.kes` 4.3, `step_pow.kes` 26, `copy.kes` 0.9, Rust step 3.8, Rust pow 15.7, Rust copy 0.2. Different numbers are fine; write down what you measure.

Also record the old compiler's times for the other benchmarks, for Task 5 to compare with:
- Run `run.ps1` in `bench/2a`, `bench/heightmap`, `bench/voxels3d` and `bench/maps` (PowerShell, from each folder; they use `kestrel/target/release/kestrel.exe`, so build it first with `cargo build --release -p kestrel-cli` in `kestrel/`).
- Save each script's output to a scratch file outside the repo.

- [ ] **Step 7: Write `results.md` with the baseline**

Create `kestrel/bench/kernels/results.md`:
- Say what the kernels are and how to run them: `cargo build --release`, `py measure.py`, and `--dump`.
- Add a "Before scalar replacement" table (date, commit, one thread and 16 threads) with the numbers from Step 6.
- Say that "after" numbers come with Task 5.

Plain sentences, like the other `results.md` files.

- [ ] **Step 8: Run the workspace tests and clippy, then commit**

Run (in `kestrel/`): `cargo test --workspace --no-fail-fast` and `cargo clippy --workspace --all-targets -- -D warnings`. Expected: all pass, no warnings.

```bash
git add kestrel/Cargo.toml kestrel/codegen kestrel/bench/kernels
git commit -m "Add a kernel benchmark against Rust and a way to dump a function's machine code" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git push
```

---

### Task 2: `scalarize`: plain structs in registers

**Files:**
- Create: `kestrel/kir/src/leaves.rs` (plain types and their leaves)
- Create: `kestrel/kir/src/visit_mut.rs` (a mutable KIR visitor)
- Create: `kestrel/kir/src/split.rs` (the rewriter `scalarize` and `promote` share)
- Create: `kestrel/kir/src/scalarize.rs`
- Modify: `kestrel/kir/src/ir.rs` (`LocalKind::Unused`), `kestrel/kir/src/lib.rs` (modules, `optimize`)
- Modify: `kestrel/codegen/src/func/mod.rs` (`Slot::Unused`), `kestrel/codegen/src/func/place.rs:32-34`
- Modify: `kestrel/driver/src/lib.rs:105`, `kestrel/codegen/tests/common/mod.rs` (three places), `kestrel/codegen/tests/natives.rs:171`
- Create: `kestrel/kir/tests/scalarize.rs`
- Modify: `kestrel/codegen/tests/dump.rs` (the stack-slot test)
- Create: `kestrel/tests/programs/scalarize_swizzle.kes`, `scalarize_if.kes`, `scalarize_calls.kes`, `scalarize_payloads.kes`

**Interfaces:**
- Consumes: `kestrel_codegen::dump` and `common::optimized` (Task 1).
- Produces: `pub fn kestrel_kir::optimize(m: &mut Module)`, which runs `inline::inline_small`, then `scalarize::scalarize`. Task 3 adds `promote::promote` after them.
- Produces: `LocalKind::Unused`, a local nothing refers to any more.
- Produces, in `kir/src/leaves.rs`:
  - `pub const MAX_LEAVES: usize = 16`
  - `pub struct Leaf { pub path: Vec<u32>, pub ty: KTy }`
  - `pub fn leaves(structs: &[StructLayout], ty: KTy) -> Option<Vec<Leaf>>`
  - `pub fn is_plain_struct(structs, ty) -> bool`
  - `pub fn field_path(path: &[Proj]) -> Option<Vec<u32>>`
  - `pub fn under(all: &[Leaf], prefix: &[u32]) -> Range<usize>`
  - `pub fn leaf_place(base: &Place, leaf: &Leaf) -> Place`
  - `pub fn rebuild(structs, ty, vals: &mut impl Iterator<Item = Expr>, line) -> Expr`
  - `pub fn leaf_name(structs, name: &str, ty, leaf: &Leaf) -> String`
  - `pub fn zero(ty, line) -> Expr`
  - `pub fn load(l: LocalIdx, ty, line) -> Expr`
- Produces, in `kir/src/visit_mut.rs`: `pub trait VisitMut { visit_stmts, visit_stmt, visit_expr, visit_place }`, and `walk_stmt`, `walk_expr`, `walk_place`.
- Produces, in `kir/src/split.rs`:
  - `pub(crate) struct Split { pub leaves: Vec<Leaf>, pub locals: Vec<LocalIdx> }`
  - `pub(crate) struct Rewriter<'a> { pub structs: &'a [StructLayout], pub locals: &'a mut Vec<Local>, pub roots: HashMap<LocalIdx, Split> }`, which implements `VisitMut`.

- [ ] **Step 1: Write the failing tests**

Create `kestrel/kir/tests/scalarize.rs`:

```rust
use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn optimized(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = lower(&p, &t.unwrap(), &LineIndex::new(src));
    optimize(&mut m);
    m
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

/// The names of `f`'s struct locals that still live in memory.
fn struct_slots(f: &Func) -> Vec<&str> {
    f.locals.iter().filter(|l| matches!(l.ty, KTy::Struct(_)) && l.kind == LocalKind::Addressed).map(|l| l.name.as_str()).collect()
}

#[test]
fn vector_temporaries_become_one_local_per_field() {
    let src = "pure f(a: vec2, b: vec2) -> float {\n let d = (a - b).normalized() * 2\n d.x + d.y\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "f");
    assert!(struct_slots(f).is_empty(), "{:#?}", f.locals);
    let d = f.locals.iter().find(|l| l.name == "d").unwrap();
    assert_eq!(d.kind, LocalKind::Unused);
    assert!(f.locals.iter().any(|l| l.name == "d.x" && l.ty == KTy::F32 && l.kind == LocalKind::Value), "{:#?}", f.locals);
}

#[test]
fn a_struct_passed_as_var_stays_in_memory() {
    // `push` returns early, so it isn't inlined and `p` is still passed by pointer.
    let src = "struct P {\n x: float\n y: float\n}\npush(var p: P) {\n if p.x > 10 { return }\n p.x += 1\n}\nmain() {\n var p = P(x: 1, y: 2)\n push(var p)\n print(p)\n}\n";
    assert_eq!(struct_slots(func(&optimized(src), "main")), vec!["p"]);
}

#[test]
fn a_number_passed_as_var_to_an_inlined_call_goes_back_to_a_register() {
    let src = "bump(var n: int) {\n n += 1\n}\nmain() {\n var n = 1\n bump(var n)\n print(n)\n}\n";
    let m = optimized(src);
    let n = func(&m, "main").locals.iter().find(|l| l.name == "n").unwrap();
    assert_eq!(n.kind, LocalKind::Value);
}

#[test]
fn struct_stores_to_memory_go_one_field_at_a_time() {
    let src = "struct B {\n pos: vec2\n vel: vec2\n}\nmain() {\n var bs = [B(pos: vec2(1, 2), vel: vec2(3, 4))]\n bs[0].vel = bs[0].vel.yx + bs[0].pos\n bs[0].pos = bs[0].vel\n print(bs)\n}\n";
    let m = optimized(src);
    let mut copies = 0;
    visit::walk(&func(&m, "main").body, &mut |n| {
        if let visit::Node::Stmt(Stmt::Assign(p, e)) = n {
            if let KTy::Struct(_) = e.ty {
                // Only a copy from one place in memory to another stays whole.
                assert!(matches!(e.kind, ExprKind::Load(_)), "{p:?} = {e:#?}");
                copies += 1;
            }
        }
    });
    assert_eq!(copies, 1);
}

#[test]
fn a_struct_with_more_than_16_fields_at_the_bottom_stays_in_memory() {
    let src = "struct W {\n a: vec3\n b: vec3\n c: vec3\n d: vec3\n e: vec3\n f: vec2\n}\nmain() {\n var w = W(a: vec3(1, 1, 1), b: vec3(1, 1, 1), c: vec3(1, 1, 1), d: vec3(1, 1, 1), e: vec3(1, 1, 1), f: vec2(1, 1))\n w.f = w.f.yx\n print(w)\n}\n";
    assert_eq!(struct_slots(func(&optimized(src), "main")), vec!["w"]);
}
```

Add to `kestrel/codegen/tests/dump.rs`:

```rust
/// The particle demo's step, without the window.
const STEP: &str = "let PULL = 900.0\nlet BOUNCE = 0.7\nstruct Particle {\n pos: vec2\n vel: vec2\n pure step(var self, dt: float, target: vec2, keep: float) {\n  self.vel += (target - self.pos).normalized() * PULL * dt\n  self.vel *= keep\n  self.pos += self.vel * dt\n  if self.pos.x < 0 {\n   self.pos.x = 0\n   self.vel.x = abs(self.vel.x) * BOUNCE\n  } else if self.pos.x > 1280 {\n   self.pos.x = 1280\n   self.vel.x = -abs(self.vel.x) * BOUNCE\n  }\n  if self.pos.y < 0 {\n   self.pos.y = 0\n   self.vel.y = abs(self.vel.y) * BOUNCE\n  } else if self.pos.y > 720 {\n   self.pos.y = 720\n   self.vel.y = -abs(self.vel.y) * BOUNCE\n  }\n }\n}\nrun(var ps: list[Particle], dt: float, target: vec2, keep: float) {\n par for p in ps {\n  p.step(dt, target, keep)\n }\n}\nmain() {}\n";

#[test]
fn the_particle_step_in_a_par_for_uses_no_stack_slots() {
    let m = common::optimized(STEP);
    let body = &m.funcs[m.par_bodies[0].0 as usize].name;
    let code = dump(&m, RELEASE, body).unwrap();
    assert!(!code.contains("explicit_slot"), "{code}");
}
```

In `codegen/tests/common/mod.rs`, replace each `kestrel_kir::inline::inline_small(&mut m);` with `kestrel_kir::optimize(&mut m);`. There are three: `jit_with`, `jit_natives_as` and `optimized`. Do the same in `codegen/tests/natives.rs:171`, and in `driver/src/lib.rs` (`kestrel_kir::inline::inline_small(&mut module);` becomes `kestrel_kir::optimize(&mut module);`).

- [ ] **Step 2: Run them to see them fail**

Run (in `kestrel/`): `cargo test -p kestrel-kir --test scalarize`
Expected: does not compile: "cannot find function `optimize`".

- [ ] **Step 3: Add `LocalKind::Unused` and teach codegen about it**

In `kestrel/kir/src/ir.rs`, add a last variant to `LocalKind`:

```rust
    /// Nothing refers to it any more: a struct `scalarize` split into one local per field.
    Unused,
```

In `kestrel/codegen/src/func/mod.rs`:
- Add a last variant to `enum Slot`:

```rust
    /// A local nothing uses (see `LocalKind::Unused`).
    Unused,
```

- In `lower_func`'s `match local.kind`, add `LocalKind::Unused => Slot::Unused,` after the `Addressed` arm.
- In the parameter loop's `match slots[p.0 as usize]`, add `Slot::Unused => unreachable!("a parameter is used"),`.

In `kestrel/codegen/src/func/place.rs` (`place_addr`), add after the `Slot::Var(_)` arm:

```rust
            Slot::Unused => unreachable!("nothing refers to an unused local"),
```

- [ ] **Step 4: Add `leaves.rs`**

Create `kestrel/kir/src/leaves.rs`:

```rust
//! Plain types: numbers, bools, and structs made only of those and of other plain structs. A
//! plain type's leaves are the scalars at its bottom, found through nested structs, in field
//! order; `scalarize` and `promote` keep each leaf in a local of its own.

use std::ops::Range;

use crate::ir::*;
use crate::layout::StructLayout;

/// Plain structs with more leaves than this stay in memory.
pub const MAX_LEAVES: usize = 16;

/// One scalar at the bottom of a plain type.
#[derive(Clone, Debug, PartialEq)]
pub struct Leaf {
    /// Field indexes from the top of the type down to the scalar (empty for a scalar type).
    pub path: Vec<u32>,
    pub ty: KTy,
}

/// The leaves of `ty` in field order, or `None` when it isn't plain: it holds something other
/// than numbers and bools, has no leaves, or has more than `MAX_LEAVES`.
pub fn leaves(structs: &[StructLayout], ty: KTy) -> Option<Vec<Leaf>> {
    let mut out = Vec::new();
    collect(structs, ty, &mut Vec::new(), &mut out)?;
    (!out.is_empty() && out.len() <= MAX_LEAVES).then_some(out)
}

fn collect(structs: &[StructLayout], ty: KTy, path: &mut Vec<u32>, out: &mut Vec<Leaf>) -> Option<()> {
    match ty {
        KTy::Bool | KTy::U8 | KTy::I32 | KTy::I64 | KTy::F32 | KTy::F64 => out.push(Leaf { path: path.clone(), ty }),
        KTy::Struct(s) => {
            for (i, f) in structs[s.0 as usize].fields.iter().enumerate() {
                path.push(i as u32);
                collect(structs, f.ty, path, out)?;
                path.pop();
                if out.len() > MAX_LEAVES {
                    return None;
                }
            }
        }
        _ => return None,
    }
    Some(())
}

/// Whether `ty` is a plain struct (a plain type that isn't a scalar).
pub fn is_plain_struct(structs: &[StructLayout], ty: KTy) -> bool {
    matches!(ty, KTy::Struct(_)) && leaves(structs, ty).is_some()
}

/// The field indexes of a path made only of `Proj::Field` steps (every path inside a plain
/// type), or `None` for any other step.
pub fn field_path(path: &[Proj]) -> Option<Vec<u32>> {
    path.iter().map(|step| if let Proj::Field(i) = step { Some(*i) } else { None }).collect()
}

/// The leaves under `prefix` (a field path into the type), which are next to each other.
pub fn under(all: &[Leaf], prefix: &[u32]) -> Range<usize> {
    let start = all.iter().position(|l| l.path.starts_with(prefix)).expect("a field path into the type");
    let len = all[start..].iter().take_while(|l| l.path.starts_with(prefix)).count();
    start..start + len
}

/// A place's path extended by a leaf's field steps.
pub fn leaf_place(base: &Place, leaf: &Leaf) -> Place {
    let mut p = base.clone();
    p.path.extend(leaf.path.iter().map(|&i| Proj::Field(i)));
    p
}

/// The value of type `ty` built from the leaf values `vals` (in leaf order): the scalar itself,
/// or a `StructLit` tree.
pub fn rebuild(structs: &[StructLayout], ty: KTy, vals: &mut impl Iterator<Item = Expr>, line: u32) -> Expr {
    match ty {
        KTy::Struct(s) => {
            let fields = structs[s.0 as usize].fields.iter().map(|f| rebuild(structs, f.ty, vals, line)).collect();
            Expr { kind: ExprKind::StructLit(s, fields), ty, line }
        }
        _ => vals.next().expect("a value for every leaf"),
    }
}

/// `name.field.field` for a leaf of a local named `name` of type `ty`.
pub fn leaf_name(structs: &[StructLayout], name: &str, ty: KTy, leaf: &Leaf) -> String {
    let mut out = name.to_string();
    let mut ty = ty;
    for &i in &leaf.path {
        let KTy::Struct(s) = ty else { unreachable!("a leaf path goes through structs") };
        let f = &structs[s.0 as usize].fields[i as usize];
        out.push('.');
        out.push_str(&f.name);
        ty = f.ty;
    }
    out
}

/// A zero of scalar type `ty`.
pub fn zero(ty: KTy, line: u32) -> Expr {
    let kind = match ty {
        KTy::F32 | KTy::F64 => ExprKind::Float(0.0),
        KTy::Bool => ExprKind::Bool(false),
        _ => ExprKind::Int(0),
    };
    Expr { kind, ty, line }
}

/// A load of a local (no path).
pub fn load(l: LocalIdx, ty: KTy, line: u32) -> Expr {
    Expr { kind: ExprKind::Load(Place::local(l)), ty, line }
}
```

- [ ] **Step 5: Add `visit_mut.rs`**

Create `kestrel/kir/src/visit_mut.rs`:

```rust
//! Rewrites KIR in place. A pass implements `VisitMut`, overrides the methods for the nodes it
//! changes, and calls the matching `walk_*` function to carry on into everything else.

use crate::ir::*;

pub trait VisitMut {
    /// A statement list. Override it to replace a statement with several.
    fn visit_stmts(&mut self, list: &mut Vec<Stmt>) {
        for s in list.iter_mut() {
            self.visit_stmt(s);
        }
    }

    fn visit_stmt(&mut self, s: &mut Stmt) {
        walk_stmt(self, s);
    }

    fn visit_expr(&mut self, e: &mut Expr) {
        walk_expr(self, e);
    }

    /// A place: its index expressions.
    fn visit_place(&mut self, p: &mut Place) {
        walk_place(self, p);
    }
}

/// Visits what `s` holds, in the order codegen evaluates it.
pub fn walk_stmt<V: VisitMut + ?Sized>(v: &mut V, s: &mut Stmt) {
    match s {
        Stmt::Let(_, e) | Stmt::Return(Some(e)) | Stmt::RandSeed(e) => v.visit_expr(e),
        Stmt::Assign(p, e) | Stmt::ListAdd(p, e) => {
            v.visit_expr(e);
            v.visit_place(p);
        }
        Stmt::If { cond, then_body, else_body } => {
            v.visit_expr(cond);
            v.visit_stmts(then_body);
            v.visit_stmts(else_body);
        }
        Stmt::While { cond, body } => {
            v.visit_expr(cond);
            v.visit_stmts(body);
        }
        Stmt::ForRange { start, end, body, .. } => {
            v.visit_expr(start);
            v.visit_expr(end);
            v.visit_stmts(body);
        }
        Stmt::Call(_, args) => walk_args(v, args),
        Stmt::Print(parts) | Stmt::Write(parts) => walk_print(v, parts),
        Stmt::Bind(_, p) | Stmt::Dup(p) | Stmt::FreeArena(p) | Stmt::DropGpu(p) | Stmt::PushArena(Some(p)) => v.visit_place(p),
        Stmt::MapSet(p, k, val, _) => {
            v.visit_expr(k);
            v.visit_expr(val);
            v.visit_place(p);
        }
        Stmt::MapUnlock(p, saved) => {
            v.visit_expr(saved);
            v.visit_place(p);
        }
        Stmt::ParFor { args, count, .. } => {
            walk_args(v, args);
            v.visit_expr(count);
        }
        Stmt::BindGlobal(..) | Stmt::PushArena(None) | Stmt::PopArena { .. } | Stmt::SetLine(_) | Stmt::Break | Stmt::Continue | Stmt::Return(None) => {}
    }
}

/// Visits the expressions, statement lists and places directly inside `e`.
pub fn walk_expr<V: VisitMut + ?Sized>(v: &mut V, e: &mut Expr) {
    match &mut e.kind {
        ExprKind::Format(parts) => walk_print(v, parts),
        ExprKind::Load(p) | ExprKind::Tag(p) | ExprKind::Len(p) | ExprKind::Move(p) => v.visit_place(p),
        ExprKind::Map(_, p, xs) => {
            for x in xs {
                v.visit_expr(x);
            }
            v.visit_place(p);
        }
        ExprKind::Unary(_, x) | ExprKind::Cast(x) | ExprKind::FieldOf(x, _) | ExprKind::ArrayNew(x) | ExprKind::Some(x) => v.visit_expr(x),
        ExprKind::Binary(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) | ExprKind::RandInt(a, b) => {
            v.visit_expr(a);
            v.visit_expr(b);
        }
        ExprKind::Call(_, args) => walk_args(v, args),
        ExprKind::StructLit(_, xs) | ExprKind::VariantLit(_, _, xs) | ExprKind::ArrayLit(xs) | ExprKind::Math(_, xs) => {
            for x in xs {
                v.visit_expr(x);
            }
        }
        ExprKind::Seq(stmts, value) => {
            v.visit_stmts(stmts);
            v.visit_expr(value);
        }
        ExprKind::If { cond, then_body, then_val, else_body, else_val } => {
            v.visit_expr(cond);
            v.visit_stmts(then_body);
            if let Some(x) = then_val {
                v.visit_expr(x);
            }
            v.visit_stmts(else_body);
            if let Some(x) = else_val {
                v.visit_expr(x);
            }
        }
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::StrLit(_)
        | ExprKind::ListNew
        | ExprKind::MapNew
        | ExprKind::NewArena
        | ExprKind::NoneLit
        | ExprKind::RandFloat => {}
    }
}

pub fn walk_place<V: VisitMut + ?Sized>(v: &mut V, p: &mut Place) {
    for at in p.path.iter_mut().filter_map(Proj::at_mut) {
        v.visit_expr(at);
    }
}

fn walk_args<V: VisitMut + ?Sized>(v: &mut V, args: &mut [Arg]) {
    for a in args {
        match a {
            Arg::Value(e) => v.visit_expr(e),
            Arg::Ref(p) => v.visit_place(p),
        }
    }
}

fn walk_print<V: VisitMut + ?Sized>(v: &mut V, parts: &mut [PrintArg]) {
    for p in parts {
        match p {
            PrintArg::Value(e) => v.visit_expr(e),
            PrintArg::Interp(inner) => walk_print(v, inner),
            PrintArg::Str(_) => {}
        }
    }
}
```

- [ ] **Step 6: Add the rewriter, `split.rs`**

Create `kestrel/kir/src/split.rs`:

```rust
//! The rewrite `scalarize` and `promote` share. Every place rooted at a chosen local (a **split
//! root**) becomes the local holding that leaf, and every store of a plain struct value is done
//! one leaf at a time, with all of the value's leaves computed before any is stored.

use std::collections::HashMap;

use crate::ir::*;
use crate::layout::StructLayout;
use crate::leaves::{field_path, is_plain_struct, leaf_place, leaves, load, rebuild, under, zero, Leaf};
use crate::visit_mut::{walk_expr, walk_place, walk_stmt, VisitMut};

/// A split root's leaves and the local that holds each.
pub(crate) struct Split {
    pub leaves: Vec<Leaf>,
    pub locals: Vec<LocalIdx>,
}

pub(crate) struct Rewriter<'a> {
    pub structs: &'a [StructLayout],
    pub locals: &'a mut Vec<Local>,
    pub roots: HashMap<LocalIdx, Split>,
}

impl Rewriter<'_> {
    pub(crate) fn temp(&mut self, name: &str, ty: KTy, kind: LocalKind) -> LocalIdx {
        self.locals.push(Local { name: name.to_string(), ty, kind });
        LocalIdx(self.locals.len() as u32 - 1)
    }

    /// The local holding the leaf at `p`, when `p` is rooted at a split root.
    fn leaf_local(&self, p: &Place) -> Option<LocalIdx> {
        let split = self.roots.get(&p.root)?;
        let path = field_path(&p.path).expect("a split root's places are field paths");
        let k = split.leaves.iter().position(|l| l.path == path).expect("a scalar place is a leaf");
        Some(split.locals[k])
    }

    /// The value at `p` (rooted at a split root): its leaf local, or a `StructLit` of them.
    fn read(&self, p: &Place, ty: KTy, line: u32) -> Expr {
        let split = &self.roots[&p.root];
        let path = field_path(&p.path).expect("a split root's places are field paths");
        let range = under(&split.leaves, &path);
        let mut vals = range.map(|k| load(split.locals[k], split.leaves[k].ty, line));
        rebuild(self.structs, ty, &mut vals, line)
    }

    /// Whether `e` is a plain struct value that is copied as a whole between two places in
    /// memory, the one struct store that isn't split.
    fn memory_copy(&self, e: &Expr) -> bool {
        matches!(&e.kind, ExprKind::Load(q) if !self.roots.contains_key(&q.root))
    }

    /// `e` in a fresh local, unless it is a constant: the result can be read later, after other
    /// code has run, and still give `e`'s value.
    fn hold(&mut self, mut e: Expr, out: &mut Vec<Stmt>) -> Expr {
        self.visit_expr(&mut e);
        if matches!(e.kind, ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_)) {
            return e;
        }
        let (ty, line) = (e.ty, e.line);
        let t = self.temp("leaf", ty, LocalKind::Value);
        out.push(Stmt::Let(t, e));
        load(t, ty, line)
    }

    /// `p` with every index expression that isn't a constant or a local evaluated now, into a
    /// local, so the place can be used several times and still mean one element.
    fn stable(&mut self, mut p: Place, out: &mut Vec<Stmt>) -> Place {
        for at in p.path.iter_mut().filter_map(Proj::at_mut) {
            let simple = match &at.kind {
                ExprKind::Int(_) => true,
                ExprKind::Load(q) => q.path.is_empty() && self.locals[q.root.0 as usize].kind == LocalKind::Value,
                _ => false,
            };
            if !simple {
                let (ty, line) = (at.ty, at.line);
                let t = self.temp("index", ty, LocalKind::Value);
                let v = std::mem::replace(at, load(t, ty, line));
                out.push(Stmt::Let(t, v));
            }
        }
        p
    }

    /// The leaves of `e`, a plain struct value, in leaf order. The code computing them goes to
    /// `out` in the order codegen evaluates `e`; each leaf is a constant or a load of a local that
    /// holds it.
    pub(crate) fn split(&mut self, e: Expr, out: &mut Vec<Stmt>) -> Vec<Expr> {
        let Expr { kind, ty, line } = e;
        match kind {
            ExprKind::StructLit(_, fields) => {
                let mut vals = Vec::new();
                for f in fields {
                    if f.ty.is_aggregate() {
                        vals.extend(self.split(f, out));
                    } else {
                        vals.push(self.hold(f, out));
                    }
                }
                vals
            }
            ExprKind::Load(mut p) => {
                self.visit_place(&mut p);
                let all = leaves(self.structs, ty).expect("a plain struct");
                if self.roots.contains_key(&p.root) {
                    let split = &self.roots[&p.root];
                    let range = under(&split.leaves, &field_path(&p.path).expect("a field path"));
                    let reads: Vec<Expr> = range.map(|k| load(split.locals[k], split.leaves[k].ty, line)).collect();
                    return reads.into_iter().map(|r| self.hold(r, out)).collect();
                }
                let p = self.stable(p, out);
                all.iter().map(|l| self.hold(Expr { kind: ExprKind::Load(leaf_place(&p, l)), ty: l.ty, line }, out)).collect()
            }
            ExprKind::FieldOf(base, i) => {
                let base_ty = base.ty;
                let vals = self.split(*base, out);
                let all = leaves(self.structs, base_ty).expect("a plain struct");
                vals[under(&all, &[i])].to_vec()
            }
            ExprKind::Seq(stmts, value) => {
                let mut stmts = stmts;
                self.visit_stmts(&mut stmts);
                out.extend(stmts);
                self.split(*value, out)
            }
            ExprKind::If { mut cond, then_body, then_val, else_body, else_val } => {
                self.visit_expr(&mut cond);
                let all = leaves(self.structs, ty).expect("a plain struct");
                let temps: Vec<LocalIdx> = all
                    .iter()
                    .map(|l| {
                        let t = self.temp("leaf", l.ty, LocalKind::Value);
                        out.push(Stmt::Let(t, zero(l.ty, line)));
                        t
                    })
                    .collect();
                let then_body = self.branch(then_body, then_val, &temps);
                let else_body = self.branch(else_body, else_val, &temps);
                out.push(Stmt::If { cond: *cond, then_body, else_body });
                temps.iter().zip(&all).map(|(&t, l)| load(t, l.ty, line)).collect()
            }
            kind => {
                // A call's result, or any other value built in memory: keep it in a temporary.
                let mut e = Expr { kind, ty, line };
                self.visit_expr(&mut e);
                let t = self.temp("value", ty, LocalKind::Addressed);
                out.push(Stmt::Let(t, e));
                let at = Place::local(t);
                let all = leaves(self.structs, ty).expect("a plain struct");
                all.iter().map(|l| Expr { kind: ExprKind::Load(leaf_place(&at, l)), ty: l.ty, line }).collect()
            }
        }
    }

    /// One branch of a struct-valued `if`: its statements, then its value's leaves stored in `temps`.
    /// A branch without a value never finishes, so it stores nothing.
    fn branch(&mut self, body: Vec<Stmt>, val: Option<Box<Expr>>, temps: &[LocalIdx]) -> Vec<Stmt> {
        let mut body = body;
        self.visit_stmts(&mut body);
        if let Some(v) = val {
            let vals = self.split(*v, &mut body);
            for (&t, v) in temps.iter().zip(vals) {
                body.push(Stmt::Assign(Place::local(t), v));
            }
        }
        body
    }

    /// `dest = e`, rewritten: a leaf of a split root is stored in its local, and a plain struct
    /// value is stored one leaf at a time after all of its leaves are computed. `dest`'s index
    /// expressions run after `e`, as codegen orders a store.
    fn store(&mut self, dest: Place, e: Expr, out: &mut Vec<Stmt>) {
        let mut dest = dest;
        if !e.ty.is_aggregate() {
            let mut e = e;
            self.visit_expr(&mut e);
            self.visit_place(&mut dest);
            let dest = match self.leaf_local(&dest) {
                Some(l) => Place::local(l),
                None => dest,
            };
            out.push(Stmt::Assign(dest, e));
            return;
        }
        let ty = e.ty;
        let vals = self.split(e, out);
        self.visit_place(&mut dest);
        let dest = self.stable(dest, out);
        for (leaf, v) in leaves(self.structs, ty).expect("a plain struct").iter().zip(vals) {
            let p = leaf_place(&dest, leaf);
            let p = match self.leaf_local(&p) {
                Some(l) => Place::local(l),
                None => p,
            };
            out.push(Stmt::Assign(p, v));
        }
    }

    fn stmt(&mut self, s: Stmt, out: &mut Vec<Stmt>) {
        match s {
            Stmt::Let(l, e) if self.roots.contains_key(&l) => self.store(Place::local(l), e, out),
            Stmt::Assign(p, e) if self.roots.contains_key(&p.root) => self.store(p, e, out),
            Stmt::Assign(p, e) if is_plain_struct(self.structs, e.ty) && !self.memory_copy(&e) => self.store(p, e, out),
            mut s => {
                walk_stmt(self, &mut s);
                out.push(s);
            }
        }
    }
}

impl VisitMut for Rewriter<'_> {
    fn visit_stmts(&mut self, list: &mut Vec<Stmt>) {
        let mut out = Vec::with_capacity(list.len());
        for s in std::mem::take(list) {
            self.stmt(s, &mut out);
        }
        *list = out;
    }

    fn visit_expr(&mut self, e: &mut Expr) {
        match &mut e.kind {
            ExprKind::Load(p) if self.roots.contains_key(&p.root) => {
                walk_place(self, p);
                *e = self.read(p, e.ty, e.line);
            }
            ExprKind::FieldOf(base, i) if is_plain_struct(self.structs, base.ty) => {
                let (i, line, ty) = (*i, e.line, e.ty);
                let base = std::mem::replace(&mut **base, zero(KTy::Bool, line));
                let base_ty = base.ty;
                let mut pre = Vec::new();
                let vals = self.split(base, &mut pre);
                let all = leaves(self.structs, base_ty).expect("a plain struct");
                let value = rebuild(self.structs, ty, &mut vals[under(&all, &[i])].iter().cloned(), line);
                *e = if pre.is_empty() { value } else { Expr { kind: ExprKind::Seq(pre, Box::new(value)), ty, line } };
            }
            _ => walk_expr(self, e),
        }
    }
}
```

- [ ] **Step 7: Add `scalarize.rs` and `optimize`**

Create `kestrel/kir/src/scalarize.rs`:

```rust
//! Keeps plain structs in registers. After inlining, a local of a plain struct type that nothing
//! needs the address of is split into one `Value` local per leaf (`vec2` into `.x` and `.y`), and
//! every struct store becomes one store per leaf (see `split`). A number that had to live in
//! memory only for a `var` argument that inlining removed goes back to being a `Value`.

use std::collections::{HashMap, HashSet};

use crate::ir::*;
use crate::leaves::{leaf_name, leaves};
use crate::split::{Rewriter, Split};
use crate::visit::{walk, Node};
use crate::visit_mut::VisitMut;

pub fn scalarize(m: &mut Module) {
    let Module { funcs, structs, .. } = m;
    for f in funcs.iter_mut().filter(|f| f.native.is_none()) {
        let pinned = pinned(&f.body);
        let mut roots = HashMap::new();
        for i in 0..f.locals.len() {
            let l = LocalIdx(i as u32);
            let local = &f.locals[i];
            if local.kind != LocalKind::Addressed || pinned.contains(&l) {
                continue;
            }
            let Some(all) = leaves(structs, local.ty) else { continue };
            if !local.ty.is_aggregate() {
                f.locals[i].kind = LocalKind::Value;
                continue;
            }
            let (name, ty) = (local.name.clone(), local.ty);
            let locals = all
                .iter()
                .map(|leaf| {
                    f.locals.push(Local { name: leaf_name(structs, &name, ty, leaf), ty: leaf.ty, kind: LocalKind::Value });
                    LocalIdx(f.locals.len() as u32 - 1)
                })
                .collect();
            f.locals[i].kind = LocalKind::Unused;
            roots.insert(l, Split { leaves: all, locals });
        }
        if roots.is_empty() {
            continue;
        }
        let mut rw = Rewriter { structs, locals: &mut f.locals, roots };
        rw.visit_stmts(&mut f.body);
    }
}

/// The locals whose address something needs: passed as `var` (to a call or a `par for`), bound
/// by a loop, or used whole by an operation on memory (moved, dropped, measured, ...).
fn pinned(body: &[Stmt]) -> HashSet<LocalIdx> {
    let mut out = HashSet::new();
    let args = |args: &[Arg], out: &mut HashSet<LocalIdx>| {
        for a in args {
            if let Arg::Ref(p) = a {
                out.insert(p.root);
            }
        }
    };
    walk(body, &mut |n| match n {
        Node::Stmt(Stmt::Call(_, a)) | Node::Stmt(Stmt::ParFor { args: a, .. }) => args(a, &mut out),
        Node::Expr(Expr { kind: ExprKind::Call(_, a), .. }) => args(a, &mut out),
        Node::Stmt(
            Stmt::Bind(_, p)
            | Stmt::ListAdd(p, _)
            | Stmt::Dup(p)
            | Stmt::FreeArena(p)
            | Stmt::DropGpu(p)
            | Stmt::PushArena(Some(p))
            | Stmt::MapSet(p, ..)
            | Stmt::MapUnlock(p, _),
        ) => {
            out.insert(p.root);
        }
        Node::Expr(Expr { kind: ExprKind::Move(p) | ExprKind::Len(p) | ExprKind::Tag(p) | ExprKind::Map(_, p, _), .. }) => {
            out.insert(p.root);
        }
        _ => {}
    });
    out
}
```

In `kestrel/kir/src/lib.rs`, make the module list:

```rust
pub mod inline;
pub mod ir;
mod layout;
mod leaves;
mod lower;
mod native;
mod ops;
mod rename;
pub mod scalarize;
mod split;
pub mod visit;
pub mod visit_mut;
```

Then add at the end:

```rust
/// Inlines small functions, then keeps plain structs in registers (see `scalarize`). The driver
/// runs it on every program.
pub fn optimize(m: &mut Module) {
    inline::inline_small(m);
    scalarize::scalarize(m);
}
```

- [ ] **Step 8: Run the new tests to see them pass**

Run: `cargo test -p kestrel-kir --test scalarize` and `cargo test -p kestrel-codegen --test dump`
Expected: 5 passed, 3 passed.

- [ ] **Step 9: Add the program tests (they guard the output, so they pass before and after)**

Create `kestrel/tests/programs/scalarize_swizzle.kes`:

```
// expect: vec2(x: 2, y: 1)
// expect: vec3(x: 3, y: 2, z: 1)
// expect: vec3(x: 5, y: 3, z: 4)
// expect: vec2(x: 5, y: 5) vec2(x: 4, y: 3)
// expect: 5 100
// expect: vec2(x: 100, y: 5) vec2(x: 3, y: 4) vec2(x: 3, y: 4)
// Struct values that read the place they replace: every field is read before any is stored.
struct Body {
    pos: vec2
    vel: vec2
}

main() {
    var v = vec2(1, 2)
    v = v.yx
    print(v)
    var w = vec3(1, 2, 3)
    w = w.zyx
    print(w)
    w = w.yzx + w
    print(w)
    var b = Body(pos: vec2(1, 2), vel: vec2(3, 4))
    b.vel = b.vel.yx
    b.pos = b.pos + b.vel
    print(b.pos, b.vel)
    // A copy is its own value.
    var c = b
    c.pos.x = 100
    print(b.pos.x, c.pos.x)
    var bs = [b, c]
    bs[1].vel = bs[1].vel.yx
    bs[0] = bs[1]
    bs[1].pos = bs[0].vel
    print(bs[0].pos, bs[0].vel, bs[1].pos)
}
```

Create `kestrel/tests/programs/scalarize_if.kes`:

```
// expect: vec2(x: 2, y: 4) vec2(x: 6, y: 8)
// expect: vec2(x: 0, y: 0) vec2(x: 0.6, y: 0.8)
// expect: Pair(a: vec2(x: 4, y: 6), b: vec2(x: 2, y: 3))
// expect: Pair(a: vec2(x: 4, y: 6), b: vec2(x: 4, y: 1))
// expect: eval 2
// expect: eval 0
// expect: vec2(x: 6, y: 8)
// expect: eval 1
// expect: eval 2
// expect: Pair(a: vec2(x: 3, y: 4), b: vec2(x: 6, y: 5))
// expect: 3 -1
// Struct values made by `if`, by vector methods, and from places whose index has side effects.
struct Pair {
    a: vec2
    b: vec2
}

noisy(n: int) -> int {
    print("eval", n)
    n
}

pick(c: bool) -> vec2 {
    let v = if c { vec2(1, 2) } else { vec2(3, 4) }
    v * 2
}

first_or_stop(xs: [vec2]) -> float {
    let v = if xs.len > 0 { xs[0] } else { return -1.0 }
    v.x + v.y
}

main() {
    print(pick(true), pick(false))
    print(vec2(0, 0).normalized(), vec2(3, 4).normalized())
    var p = Pair(a: vec2(1, 1), b: vec2(2, 3))
    p.a = if p.a.x > 0 { p.b * 2 } else { p.b }
    print(p)
    p.b = if p.a.x > 100 { p.b } else { p.a.yx - p.b }
    print(p)
    let vs = [vec2(1, 2), vec2(3, 4), vec2(5, 6)]
    let s = vs[noisy(2)] + vs[noisy(0)]
    print(s)
    var q = Pair(a: vs[noisy(1)], b: vs[noisy(2)].yx)
    print(q)
    print(first_or_stop([vec2(1, 2)]), first_or_stop(array[vec2](0)))
}
```

Create `kestrel/tests/programs/scalarize_calls.kes`:

```
// expect: 5000000009.25
// expect: Cell(solid: true, kind: 7, n: 4, big: 15000000000, w: 0.25, at: vec3(x: 2, y: 4, z: 6))
// expect: cell 4 15000000000 vec3(x: 2, y: 4, z: 6)
// expect: false 8
// expect: 4 2
// expect: 25
// Plain structs split into one local per field, crossing calls, prints, lists and `var`.
struct Cell {
    solid: bool
    kind: u8
    n: int
    big: i64
    w: f64
    at: vec3
}

// More than 16 fields at the bottom: stays in memory.
struct Wide {
    a: vec3
    b: vec3
    c: vec3
    d: vec3
    e: vec3
    f: vec2
}

pure sum(c: Cell) -> f64 {
    c.w + c.at.x + c.at.y + c.at.z + c.n + c.big
}

pure grow(c: Cell) -> Cell {
    var d = c
    d.n += 1
    d.big *= 3
    d.at = d.at * 2
    d
}

bump(var c: Cell) {
    c.kind += 1
    c.solid = not c.solid
}

pure total(w: Wide) -> float {
    w.a.x + w.b.y + w.c.z + w.d.x + w.e.y + w.f.x + w.f.y
}

main() {
    var c = Cell(solid: true, kind: 7, n: 3, big: 5000000000, w: 0.25, at: vec3(1, 2, 3))
    print(sum(c))
    let g = grow(c)
    print(g)
    print("cell {g.n} {g.big} {g.at}")
    bump(var c)
    print(c.solid, c.kind)
    var cells = list[Cell]()
    cells.add(c)
    cells.add(grow(c))
    print(cells[1].n, cells.len)
    var wide = Wide(a: vec3(1, 2, 3), b: vec3(4, 5, 6), c: vec3(7, 8, 9), d: vec3(1, 1, 1), e: vec3(2, 2, 2), f: vec2(3, 3))
    wide.f = wide.f.yx + vec2(1, 0)
    print(total(wide))
}
```

Create `kestrel/tests/programs/scalarize_payloads.kes`:

```
// expect: Shape.Dot(at: vec2(x: 11, y: 22))
// expect: 5
// expect: Shape.Line(a: vec2(x: 1, y: 1), b: vec2(x: 5, y: 4)) 5
// expect: Holder(spot: vec2(x: 6, y: 4))
// expect: vec2(x: 2, y: 1) none
// expect: vec2(x: 3, y: 2)
// Plain structs stored inside enum variants and optionals, built from values kept in registers.
enum Shape {
    Dot { at: vec2 }
    Line { a: vec2, b: vec2 }

    pure length(self) -> float {
        match self {
            Line l { l.a.distance_to(l.b) }
            else { 0 }
        }
    }

    shift(var self, by: vec2) {
        match self {
            Dot d { d.at = d.at + by }
            Line l {
                l.a = l.a + by
                l.b = l.b.yx + by
            }
        }
    }
}

struct Holder {
    spot: vec2?
}

pure middle(a: vec2, b: vec2) -> vec2? {
    if a.x > b.x { none } else { (a + b) * 0.5 }
}

main() {
    var d = Shape.Dot(at: vec2(1, 2))
    d.shift(vec2(10, 20))
    print(d)
    var l = Shape.Line(a: vec2(0, 0), b: vec2(3, 4))
    print(l.length())
    l.shift(vec2(1, 1))
    print(l, l.length())
    var h = Holder(spot: none)
    let v = vec2(2, 3) * 2
    h.spot = v.yx
    print(h)
    let m = middle(vec2(0, 0), vec2(4, 2)) else { return }
    print(m, middle(vec2(5, 0), vec2(1, 1)))
    var maybe: vec2? = none
    maybe = m + vec2(1, 1)
    print(maybe)
}
```

- [ ] **Step 10: Run everything**

Run (in `kestrel/`): `cargo test --workspace --no-fail-fast` and `cargo clippy --workspace --all-targets -- -D warnings`.
Expected: every test passes (the program tests run both through the JIT and as built exes), and clippy is clean.

Then rebuild the benchmark harness (`cargo build --release` in `bench/kernels`) and run `target/release/kernels.exe step.kes --release --dump par`. Expected: no `explicit_slot` lines. Run `py measure.py --rounds 1`. Expected: `step.kes` about 5.5 ns at one thread, down from about 12.8. This is the spike's "scalar temporaries only" figure; Task 3 brings it to about 4.3.

- [ ] **Step 11: Commit**

```bash
git add kestrel/kir kestrel/codegen kestrel/driver kestrel/tests/programs
git commit -m "Keep plain structs in registers: split them into one local per field after inlining" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git push
```

---

### Task 3: `promote`: the fields of `var` parameters and loop elements in registers

**Files:**
- Create: `kestrel/kir/src/promote.rs`
- Modify: `kestrel/kir/src/lib.rs` (module and `optimize`)
- Create: `kestrel/kir/tests/promote.rs`
- Create: `kestrel/tests/programs/promote_var_self.kes`, `promote_loops.kes`, `promote_panic.kes`

**Interfaces:**
- Consumes: `Rewriter`, `Split` (`split.rs`); `leaves`, `leaf_name`, `leaf_place`, `field_path`, `under`, `load`, `Leaf` (`leaves.rs`); `VisitMut`, `walk_stmt` (`visit_mut.rs`), all from Task 2.
- Produces: `pub fn kestrel_kir::promote::promote(m: &mut Module)`. `optimize` becomes inline, then `scalarize`, then `promote`.

- [ ] **Step 1: Write the failing tests**

Create `kestrel/kir/tests/promote.rs`:

```rust
use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn optimized(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = lower(&p, &t.unwrap(), &LineIndex::new(src));
    optimize(&mut m);
    m
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

fn local(f: &Func, name: &str) -> LocalIdx {
    LocalIdx(f.locals.iter().position(|l| l.name == name).unwrap() as u32)
}

/// How many places in `stmts` are rooted at `root`: (read, written).
fn uses(stmts: &[Stmt], root: LocalIdx) -> (usize, usize) {
    let (mut read, mut written) = (0, 0);
    visit::places(stmts, &mut |p, w| {
        if p.root == root {
            if w {
                written += 1;
            } else {
                read += 1;
            }
        }
    });
    (read, written)
}

const STEP: &str = "struct Particle {\n pos: vec2\n vel: vec2\n pure step(var self, dt: float, target: vec2) {\n  self.vel += (target - self.pos).normalized() * 900.0 * dt\n  self.pos += self.vel * dt\n  if self.pos.x < 0 {\n   self.pos.x = 0\n   self.vel.x = abs(self.vel.x) * 0.7\n  }\n }\n}\nrun(var ps: list[Particle], dt: float, target: vec2) {\n par for p in ps {\n  p.step(dt, target)\n }\n}\nmain() {}\n";

#[test]
fn a_par_for_item_is_loaded_once_and_written_back_once_per_pass() {
    let m = optimized(STEP);
    let body = &m.funcs[m.par_bodies[0].0 as usize];
    // Four fields loaded after the `Bind`, and four written back at the end of the pass.
    assert_eq!(uses(&body.body, local(body, "p_ref")), (4, 4), "{:#?}", body.body);
}

/// The statements of `f` before its first loop.
fn before_loop(f: &Func) -> Vec<Stmt> {
    f.body.iter().take_while(|s| !matches!(s, Stmt::ForRange { .. } | Stmt::While { .. })).cloned().collect()
}

#[test]
fn captured_values_are_loaded_once_per_par_for_chunk() {
    let m = optimized(STEP);
    let body = &m.funcs[m.par_bodies[0].0 as usize];
    // `dt`, and `target`'s two fields, are loaded before the loop over the chunk.
    assert_eq!(uses(&before_loop(body), local(body, "dt")), (1, 0), "{:#?}", body.body);
    assert_eq!(uses(&before_loop(body), local(body, "target")), (2, 0));
}

#[test]
fn var_self_is_written_back_before_each_return() {
    let src = "struct C {\n n: int\n m: int\n count(var self, limit: int) -> int {\n  while self.n < limit {\n   self.n += 1\n   if self.n == 5 {\n    return 5\n   }\n  }\n  self.m = self.n\n  self.n\n }\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "C.count");
    // Both fields loaded at the start; both written back before the early `return`, before the
    // final one, and at the end of the body (dead, after the final `return`).
    assert_eq!(uses(&f.body, f.params[0]), (2, 6), "{:#?}", f.body);
}

#[test]
fn a_loop_that_reads_its_list_keeps_the_item_in_memory() {
    let src = "struct P {\n x: float\n}\nmain() {\n var ps = [P(x: 1), P(x: 2)]\n for i, p in ps {\n  p.x = ps[0].x + i\n }\n print(ps)\n}\n";
    let m = optimized(src);
    let main = func(&m, "main");
    assert_eq!(uses(&main.body, local(main, "p_ref")), (0, 1), "{:#?}", main.body);
}

#[test]
fn var_self_passed_on_as_var_stays_in_memory() {
    // Both methods return early, so neither is inlined, and `twice` passes `self` on as `var`.
    let src = "struct C {\n n: int\n inc(var self) {\n  if self.n > 9 {\n   return\n  }\n  self.n += 1\n }\n twice(var self) {\n  if self.n < 0 {\n   return\n  }\n  self.inc()\n  self.n += 1\n  self.inc()\n }\n}\nmain() {}\n";
    let m = optimized(src);
    assert!(!func(&m, "C.twice").locals.iter().any(|l| l.name.starts_with("self.")));
    assert!(func(&m, "C.inc").locals.iter().any(|l| l.name == "self.n"));
}

#[test]
fn a_read_only_struct_parameter_is_loaded_once_and_never_written() {
    let src = "pure spread(a: vec2, b: vec2) -> float {\n var t = 0.0\n for i from 0 to 4 {\n  t += a.x * i + a.y - b.x * b.y\n }\n t\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "spread");
    // Each field is loaded once, before the loop, and nothing is written back.
    assert_eq!(uses(&before_loop(f), f.params[0]), (2, 0), "{:#?}", f.body);
    assert_eq!(uses(&before_loop(f), f.params[1]), (2, 0));
    assert_eq!(uses(&f.body, f.params[0]), (2, 0));
    assert_eq!(uses(&f.body, f.params[1]), (2, 0));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p kestrel-kir --test promote`
Expected: 5 of the 6 fail. The fields are still read from memory where they're used, so the counts differ, and `C.inc` has no `self.n` local yet. Only `a_loop_that_reads_its_list_keeps_the_item_in_memory` passes already: it checks that a place stays in memory.

- [ ] **Step 3: Add `promote.rs`**

Create `kestrel/kir/src/promote.rs`:

```rust
//! Keeps the fields of an exclusive place in registers for a region of code, so a field written
//! there isn't read back from memory. The places are pointer parameters of plain type (`var self`,
//! a `var` number, a read-only struct), for the whole body, and loop elements of plain type
//! (`for p in ps`, a `par for` item), from their `Bind` to the end of the loop body. Each field the
//! region uses is loaded once at its start, and each field it stores is written back at every
//! way out: the end, `return`, and a `break` or `continue` of the element's loop.
//!
//! Only one route reaches such a place while the region runs: the checker refuses a `var`
//! argument whose variable appears in another argument of the same call, and there are no
//! global variables that change. A region that reaches the place another way (passing it on as
//! `var`, a `par for`, or for a loop element its collection or another element of it) is left
//! alone. A runtime panic loses the fields not yet written back, which nothing can observe.

use std::collections::{HashMap, HashSet};

use crate::ir::*;
use crate::layout::StructLayout;
use crate::leaves::{field_path, leaf_name, leaf_place, leaves, load, under, Leaf};
use crate::split::{Rewriter, Split};
use crate::visit::{walk, walk_expr, Node};
use crate::visit_mut::{walk_stmt, VisitMut};

pub fn promote(m: &mut Module) {
    let Module { funcs, structs, .. } = m;
    for f in funcs.iter_mut().filter(|f| f.native.is_none()) {
        let homes = bind_roots(&f.body);
        let mut body = std::mem::take(&mut f.body);
        Elements { structs, locals: &mut f.locals, homes: &homes }.visit_stmts(&mut body);
        for &p in &f.params {
            if f.locals[p.0 as usize].kind == LocalKind::RefParam {
                body = region(structs, &mut f.locals, &homes, p, body, false);
            }
        }
        f.body = body;
    }
}

/// Each loop element's `Bind`: the root of the place it points at.
fn bind_roots(body: &[Stmt]) -> HashMap<LocalIdx, LocalIdx> {
    let mut out = HashMap::new();
    walk(body, &mut |n| {
        if let Node::Stmt(Stmt::Bind(l, p)) = n {
            out.insert(*l, p.root);
        }
    });
    out
}

/// The local whose memory `l` points into, following loop elements to what they belong to.
fn home(mut l: LocalIdx, homes: &HashMap<LocalIdx, LocalIdx>) -> LocalIdx {
    for _ in 0..homes.len() {
        match homes.get(&l) {
            Some(&up) => l = up,
            None => break,
        }
    }
    l
}

/// Promotes every loop element in the rest of the statement list after its `Bind`.
struct Elements<'a> {
    structs: &'a [StructLayout],
    locals: &'a mut Vec<Local>,
    homes: &'a HashMap<LocalIdx, LocalIdx>,
}

impl VisitMut for Elements<'_> {
    fn visit_stmts(&mut self, list: &mut Vec<Stmt>) {
        // Inner loops first.
        for s in list.iter_mut() {
            self.visit_stmt(s);
        }
        let mut i = 0;
        while i < list.len() {
            if let Stmt::Bind(l, _) = list[i] {
                let end = list[i + 1..].iter().position(|s| matches!(s, Stmt::Bind(m, _) if *m == l)).map_or(list.len(), |k| i + 1 + k);
                let rest: Vec<Stmt> = list.drain(i + 1..end).collect();
                let done = region(self.structs, self.locals, self.homes, l, rest, true);
                let n = done.len();
                list.splice(i + 1..i + 1, done);
                i += n;
            }
            i += 1;
        }
    }
}

/// `stmts` with `root`'s fields kept in locals, when `root` is plain and nothing else reaches
/// its memory there. `in_loop` says `stmts` is the rest of a loop body, which a `break` or
/// `continue` of that loop leaves.
fn region(
    structs: &[StructLayout],
    locals: &mut Vec<Local>,
    homes: &HashMap<LocalIdx, LocalIdx>,
    root: LocalIdx,
    stmts: Vec<Stmt>,
    in_loop: bool,
) -> Vec<Stmt> {
    let (name, ty) = (locals[root.0 as usize].name.clone(), locals[root.0 as usize].ty);
    let Some(all) = leaves(structs, ty) else { return stmts };
    if !exclusive(&stmts, root, homes) {
        return stmts;
    }
    let (used, dirty) = usage(&stmts, root, &all);
    if used.is_empty() {
        return stmts;
    }
    let leaf_locals: Vec<LocalIdx> = all
        .iter()
        .map(|leaf| {
            locals.push(Local { name: leaf_name(structs, &name, ty, leaf), ty: leaf.ty, kind: LocalKind::Value });
            LocalIdx(locals.len() as u32 - 1)
        })
        .collect();
    let mut body = stmts;
    let roots = HashMap::from([(root, Split { leaves: all.clone(), locals: leaf_locals.clone() })]);
    Rewriter { structs, locals, roots }.visit_stmts(&mut body);
    let at = Place::local(root);
    // The loads and write-backs can't fail, so they need no source line.
    let line = 0;
    let mut out: Vec<Stmt> = used
        .iter()
        .map(|&k| Stmt::Let(leaf_locals[k], Expr { kind: ExprKind::Load(leaf_place(&at, &all[k])), ty: all[k].ty, line }))
        .collect();
    if !dirty.is_empty() {
        let back: Vec<Stmt> = dirty.iter().map(|&k| Stmt::Assign(leaf_place(&at, &all[k]), load(leaf_locals[k], all[k].ty, line))).collect();
        let written: HashSet<LocalIdx> = dirty.iter().map(|&k| leaf_locals[k]).collect();
        Exits { back: &back, written: &written, locals, in_loop, depth: 0 }.visit_stmts(&mut body);
        // Dead after a final `return`, `break` or `continue`; codegen skips it then.
        body.extend(back);
    }
    out.extend(body);
    out
}

/// Whether `stmts` reach `root`'s memory only through plain reads and writes of its fields: it
/// isn't passed as `var`, captured by a `par for` or bound, and no other local into the same
/// memory (for a loop element: its collection, or another element of it) is used.
fn exclusive(stmts: &[Stmt], root: LocalIdx, homes: &HashMap<LocalIdx, LocalIdx>) -> bool {
    let mine = home(root, homes);
    let mut ok = true;
    let refs = |args: &[Arg]| args.iter().any(|a| matches!(a, Arg::Ref(p) if p.root == root));
    walk(stmts, &mut |n| match n {
        Node::Place(p, _) if p.root != root && home(p.root, homes) == mine => ok = false,
        Node::Stmt(Stmt::Call(_, a) | Stmt::ParFor { args: a, .. }) | Node::Expr(Expr { kind: ExprKind::Call(_, a), .. }) if refs(a) => ok = false,
        Node::Stmt(Stmt::Bind(_, p)) if p.root == root => ok = false,
        _ => {}
    });
    ok
}

/// The leaves `stmts` read or write, and the ones they write, by index into the root's leaves.
fn usage(stmts: &[Stmt], root: LocalIdx, all: &[Leaf]) -> (Vec<usize>, Vec<usize>) {
    let (mut used, mut dirty) = (vec![false; all.len()], vec![false; all.len()]);
    crate::visit::places(stmts, &mut |p, written| {
        if p.root == root {
            for k in under(all, &field_path(&p.path).expect("a plain place's path is fields")) {
                used[k] = true;
                dirty[k] |= written;
            }
        }
    });
    let pick = |v: &[bool]| v.iter().enumerate().filter(|(_, &b)| b).map(|(k, _)| k).collect();
    (pick(&used), pick(&dirty))
}

/// Puts `back` (the write-backs) before every way out of a region.
struct Exits<'a> {
    back: &'a [Stmt],
    /// The locals of the fields written back: a `return` value that stores to one is computed first.
    written: &'a HashSet<LocalIdx>,
    locals: &'a mut Vec<Local>,
    in_loop: bool,
    /// Loops entered inside the region.
    depth: u32,
}

impl Exits<'_> {
    fn stores_written(&self, e: &Expr) -> bool {
        let mut found = false;
        walk_expr(e, &mut |n| {
            if let Node::Stmt(Stmt::Assign(p, _)) = n {
                found |= self.written.contains(&p.root);
            }
        });
        found
    }
}

impl VisitMut for Exits<'_> {
    fn visit_stmts(&mut self, list: &mut Vec<Stmt>) {
        let mut out = Vec::with_capacity(list.len());
        for mut s in std::mem::take(list) {
            self.visit_stmt(&mut s);
            match s {
                Stmt::Return(Some(e)) if self.stores_written(&e) => {
                    let kind = if e.ty.is_aggregate() { LocalKind::Addressed } else { LocalKind::Value };
                    self.locals.push(Local { name: "result".into(), ty: e.ty, kind });
                    let t = LocalIdx(self.locals.len() as u32 - 1);
                    let (ty, line) = (e.ty, e.line);
                    out.push(Stmt::Let(t, e));
                    out.extend(self.back.iter().cloned());
                    out.push(Stmt::Return(Some(load(t, ty, line))));
                }
                Stmt::Return(_) => {
                    out.extend(self.back.iter().cloned());
                    out.push(s);
                }
                Stmt::Break | Stmt::Continue if self.in_loop && self.depth == 0 => {
                    out.extend(self.back.iter().cloned());
                    out.push(s);
                }
                s => out.push(s),
            }
        }
        *list = out;
    }

    fn visit_stmt(&mut self, s: &mut Stmt) {
        let inner = matches!(s, Stmt::While { .. } | Stmt::ForRange { .. });
        self.depth += inner as u32;
        walk_stmt(self, s);
        self.depth -= inner as u32;
    }
}
```

In `kestrel/kir/src/lib.rs`:
- Add `pub mod promote;` after `mod ops;`.
- Change `optimize` to:

```rust
/// Inlines small functions, then keeps plain structs and the fields of `var` parameters and loop
/// elements in registers (see `scalarize` and `promote`). The driver runs it on every program.
pub fn optimize(m: &mut Module) {
    inline::inline_small(m);
    scalarize::scalarize(m);
    promote::promote(m);
}
```

- [ ] **Step 4: Run the new tests to see them pass**

Run: `cargo test -p kestrel-kir --test promote`
Expected: 6 passed.

- [ ] **Step 5: Add the program tests (guards: they pass before and after)**

Create `kestrel/tests/programs/promote_var_self.kes`:

```
// expect: 12 Ball(pos: vec2(x: 24, y: 0), vel: vec2(x: 2, y: 0.375), hits: 3)
// expect: 3 Ball(pos: vec2(x: 102, y: 994), vel: vec2(x: 0, y: -3), hits: 0)
// expect: 78 78
// expect: 14 1
// expect: 21 -1
// expect: 10 -1 1
// expect: 12
// expect: 6
// `var self` methods that return early are never inlined, so `self` stays a pointer whose fields
// are kept in registers for the whole call and written back at every way out.
struct Ball {
    pos: vec2
    vel: vec2
    hits: int

    fall(var self, ground: float, steps: int) -> int {
        var n = 0
        while n < steps {
            n += 1
            self.vel.y -= 1
            self.pos += self.vel
            if self.pos.y < ground {
                self.pos.y = ground
                self.vel.y = -self.vel.y * 0.5
                self.hits += 1
                if self.hits == 3 {
                    return n
                }
                continue
            }
            if self.pos.x > 100 {
                break
            }
        }
        self.vel.x = 0
        n
    }

    // The loop's condition reads `self`, and the method returns from inside the loop.
    climb(var self, top: float) {
        while self.pos.y < top {
            self.pos.y += 7
            if self.pos.y > 15 {
                self.hits = -1
                return
            }
        }
        self.hits = 1
    }

    next(var self) -> int {
        self.hits += 1
        self.hits
    }

    // The value returned changes `self` while it is computed (`next` is inlined into it).
    tick(var self, stop: int) -> int {
        if self.hits >= stop {
            return -1
        }
        return self.next() * 10
    }

    // Read-only: `self` and `other` are passed by pointer and only read.
    gap(self, other: Ball) -> float {
        if other.pos.x > self.pos.x {
            return other.pos.x - self.pos.x
        }
        self.pos.x - other.pos.x
    }
}

// A `var` number.
count_to(var n: int, limit: int) {
    while n < limit {
        n += 3
        if n > 10 {
            return
        }
    }
}

main() {
    var a = Ball(pos: vec2(0, 10), vel: vec2(2, 0), hits: 0)
    let steps = a.fall(0, 50)
    print(steps, a)
    var b = Ball(pos: vec2(90, 1000), vel: vec2(4, 0), hits: 0)
    let more = b.fall(0, 50)
    print(more, b)
    print(a.gap(b), b.gap(a))
    var c = Ball(pos: vec2(0, 0), vel: vec2(0, 0), hits: 0)
    c.climb(10)
    print(c.pos.y, c.hits)
    c.climb(40)
    print(c.pos.y, c.hits)
    var d = Ball(pos: vec2(0, 0), vel: vec2(0, 0), hits: 0)
    let first = d.tick(5)
    let second = d.tick(1)
    print(first, second, d.hits)
    var k = 0
    count_to(var k, 100)
    print(k)
    var m = 0
    count_to(var m, 5)
    print(m)
}
```

Create `kestrel/tests/programs/promote_loops.kes`:

```
// expect: -317
// expect: 460
// expect: 2035
// expect: 4710
// expect: {1: vec2(x: 2, y: 2), 2: vec2(x: 4, y: 5)}
// Loop items whose fields are kept in registers for one pass of the loop, and loops where they
// can't be because the item's memory is reachable another way.
struct P {
    x: float
    y: float
    vx: float
    vy: float
}

total(ps: list[P]) -> float {
    var t = 0.0
    for i, p in ps {
        t += (p.x + p.y * 2 + p.vx * 3 + p.vy * 5) * (i + 1)
    }
    t
}

main() {
    var ps = list[P]()
    for i from 0 to 6 {
        ps.add(P(x: i, y: 10, vx: 1, vy: -4))
    }
    // `continue` after a change, and a `break` out of an inner loop.
    for i, p in ps {
        p.vy -= 1
        p.y += p.vy
        if i == 1 {
            p.x = -1
            continue
        }
        for k from 0 to 10 {
            p.x += 1
            if k == 2 {
                break
            }
        }
        if p.y < 6 {
            p.y = -p.y
        }
        if i == 4 {
            break
        }
    }
    print(total(ps))
    // The body reads the list it changes: the first item's new speed is seen by the rest.
    for i, p in ps {
        p.vx = ps[0].vx + i
        if i == 0 {
            p.vx = 10
        }
    }
    print(total(ps))
    // Two loops over the same list, one inside the other.
    for a in ps {
        for b in ps {
            a.x += b.vx
        }
    }
    print(total(ps))
    // A `par for` over the items.
    par for p in ps {
        p.x = p.x * 2 + p.y
        p.vy = -p.vy
    }
    print(total(ps))
    // Map values.
    var m = {1: vec2(1, 2), 2: vec2(3, 4)}
    for k, v in m {
        v.x += k
        v = v.yx
    }
    print(m)
}
```

Create `kestrel/tests/programs/promote_panic.kes`. The panic's line number counts the four directive lines, so keep them exactly as shown:

```
// expect: 11 21
// expect: 12 32
// expect-exit: 101
// expect-stderr: index 7 out of bounds for length 3 on line 17
// A panic in the middle of a loop whose item is kept in registers: the program stops the same way.
struct P {
    x: float
    y: float
}

main() {
    var ps = [P(x: 1, y: 1), P(x: 2, y: 2), P(x: 3, y: 3)]
    let lookup = [10.0, 20.0, 30.0]
    let idx = [0, 1, 7]
    for i, p in ps {
        p.x += 10
        p.y = p.x + lookup[idx[i]]
        print(p.x, p.y)
    }
    print("not reached")
}
```

- [ ] **Step 6: Run everything**

Run (in `kestrel/`): `cargo test --workspace --no-fail-fast` and `cargo clippy --workspace --all-targets -- -D warnings`.
Expected: all pass, clippy clean.

Then `cargo build --release` in `bench/kernels`, and `py measure.py --rounds 1`. Expected: `step.kes` about 4.3 ns at one thread, the same as `step_scalar.kes` within a few percent.

- [ ] **Step 7: Commit**

```bash
git add kestrel/kir kestrel/tests/programs
git commit -m "Keep the fields of var parameters and loop elements in registers, written back at every way out" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git push
```

---

### Task 4: One runtime function per math operation

**Files:**
- Modify: `kestrel/runtime/src/math.rs` (the top of the file, up to `thread_local!`, and the `math_ops` test)
- Modify: `kestrel/codegen/src/rt.rs` (imports, `RtFuncs::math`, `MATH`, `jit_symbols`)
- Modify: `kestrel/codegen/src/func/math.rs` (doc comment, import, the match, `runtime_math`)

**Interfaces:**
- Produces: `#[no_mangle] extern "C"` functions in `kestrel_runtime::math`:
  - one `f64` each: `kestrel_rt_sin`, `kestrel_rt_cos`, `kestrel_rt_tan`, `kestrel_rt_asin`, `kestrel_rt_acos`, `kestrel_rt_atan`, `kestrel_rt_exp`, `kestrel_rt_log`;
  - two `f64`s each: `kestrel_rt_atan2(y, x)` and `kestrel_rt_pow(a, b)`.
- Removes: `kestrel_rt_math` and the `SIN`...`POW` constants.

- [ ] **Step 1: Write the failing test**

In `kestrel/runtime/src/math.rs`, replace the `math_ops` test with:

```rust
    #[test]
    fn math_ops() {
        assert_eq!(kestrel_rt_sin(0.0), 0.0);
        assert_eq!(kestrel_rt_cos(0.0), 1.0);
        assert_eq!(kestrel_rt_tan(0.0), 0.0);
        assert_eq!(kestrel_rt_asin(0.0), 0.0);
        assert_eq!(kestrel_rt_acos(1.0), 0.0);
        assert_eq!(kestrel_rt_atan(0.0), 0.0);
        assert_eq!(kestrel_rt_atan2(0.0, 1.0), 0.0);
        assert_eq!(kestrel_rt_exp(0.0), 1.0);
        assert_eq!(kestrel_rt_log(1.0), 0.0);
        assert_eq!(kestrel_rt_pow(2.0, 10.0), 1024.0);
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p kestrel-runtime math_ops`
Expected: does not compile: "cannot find function `kestrel_rt_sin`".

- [ ] **Step 3: Add the runtime functions**

In `kestrel/runtime/src/math.rs`, replace everything from the top of the file down to (not including) `thread_local!`:
- the module doc;
- the `use` lines;
- the ten `pub const` operation numbers;
- `kestrel_rt_math`.

The replacement is:

```rust
//! Math and random numbers. Compiled code does `sqrt`, `floor` and the like
//! with Cranelift instructions; the rest (trigonometry, exponentials) calls
//! one function here per operation, in `f64`.

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

#[no_mangle]
pub extern "C" fn kestrel_rt_sin(a: f64) -> f64 {
    a.sin()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_cos(a: f64) -> f64 {
    a.cos()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_tan(a: f64) -> f64 {
    a.tan()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_asin(a: f64) -> f64 {
    a.asin()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_acos(a: f64) -> f64 {
    a.acos()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_atan(a: f64) -> f64 {
    a.atan()
}

/// The angle of the point (`x`, `y`).
#[no_mangle]
pub extern "C" fn kestrel_rt_atan2(y: f64, x: f64) -> f64 {
    y.atan2(x)
}

#[no_mangle]
pub extern "C" fn kestrel_rt_exp(a: f64) -> f64 {
    a.exp()
}

/// The natural logarithm.
#[no_mangle]
pub extern "C" fn kestrel_rt_log(a: f64) -> f64 {
    a.ln()
}

#[no_mangle]
pub extern "C" fn kestrel_rt_pow(a: f64, b: f64) -> f64 {
    a.powf(b)
}

```

- [ ] **Step 4: Call them from codegen**

In `kestrel/codegen/src/rt.rs`:
- Add `use kestrel_kir::MathOp;` after the `cranelift_module` import.
- Replace the field `pub math: FuncId,` with:

```rust
    /// The function for each float operation the runtime does (see `MATH`).
    pub math: Vec<(MathOp, FuncId)>,
```

- Replace the `math: decl("kestrel_rt_math", ...)?,` line in `declare` with:

```rust
            math: MATH.iter().map(|&(op, name, args)| Ok((op, decl(name, &[F64, F64][..args], Some(F64))?))).collect::<Result<_, String>>()?,
```

- Add before `jit_symbols`:

```rust
/// The float operations the runtime does: each one's function and how many `f64`s it takes.
pub(crate) const MATH: [(MathOp, &str, usize); 10] = [
    (MathOp::Sin, "kestrel_rt_sin", 1),
    (MathOp::Cos, "kestrel_rt_cos", 1),
    (MathOp::Tan, "kestrel_rt_tan", 1),
    (MathOp::Asin, "kestrel_rt_asin", 1),
    (MathOp::Acos, "kestrel_rt_acos", 1),
    (MathOp::Atan, "kestrel_rt_atan", 1),
    (MathOp::Atan2, "kestrel_rt_atan2", 2),
    (MathOp::Exp, "kestrel_rt_exp", 1),
    (MathOp::Log, "kestrel_rt_log", 1),
    (MathOp::Pow, "kestrel_rt_pow", 2),
];
```

- In `jit_symbols`, replace `("kestrel_rt_math", math::kestrel_rt_math as *const u8),` with:

```rust
        ("kestrel_rt_sin", math::kestrel_rt_sin as *const u8),
        ("kestrel_rt_cos", math::kestrel_rt_cos as *const u8),
        ("kestrel_rt_tan", math::kestrel_rt_tan as *const u8),
        ("kestrel_rt_asin", math::kestrel_rt_asin as *const u8),
        ("kestrel_rt_acos", math::kestrel_rt_acos as *const u8),
        ("kestrel_rt_atan", math::kestrel_rt_atan as *const u8),
        ("kestrel_rt_atan2", math::kestrel_rt_atan2 as *const u8),
        ("kestrel_rt_exp", math::kestrel_rt_exp as *const u8),
        ("kestrel_rt_log", math::kestrel_rt_log as *const u8),
        ("kestrel_rt_pow", math::kestrel_rt_pow as *const u8),
```

In `kestrel/codegen/src/func/math.rs`:
- The module doc's second and third sentences become: "`round` are Cranelift instructions; the rest call the runtime's function for that operation (`kestrel_rt_pow`, ...), which works in `f64`. The random numbers call the runtime's generator."
- Remove `use kestrel_runtime::math as rt_op;`.
- Replace the ten `MathOp::Sin => self.runtime_math(rt_op::SIN, &vals, ty),` ... `MathOp::Pow => ...` arms with:

```rust
            MathOp::Sin
            | MathOp::Cos
            | MathOp::Tan
            | MathOp::Asin
            | MathOp::Acos
            | MathOp::Atan
            | MathOp::Atan2
            | MathOp::Exp
            | MathOp::Log
            | MathOp::Pow => self.runtime_math(op, &vals, ty),
```

- Replace the head of `runtime_math`: its doc comment, its signature, and everything up to and including the line that calls `self.shared.rt.math`. The `if ty == KTy::F32 { fdemote } else { r }` tail stays. The new head is:

```rust
    /// Calls the runtime's function for `op`; `f32` operands are widened to `f64` and the result narrowed back.
    fn runtime_math(&mut self, op: MathOp, vals: &[Value], ty: KTy) -> Value {
        let args: Vec<Value> = vals.iter().map(|&v| if ty == KTy::F32 { self.b.ins().fpromote(types::F64, v) } else { v }).collect();
        let (_, id) = *self.shared.rt.math.iter().find(|(o, _)| *o == op).expect("a runtime function for every float operation");
        let r = self.call_id(id, &args).expect("a runtime math function returns a value");
```

Check that nothing else names the old function: `grep -rn "kestrel_rt_math\|rt_op::" --include=*.rs` in `kestrel/` (skip `target/`) finds nothing.

- [ ] **Step 5: Run everything**

Run (in `kestrel/`): `cargo test --workspace --no-fail-fast` and `cargo clippy --workspace --all-targets -- -D warnings`.
Expected: all pass. The existing math program and codegen tests show the results are unchanged, and the exe-built program tests show the staticlib exports the new symbols.

Then `cargo build --release` in `bench/kernels` and `py measure.py --rounds 1`. Expected: `step_pow.kes` a little faster than after Task 3 (the prototype went from 16.75 to 16.39 ns).

- [ ] **Step 6: Commit**

```bash
git add kestrel/runtime kestrel/codegen
git commit -m "Call one runtime function per math operation instead of a dispatcher" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git push
```

---

### Task 5: Measure, check the Godot add-on, and update the docs

**Files:**
- Modify: `kestrel/bench/kernels/results.md`
- Modify: `docs/superpowers/specs/2026-10-11-scalar-replacement-design.md` (an "As built" section)
- Modify: `docs/superpowers/plans/2026-10-09-core-2a-followups.md`
- Modify: `docs/superpowers/plans/2026-10-10-handoff.md`
- Modify: `README.md` (lines 119-121, the speed paragraph)

- [ ] **Step 1: Measure after**

All on a quiet machine; repeat a run that looks off.
- In `kestrel/bench/kernels`: `cargo build --release`, then `py measure.py --rounds 3 --threads 1` and `py measure.py --rounds 3 --threads 16`.
- In `kestrel/`: `cargo build --release -p kestrel-cli`. Then run `run.ps1` in `bench/2a`, `bench/heightmap`, `bench/voxels3d` and `bench/maps`, and compare each with the baseline saved in Task 1.
- **The particles demo's update at 200,000 particles,** in a hidden window, from `kestrel/`:
  1. Copy `demos/particles/particles.kes` to a scratch folder outside the repo, with `START_COUNT = 200000`.
  2. Run it with `KESTREL_HIDDEN=1`, as `target/release/kestrel.exe run --release <copy>`, for about 10 seconds. Stop it from a script with a timeout; it prints one line a second.
  3. Read the `update` times, then do the same with the old compiler's binary saved before Task 2, or at the commit before Task 2 in a scratch worktree.

  Hidden windows are sometimes capped at 60 fps on this machine. The update time is still valid; the frame time is not.
- **Compile time:** `kestrel check` and `kestrel build` of `demos/voxels/voxels.kes`, a median of 7 each, against the old binary.

Expected: `step.kes` within 1.2x of Rust step and within 5% of `step_scalar.kes`; `step_pow.kes` within 1.15x of Rust pow. Everything else is no slower beyond noise. If something is slower, find out why before going on: dump the hot function with `bench/kernels`'s `--dump`, or the program's loop.

- [ ] **Step 2: Check the Godot add-on**

1. Close Godot if it is open (Windows locks the add-on's DLL).
2. Run `kestrel/godot/build.ps1`.
3. Run `cargo clippy --workspace --all-targets -- -D warnings` in `kestrel/godot`.
4. From `kestrel/godot`, run `<Godot>_console.exe --headless --path demo --script res://tests/test_bridge.gd`. The Godot path is in the handoff. Run `--headless --path demo --import` once first if the project hasn't been imported.

Expected: 30 checks pass. The exit code is the number of failed checks.

- [ ] **Step 3: Write the results**

- `bench/kernels/results.md`: add an "After scalar replacement" table next to "Before" (one thread and 16 threads), and the before/after for `bench/2a`, `heightmap`, `voxels3d`, `maps`, the demo's update and compile time. Name what got faster, what didn't move, and any slowdown. Say plainly that the copy loop is still about 4.5x Rust, and why: list-header reloads, a bounds check, and two 4-byte moves instead of one 8-byte move.
- The spec: add an "As built (2026-10-11)" section with the measured numbers and anything that differs from the design. One known difference: `promote`'s generated loads and write-backs carry line 0.
- The follow-ups file: add a "Found while building scalar replacement (2026-10-11)" section. Move the spec's "Not in this design" items there:
  - vectorizing;
  - bounds checks and list-header reloads, with the alias-region trap;
  - promoting an inlined `var self` on a checked indexed place;
  - range analysis.

  Add anything new this work found. Update the "Performance" section's sub-project 5 lines.
- The handoff:
  - add this work under "Done", with the test count and the headline numbers;
  - update "Status";
  - update "Next": the remaining candidates from the follow-ups, for the user to choose.
- `README.md` lines 119-121: replace the particle-loop sentence with the new figures (the step about 1.15x Rust instead of 3.4x; the copy loop still behind). Keep the paragraph's form.

- [ ] **Step 4: Commit**

```bash
git add kestrel/bench/kernels/results.md docs README.md
git commit -m "Record the scalar replacement results and point the handoff at what's next" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git push
```
