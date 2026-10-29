# Proposal: Kestrel on the GPU: `par gpu for` and a shader package (2026-10-27)

Status: a proposal for the user. Not a spec; nothing is built, compiled or measured. Every new name below is the user's call, and each one has a recommendation. Numbers marked "expected" are estimates from the measurements we already have, not new measurements.

## The pitch

One language for CPU code, GPU compute and rendering shaders. The same `pure` function (the voxel engine's `noise.kes`, a particle's `step`, a colour helper) can run in a `par for` on the CPU cores, in a `par gpu for` on the graphics card, or inside a material's fragment shader. One compiler checks all three with the same rules and the same plain-sentence errors.

Kestrel is unusually close to this already. A `par for` body may only call `pure` functions and change its own item and its own locals. That is exactly the shape a GPU wants: one small program run once per item, with no item touching another's data. `int` and `float` are already 32 bits, like a GPU's. `vec2`/`vec3`/`vec4` are shader types under other names. Godot keeps GDScript and its shading language separate, so you can't call a GDScript function from a shader. Kestrel doesn't have to keep them apart.

## What exists today (read, not run)

- **raylib 6.0**, vendored in `kestrel/window/raylib`, built by `kestrel/window/build.rs` with `GRAPHICS_API_OPENGL_33`. So the window asks for an OpenGL 3.3 core context.
- **rlgl's compute and buffer functions are compiled out** in that build. `rlLoadShaderProgramCompute`, `rlComputeShaderDispatch`, `rlLoadShaderBuffer`, `rlReadShaderBuffer` and the rest sit under `#if defined(GRAPHICS_API_OPENGL_43)`. Without that flag they log "Compute shaders not supported, enable GRAPHICS_API_OPENGL_43" and do nothing.
- **No custom shaders are bound.** `window/src/raylib.rs` declares the `Shader` and `Material` structs (because `DrawMesh` takes a `Material`), but not `LoadShaderFromMemory`, `GetShaderLocation` or `SetShaderValue`. `window/src/material.rs` keeps one default material whose colour is set before each draw. `draw3d.kes` has meshes, textures, a camera and no lighting.
- **`par for` is already a separate function.** `kir/src/lower/par.rs` moves the body into a function `__kestrel_par_N(captures by pointer..., start, end)`, registered in `Module::par_bodies`. `codegen/src/par_thunk.rs` wraps it in a thunk that the runtime's thread pool calls. A GPU kernel is the same function with `start..end` replaced by "my invocation number".
- **GPU objects already have an owner.** `window/src/gpu.rs` keeps meshes and textures in a registry on the window's thread, behind a `GpuHandle` (`KTy::Gpu`). Freeing from another thread only queues the id. GPU buffers would join this registry.

## 1. What the user gets

### Scene A: a million particles, stepped and drawn on the GPU

`demos/particles/results.md` measured today's demo at 1,000,000 particles: **update 3.3 ms** (the `par for` on 16 threads) and **draw 38 ms**. Of the draw, filling the vertices takes 20.8 ms and uploading them takes 11.3 ms. That's 41 ms a frame, or 24 fps. The step isn't the problem. Copying a million particles to the GPU every frame is.

```
use { Particle, spawn from particles }   // the demo's struct and its `pure step`, unchanged

main() {
    window.open(1280, 720, "a million dots")
    var start = list[Particle]()
    spawn(var start, 1000000)            // on the CPU: it uses random numbers
    var parts = gpu_list(start)          // copied to the GPU once
    while window.frame() {
        let dt = window.delta()
        let target = input.mouse()
        let keep = pow(0.5, dt)
        par gpu for p in parts {         // the same body the CPU version runs
            p.step(dt, target, keep)
        }
        draw.clear(BLACK)
        draw.points(parts, size: 2, color: WHITE)   // drawn from the GPU list: nothing comes back
    }
}
```

Expected: the step is well under 1 ms of GPU time, and drawing 1M points from a buffer that is already on the GPU takes about 1 to 2 ms. So the frame drops from 41 ms to a few ms. **Most of that win comes from not copying, not from faster maths** (see section 8 for the honest comparison).

### Scene B: voxel terrain from the GPU

Generation is about 99% noise: three octaves, eight lattice hashes each, for every voxel. `bench/engine/results.md` gives **300 ms for 64 columns** through `load_now` on one core. With `par for` on all cores the frozen world runs about 8 times faster, so roughly 40 ms. On the GPU, one item would be one row of 32 voxels along x. The item is a `u32` with one bit per voxel, so the readback is 1 MB instead of 8 MB:

```
// 64 columns x 128 heights x 32 rows: 262,144 rows of 32 voxels each.
// `cols` is a captured list of the 64 columns' coordinates (captured lists come in round 2).
var rows = gpu_list[u32](64 * HEIGHT * SIZE)
par gpu for i, bits in rows {
    let col = i / (HEIGHT * SIZE)
    let y = (i / SIZE) % HEIGHT
    let z = i % SIZE
    var b: u32 = 0
    for x from 0 to SIZE {
        if solid_at(cols[col].x * SIZE + x, y, cols[col].z * SIZE + z, seed) {   // gen.kes, unchanged
            b = b | (u32(1) << x)
        }
    }
    bits = b
}
let solid = rows.read()          // waits for the GPU; 1 MB
// then the cheap typing pass (grass, dirt, stone) on the CPU, as today
```

Expected: a few ms of GPU time, plus about 1 ms of readback, against about 40 ms on all cores. That is roughly 5 to 15 times faster, **to be measured**. The catch: `noise.kes`'s `hash` multiplies `i64`s, and plain GLSL 4.30 has no 64-bit integers. So this scene needs `i64` support on the GPU (round 2), and its results may differ from the CPU's in the last bit (section 4).

### Scene C: a water material written as a Kestrel function

(Revised 2026-10-28 after a review of how shaders are written: section 5, "Writing shaders: the review's changes". The first version returned a `vec4`, multiplied the alpha by the light by mistake, used the terrain's integer noise, which made the water blocky and needs `i64` on the GPU, and kept its colours where hot reload couldn't see them.)

```
use { Fragment, Surface, value3 from shader }

// Art values: plain constants the shader reads, so hot reload picks up a changed colour.
let DEEP = Color(r: 0.05, g: 0.2, b: 0.4)
let SHALLOW = Color(r: 0.2, g: 0.6, b: 0.7)

// Uniforms: only what the game changes while it runs.
struct Water {
    scale: float = 0.125
    speed: float = 0.5
}

@prove(gpu)
pure water(px: Fragment, w: Water) -> Surface {
    let p = vec3(px.world.x * w.scale, px.time * w.speed, px.world.z * w.scale)
    Surface(albedo: lerp(DEEP, SHALLOW, value3(p)), alpha: 0.8, roughness: 0.1)
}

main() {
    ...
    draw3d.sun(dir: vec3(-0.3, -1, -0.2), color: WHITE)
    let mat = draw3d.material(fragment: water, params: Water(), blend: Blend.Alpha)
    while window.frame() {
        draw3d.mesh_with(lake, mat, vec3(0, 0, 0))   // lake: a Mesh made earlier with make_mesh
    }
}
```

In Godot terms, `water` is the `fragment()` function of a `shader_type spatial` shader. `Fragment` holds the built-ins (`VERTEX` in world space, `NORMAL`, `UV`, `COLOR`, `TIME`), the `Water` struct is the list of `uniform`s, and the returned `Surface` is the outputs (`ALBEDO`, `ALPHA`, `ROUGHNESS`, ...), which the engine then lights, as Godot does. The difference from Godot: `water` is an ordinary `pure` function. A unit test can call it on the CPU in one line, `water(Fragment(world: vec3(1, 0, 2)), Water())`, and it can call the same helpers CPU code uses.

### Scene D: hot-reloading the water

With `kestrel run`, the compiler lives in the same process as the program (JIT). Save the file, and a watcher thread compiles the changed file again, only as far as needed to regenerate the shaders, which takes tens of ms. It hands the new GLSL to the window thread, and the next frame draws with it. If the edit has an error, the old shader stays and the error is printed. Changing `DEEP` or `SHALLOW` reloads (constants the shader reads are compiled into its GLSL); changing a default in `Water` doesn't, because defaults are applied where `Water()` is built, in CPU code. If the edit changes the `Water` struct's fields, the reload is refused ("`Water` changed its fields; restart to use the new shader"), because the running program's values have the old layout. CPU code isn't reloaded: that is the separate "standalone hot reload" item in the handoff. If `noise.kes` changes, shaders get the new version and CPU code keeps the old one until a restart, and the reload says so.

## 2. The GPU subset

**What can run on the GPU:** `pure` code over plain data.

- **Types:** `int`, `u32`, `float`, `bool` (in locals; `bool` fields in buffers come with `u8` in round 2), `vec2/3/4`, `ivec2/3/4`, `Color`, structs made of those, and enums without fields. Round 2 adds `u8`, `u16`, `i64` (emulated, see below) and enums with fields.
- **Statements:** `let`, `var`, `if`, `match`, `for ... from ... to`, `while`, `break`, `continue`, `return`, and calls to other `pure` functions (any depth, no recursion).
- **Reading captured values:** numbers, vectors and structs from outside the loop become uniforms. A captured `list[T]` or `[T]` of plain data becomes a read-only GPU buffer, uploaded for that dispatch (round 2). A captured `gpu_list` is bound without any copy.
- **Writing:** only the loop's own item and locals, as in `par for` today.

**What can't:** strings and interpolation, `print`, lists that grow (`add`, `insert`, `remove_at`, a list literal in the body), maps, `arena { }`, closures and function values, interface values, natives that aren't on a short GPU list (the math natives are fine), recursion, `T?` (round 1), `f64` (GPUs run it 16 to 64 times slower on gaming cards; refuse it and say so), and `par for` / `par gpu for` inside the body.

**Bounded loops?** No requirement. Requiring provable bounds would forbid ordinary `while` loops like `generate`'s. But a GPU loop that never ends is worse than one on the CPU. On Windows the driver resets the GPU after about 2 seconds (TDR): the screen flickers, and any game running at the same time may crash. So **in a checked build each `while` loop on the GPU counts its turns** and stops with an error after 16,777,216. `--release` leaves the counter out, in keeping with "release turns off the arithmetic checks".

**How the compiler checks it.** At two levels, like `@prove` today:

1. **In `check`:** whatever the source shows directly is checked there, such as a string, a closure, a call to a non-`pure` function or a `par for` inside the loop. These errors point at the line.
2. **After inlining, in the contract walk** (`kir/src/range/contracts.rs`, `range/events.rs`): what only shows up after lowering, such as a list growing three calls down, an allocation or recursion. This reuses the chain-of-calls errors `@prove` already prints. `par gpu for` simply carries an implicit `@prove(gpu)`.

Example errors (plain sentences that say what to do):

- "line 14 can't run on the GPU: `p.name` is a string. Keep strings out of a `par gpu for`, or use `par for`."
- "the `par gpu for` on line 30 can't run on the GPU: it calls `spawn` (line 12), which adds to `parts` (line 18), and a GPU can't grow a list."
- "the `par gpu for` on line 30 can't run on the GPU: `Enemy.think` (line 55) calls itself, and GPUs have no recursion. Write it as a loop."
- "`f64` can't be used on the GPU: gaming GPUs run it many times slower. Use `float`."
- "`water` can't be a fragment shader: it takes `(Fragment, int)`, and a fragment shader takes `(Fragment, P)` where `P` is a struct of numbers, vectors, colours and textures."
- "`water` samples a texture (line 9), which only a shader can do; call it from a material, not from CPU code."

**`@prove(gpu)`** (recommended) goes on a function: "this can run on the GPU". It fails the build with the same chain when someone adds a `print` three calls down. It is a promise the compiler proves, so it fits the rule that `@` is for promises and facts. It does **not** make the function GPU-only. `noise.kes` would put `@prove(gpu)` on `octave`, so a later edit can't silently break the terrain kernel and the water shader.

**GPU-only built-ins** (`shader.sample` for textures, screen-space derivatives) make a function GPU-only. Calling such a function from CPU code is an error (the last example above). The compiler tracks this the same way it tracks `pure`.

## 3. The backend

| Option | For | Against | Verdict |
|---|---|---|---|
| **GLSL text** (4.30 for compute, 3.30 for vertex/fragment) | raylib is OpenGL; `LoadShaderFromMemory` and GL compute both take GLSL text; no new crate; the output is readable (`kestrel check --gpu-dump` prints it); the driver compiles and caches it | Drivers' GLSL compilers differ (NVIDIA forgiving, AMD and Intel strict), so generated code must stay boring; each kernel is compiled at its first run (tens of ms, cached by the driver) | **Recommended** |
| SPIR-V (via `rspirv`) | Precompiled, exact, no driver parser | In OpenGL it needs 4.6 or `GL_ARB_gl_spirv` (missing or buggy on some drivers); its real home is Vulkan, which means leaving raylib for drawing | Later, with Vulkan if ever |
| `naga` (wgpu's shader translator, a Rust crate) | One IR, then GLSL, SPIR-V, HLSL, Metal and WGSL out; validates our output; WGSL means the web later | A large dependency whose IR changes between releases; we still do layout, `i64` and `u8` lowering ourselves; for one GL target it adds more than it saves today | The plan for the web or Vulkan: keep our emitter behind a small interface so naga can replace it |

**Where it sits:** a new crate, `kestrel/gpu/`, between KIR and codegen. Each GPU body is outlined exactly as `par for` bodies are today (`Stmt::ParGpuFor`), then goes through **inlining and range analysis like any function**. Then two things happen to it:

1. Cranelift compiles it as a normal `par for` body plus thunk. **This is the CPU fallback, and it is literally today's `par for` code.**
2. `kestrel/gpu` turns the same KIR function into GLSL, which is embedded in the program as a string. Called functions become GLSL functions (or are already inlined). `vec2/vec3`, which are structs in KIR, become GLSL vectors in registers.

At run time the runtime picks the GPU or the CPU at the first dispatch.

**Memory layout must match byte for byte**, so that uploads are plain copies. GLSL's `std430` rules differ from Kestrel's in places. A GLSL `vec3` is aligned to 16 bytes and a Kestrel `vec3` to 4, and `vec2` is 8 against 4. A GLSL `bool` in a buffer is 4 bytes, and core GLSL has no 8-bit loads. So the emitter declares buffer elements as plain 4-byte scalars (plus `vec4`, which is 16-aligned in both) and rebuilds the vectors on load. `u8` and `bool` fields are read with shifts from 32-bit words (round 2). Writing one byte is harder: two invocations writing neighbouring bytes of the same word would race. So round 2 either needs whole-word ownership (an item's bytes don't share a word with another item's) or an atomic OR. Scene B's bit rows avoid the problem.

**Kestrel's meaning wins over GLSL's.** Every operation must mean in the GLSL what it means on the CPU. `normalized()` of a zero vector stays zero (GLSL's `normalize` gives NaN). `int(x)` truncates, saturates, and gives 0 for NaN (GLSL leaves an out-of-range conversion undefined). `>>` on `int` is arithmetic. The emitter writes small helper functions for these. They are where most of the subtle bugs will live, so each needs a test that runs it on both sides.

**Checks on the GPU.** A GPU can't stop a program in the middle of a dispatch. Options:

- **(a) Proven checks only:** refuse any `par gpu for` with a check left in it. Clean, but most real code would be refused (`contract_gen.kes` still fails `no_checks`).
- **(b) Clamp or wrap silently:** fast, but it breaks Kestrel's promise that a checked build stops on overflow, and that even `--release` never reads outside a list.
- **(c) An error flag (recommended).** Each dispatch gets a small error buffer. A failing check writes the kind of error, its line and the item's index (`atomicMin` on the index, so the lowest failing item is reported, as the determinism proposal wants for `par for`), then ends that invocation. After the dispatch the runtime reads the flag at the next point where it waits for the GPU anyway, and stops the program with the usual message: "integer overflow on line 12 of particles.kes (in a `par gpu for`, item 4,113)". Range analysis removes most checks exactly as on the CPU. `--release` drops the arithmetic checks (GPU integers already wrap) and **keeps bounds and division checks**, as on the CPU. Overflow checks use `uaddCarry`/`imulExtended`. They will cost more on the GPU than on the CPU; measure before tuning.

One honest difference remains: the error is noticed after the dispatch, so other items have already run, and with a resident list the program may have drawn one frame with bad data first. The program still stops with the right line and item.

## 4. `par gpu for` semantics

**Two kinds of loop, one keyword:**

- **Over a `list[T]` (copy mode):** upload the list, dispatch, wait, read it back into the list. Simple, synchronous and correct. It costs two copies over PCIe per loop: for 16 MB (1M particles) that is roughly 3 to 8 ms with the wait. Good for big work on small data (Scene B), bad for cheap work on big data every frame.
- **Over a `gpu_list[T]` (resident mode):** no copies. The dispatch is queued and the loop returns at once. Later GPU work (the next dispatch, `draw.points(parts)`) sees its results, because the runtime puts a memory barrier between them. **The CPU waits only when it reads the data:** `parts.read()` (to a new list), `parts.copy_to(var xs)`, `parts[i]` (one item; slow, so the report flags it in a loop), or an error flag being checked. Writing works the same way: `parts.write(xs)`, `parts[i] = p`. This is a GPU-side list, like Godot's `RenderingDevice` storage buffers, but owned like a Kestrel value and freed through the existing `gpu.rs` queue.

**Sync points**, said plainly: a read, a `len` change, an error check, and the window closing. Reading right after a dispatch waits for everything queued before it, which can include the previous frame's drawing. The report should list reads inside loops.

**Without compute support** (no OpenGL 4.3, no window open, or a headless test), **the same loop runs as a CPU `par for`**, with the same code and results within float rounding. A `gpu_list` then lives in normal memory. `gpu.available()` tells a program which path it got, and `KESTREL_GPU=off` in the environment forces the CPU (for tests and comparisons; the runtime reads it, the program doesn't need to). The fallback is chosen once per run.

**Getting OpenGL 4.3 without breaking 3.3 machines.** Building raylib with `GRAPHICS_API_OPENGL_43` would make the window *require* 4.3. Then a machine with only 3.3 (old integrated graphics, some virtual machines, Remote Desktop) couldn't open a window at all. Recommended instead: keep the 3.3 build. When the window opens, ask the driver what it actually gave. NVIDIA, AMD and Intel drivers on Windows usually hand back their newest core version when asked for 3.3, and this must be checked on the user's machine with a hidden window. If it is 4.3 or newer, load the dozen compute and buffer GL functions ourselves through `glfwGetProcAddress`, inside `kestrel/window`. Mixing our own GL calls with raylib's means flushing raylib's draw batch first (`rlDrawRenderBatchActive`, already bound) and restoring the state we change.

**Threads:** raylib and GL work only on the window's thread. `par gpu for` is an effect (it isn't `pure`), so it can't appear in a `pure` function or a `par for` body. That gives an existing error, and it is the right rule.

**Determinism (the caveat).** GPUs don't promise the CPU's floats. GLSL makes `+ - *` correctly rounded, but division may be off by 2.5 units in the last place, `sqrt`, `sin`, `pow` and `exp` are each vendor's own, small numbers may be flushed to zero, and a driver may fuse `a * b + c` unless the variable is marked `precise`. So a GPU result can differ from the CPU fallback and from another vendor's GPU in the last bit, and terrain can flip a block where the density is about 0. Therefore:

- **`par gpu for` is incompatible with `@prove(deterministic)`** (the 2026-10-27 determinism proposal): "`update` can't be proven `deterministic`: it runs a `par gpu for` (line 20), and GPUs round some float operations differently by vendor; use `par for`." Even the choice between GPU and CPU fallback depends on the machine, so the result does.
- **Mitigations the emitter should do anyway:** mark everything `precise` (Kestrel has no fast-math, and that rule shouldn't change on the GPU), and divide with a correctly rounded routine (a few extra instructions). That makes integer code and `+ - * /` match the CPU bit for bit. `sqrt` and the transcendentals still won't. A later, narrow `@prove(deterministic)` for GPU code limited to those operations is possible. Not proposed now.
- For the voxel engine this means a world generated on the GPU should be generated **entirely** on the GPU (or entirely on the CPU) within one save, and a multiplayer world shouldn't use GPU generation at all.

## 5. The shader package

**Stage by use, not by annotation (recommended).** Any `pure` function with the right shape can be a shader. The stage is decided where it is used:

- A **fragment shader** is `pure f(f: shader.Fragment, p: P) -> vec4`.
- A **vertex shader** is `pure v(v: shader.Vertex, p: P) -> shader.Vertex`. It returns the moved vertex in model space, and Kestrel applies the camera afterwards, like Godot's `vertex()` editing `VERTEX`.
- `P` is any struct of numbers, vectors, `Color`s and `Texture`s. Its fields become uniforms (textures become samplers).

`draw3d.material(fragment: water, vertex: wave, params: Water(...))` is a built-in that the compiler sees, like `len`. It requires named functions (not closures or function values), compiles them to GLSL 3.30 (vertex and fragment shaders don't need 4.3, so materials work wherever the window does), and returns a `draw3d.Material[Water]`. `mat.params` is an ordinary field. Changing it costs nothing until the next draw, which uploads the uniforms. `draw3d.mesh_with(mesh, mat, pos)` draws with it.

**`shader.Fragment`** holds `world: vec3`, `normal: vec3`, `uv: vec2`, `color: Color`, `screen: vec2` and `time: float`. **`shader.Vertex`** holds `pos`, `normal`, `uv`, `color`, `time` and `index`. Custom values passed from the vertex shader to the fragment shader (Godot's `varying`) wait for round 4. The likely shape is a vertex shader that returns a user struct which contains a `shader.Vertex`, and a fragment shader that takes that struct.

**A small `shader` std module**, written in Kestrel and `@prove(gpu)`, so it also runs on the CPU (except `sample`):

- **noise:** `value2`, `value3`, `simplex2`, `fbm`
- **lighting:** `lambert`, `half_lambert`, `blinn`, `fresnel`, `rim`
- **colour:** `mix`, `srgb_to_linear`, `linear_to_srgb`, `hsv`, `luminance`
- **shaping:** `smoothstep`, `step`, `remap`, `saturate`
- **GPU only:** `sample(tex, uv)`, and later `dfdx`/`dfdy`

**Lighting today** is "none: colours are drawn as given". The shader package is how lighting arrives, as a library, not as engine magic.

**Hot reload** works as in Scene D. It applies only to `kestrel run` (the JIT), because a built exe has no compiler inside it.

### Writing shaders: the review's changes (2026-10-28)

The user asked for a review of how shader functions are written before anything is built. An Opus reviewer and the controller each read this section independently and found the same core problems; these changes replace the rules above where they differ. Every new name is the user's call.

1. **The return type says who does the lighting.** `-> shader.Surface` is lit by the engine: a struct of `albedo: Color`, `alpha = 1`, `emission = BLACK`, `normal`, `roughness = 1`, `metallic = 0`, every field with a default, so fields added later (or a feature the engine adds later) never break an old shader. `-> Color` is unlit, the final colour (Godot's `unshaded`, Shadertoy's `fragColor`); a shader that wants its own lighting returns `Color` and calls `lambert` itself. Engine lighting starts small: one sun plus ambient light, set with `draw3d.sun(dir:, color:)`. A material takes `blend: Blend.Opaque | Blend.Alpha | Blend.Add` (default opaque), and transparent materials are drawn after opaque ones. *Rejected:* returning `vec4` (no room for emission or normal maps without a new signature, and its example multiplied the alpha by the light); Godot-style writes to `ALBEDO` and friends (a `pure` function has no output variables); one function per output as in three.js TSL (spreads one material over several functions).
2. **No second set of maths names.** Instead of `shader.mix`, `step` and `saturate`, the built-ins grow: `lerp` also takes two `Color`s, `smoothstep` becomes a built-in for floats and vectors, and `vec4(c)` / `Color(v)` convert. Kestrel has no overloading and `noise.kes` already exports a `mix`, so a `shader.mix` would clash in any file that imports both; `saturate` is `clamp(x, 0, 1)`. *Rejected:* operator overloading on `Color` (a language change much bigger than shaders); making `Color` an alias of `vec4` (its 16-byte alignment changes every struct that holds a colour).
3. **Shader examples use float noise** (`shader.value3(p: vec3)`, `fbm`): smooth in space and time. `noise.kes`'s integer noise stays the terrain's (bit-exact, `i64` hashes, which a GPU emulates slowly). Sharing helpers between CPU and GPU still works; the example no longer pretends the terrain's noise suits water.
4. **Art values are constants, uniforms are params.** Top-level `let` constants a shader reads are compiled into its GLSL, so hot reload picks up a changed colour. The params struct holds only what the game changes at run time. *Rejected:* re-applying struct defaults on reload (it would change hidden CPU state behind the program's back).
5. **One clock:** `Fragment.time`, which the engine wraps every hour (as Godot's `TIME` does) so a `float` keeps its precision; no time field in params.
6. **The inputs are defined:** `Fragment.world` is the world position, `Fragment.normal` is normalized and in world space, `Color` values are sRGB and the engine converts them to linear. Every `Fragment` field has a default, so a CPU test is one line.
7. **Checks inside materials.** With OpenGL 4.3 present, the error flag of section 3 works in every stage. On 3.3 (no storage buffers there), a failed check in a checked build paints the pixel magenta, and `--report` lists the checks left in each material. `--release` has none, as everywhere.
8. **Errors land where you'd look.** A signature error names both lines: "`water` (line 15) can't be a fragment shader: it takes `(Fragment, int)` ...; it is used as one on line 22". Examples put `@prove(gpu)` on shader functions so GPU errors appear at the definition. A shader with no params is allowed: `pure glow(px: Fragment) -> Color`. Stage-by-signature stays. *Rejected:* `@fragment` / `@vertex` annotations (an `@` is a promise the compiler proves, not a role), a `shader water { }` declaration (a second kind of function).
9. **The future is additive.** Custom values from the vertex shader to the fragment shader (Godot's `varying`) become an optional middle parameter: `pure water(px: Fragment, v: Ripple, w: Water)`, where the vertex shader returns `Ripple`, a struct with a `vertex: shader.Vertex` field. 2D shaders get their own input type, `shader.Canvas`, instead of a `Fragment` whose `world` and `normal` mean nothing. The two-parameter form never changes.
10. **`sample` runs on the CPU too**, reading a cached copy of the texture (slow, for tests), so a textured shader keeps its CPU unit test. Only screen-space derivatives (`dfdx`, `dfdy`) stay GPU-only.
11. **Parameter names in examples:** `px` for the `Fragment`, not `f`. Section 5's `pure f(f: shader.Fragment, ...)` was itself an error: Kestrel allows no shadowing, so a parameter can't share its function's name.

**Still open, noted by the review:** `draw3d.mesh_with(mesh, mat, pos)` reads awkwardly; `draw3d.mesh(m, pos, material: mat)` would be nicer but needs optional parameters, which Kestrel doesn't have. The user picks the name.

## 6. New syntax and names (all the user's call)

| Name | What it is | Recommendation | Rejected |
|---|---|---|---|
| `par gpu for` | the GPU loop; `gpu` is a word only after `par`, not a reserved keyword | **this** | `gpu for` (loses the link to `par for`'s rules); `@gpu par for` (`@` is for promises, not for where code runs); `par for ... on gpu` |
| `gpu_list[T]` | a list that lives on the GPU; `gpu_list(xs)`, `gpu_list[T](n)`, `.read()`, `.write(xs)`, `.copy_to(var xs)`, `.len`, `[i]` | **`gpu_list[T]`**, built in like `list` | `gpu[T]`; `gpu.Buffer[T]` (natives can't be generic, so it would be built in anyway); `list[T] on gpu` |
| `@prove(gpu)` | a sixth contract: "this can run on the GPU" | **yes** | a `gpu` keyword on functions (like `pure`): a second colouring of functions that `@prove` already covers |
| `gpu` module | `gpu.available() -> bool`, `gpu.name() -> string` | yes, tiny | |
| `shader` module | `Fragment`, `Vertex`, `sample`, plus the helpers in section 5 | yes | putting it in `draw3d` (2D shaders want it too) |
| `draw3d.material(fragment:, vertex:, params:)`, `draw3d.Material[P]`, `draw3d.mesh_with` | materials | yes | `@fragment`/`@vertex` stage annotations (not promises); a `shader water { }` declaration (a second kind of function) |
| `draw.points(list, size:, color:)` | draws a `gpu_list` of items with a `pos: vec2` field, from the GPU | **the field-name rule for round 1** | a closure picking the field (`at: (p) -> p.pos`), nicer but a closure compiled to a vertex shader is more work |
| `shader.Surface`, `-> Color` for unlit | a fragment shader's output: lit by the engine, or the final colour (section 5, "the review's changes") | **yes** | returning `vec4`; writing Godot-style output variables |
| `draw3d.sun(dir:, color:)` | the engine's one light, plus ambient | yes | lighting only in user code |
| `blend: Blend.Opaque/Alpha/Add` on `draw3d.material` | how a material mixes with what's behind it | yes | inferring blending from alpha |
| `lerp` on `Color`, built-in `smoothstep`, `vec4(c)` / `Color(v)` | maths for colours without new module names | **yes** | `shader.mix`, `step`, `saturate` (clash with `noise.mix`; no overloading) |
| `shader.Canvas` | the input of a 2D shader | yes, in round 4 | reusing `Fragment` in 2D |
| `(px: Fragment, v: V, p: P)` | varyings as an optional middle parameter | yes, in round 4 | changing the first parameter's type |
| `KESTREL_GPU=off` | an environment variable for the runtime | yes | |
| `kestrel check --gpu-dump` | prints the generated GLSL | yes | |
| (later) fixed-size arrays, e.g. `[float; 9]` or another spelling | local arrays in GPU code (a 3x3 kernel, 128 blocks in a struct) | the spec already lists fixed-size arrays as out of scope; the user picks the spelling when needed | |

Runtime-internal natives (not visible to programs) are needed in `kestrel/window`: compile a compute program, create/update/read/bind a buffer, dispatch, memory barrier, timer query, and `LoadShaderFromMemory`, `GetShaderLocation`, `SetShaderValue`, `SetShaderValueTexture`.

**Deliberately not proposed:** atomics, shared workgroup memory, barriers, 2D/3D dispatch, reductions (`sum` over a `gpu_list`), and async readback with `await`. Each is a later round, driven by need.

## 7. Prior art

- **CUDA** ([docs](https://docs.nvidia.com/cuda/)): C++ kernels, hand-written copies, NVIDIA only, races unchecked. Kestrel: any vendor, no kernel language, copies made for you, races ruled out by the `par for` rule.
- **Mojo** ([docs](https://docs.modular.com/mojo/)): one language for CPU and GPU, but GPU code is written with thread indices and device contexts. Kestrel offers only the `par for` shape.
- **Futhark** ([futhark-lang.org](https://futhark-lang.org)): pure data-parallel `map`/`reduce`/`scan` compiled to GPUs with fusion; proof that pure code can be moved to a GPU automatically. Kestrel is imperative and a game language first.
- **Halide** ([halide-lang.org](https://halide-lang.org)): the algorithm apart from its "schedule" (tiling, GPU or CPU). Kestrel's schedule is one word: `par` or `par gpu`.
- **Taichi** ([taichi-lang.org](https://www.taichi-lang.org)): Python kernels whose outer `for` runs on the GPU, with a CPU fallback: the closest analogue to `par gpu for`. Kestrel's is statically checked.
- **rust-gpu** ([github](https://github.com/Rust-GPU/rust-gpu)): Rust compiled to SPIR-V, so shaders and game code share crates; the closest analogue to the shader package. Kestrel owns its language and already has `pure`, which makes the job smaller.
- **Slang** ([shader-slang.org](https://shader-slang.org)): a modern shading language (modules, generics) with many outputs; still a separate language from the game's.
- **Godot's shading language** ([docs](https://docs.godotengine.org/en/stable/tutorials/shaders/shader_reference/shading_language.html)): `vertex()`/`fragment()`, built-ins like `ALBEDO` and `TIME`, uniforms, live reload; it can't share code with GDScript, and Godot compute means hand-written GLSL and buffers through `RenderingDevice`. Kestrel keeps Godot's model (you write `fragment`, the engine does the rest) in the gameplay language.
- **Unity** ([Burst](https://docs.unity3d.com/Packages/com.unity.burst@latest)): Burst is a checked fast subset of C# for CPU jobs; shaders stay HLSL or Shader Graph. Kestrel would be Burst's idea carried to the GPU and to shaders.

## 8. Risks, rounds, and round 1

### The three biggest risks

1. **Driver and testing fragility.** The work is GL 4.3 on a 3.3-built raylib, our own GL calls mixed with raylib's state, and three vendors' GLSL compilers. **The user's PC can't test it while they game:** a hung GPU loop can reset the driver and crash the game, and GPU timings with a game running are meaningless. GitHub-style CI has no GPU. A software OpenGL (Mesa's `llvmpipe`, which has GL 4.5 and compute) could run the GPU tests on the CPU. That is worth a spike, but it tests correctness only, not speed or real drivers.
2. **The win is in data movement, not maths, and it is easy to oversell.** A copy-mode `par gpu for` over 1M particles loses to today's CPU `par for` (3 to 8 ms of copies against 3.3 ms of work). Residency must be the normal path. Part of Scene A's 41 ms is also available without a GPU: an instanced CPU draw that uploads 8 bytes a particle instead of 72 would shrink the 32 ms fill-and-upload to a few ms.
3. **"The same function" drifting between CPU and GPU.** Float rounding, `int()` and `normalized()` edge cases, `u8`/`bool`/`i64` emulation, checks that become a flag read later, and loop caps all have to keep Kestrel's meaning in a second backend. Every language feature from now on also needs a GPU answer, or a clear "not on the GPU" error. This is a second backend's ongoing cost.

Also: `noise.kes` needs `i64` (round 2), and the scheduling around `GEN_BATCH` would have to change for GPU generation.

### Rounds

- **Round 1: compute core** (below).
- **Round 2: data breadth.** Captured read-only lists, `u8`/`u16`/`bool` packing, `i64` emulation (with `GL_ARB_gpu_shader_int64` when present), enums with fields, `@prove(gpu)` on functions, a GPU section in `--report`, `--gpu-dump`. Demo: Scene B, measured against `par for`.
- **Round 3: fragment shaders.** The `shader` module, `draw3d.material`, textures in params, hot reload in `kestrel run`. Demo: Scene C and D.
- **Round 4: vertex shaders and drawing from `gpu_list`.** Varyings, instanced meshes from a `gpu_list` (a forest of trees, a field of grass), 2D `draw` shaders (Godot's `canvas_item`).
- **Later, by need:** reductions and atomics, workgroup memory, 2D/3D dispatch, async readback, the web via naga and WGSL.

### What round 1 builds

**Scope:** `par gpu for` over a `gpu_list[T]` and over a `list[T]`, where `T` is a struct of `int`/`u32`/`float`/`vec2`/`vec3`/`vec4`/`ivec*`/`Color`, with captured scalars, vectors and structs as uniforms. The emitter is GLSL 4.30 compute, with the error flag for checks and an automatic CPU fallback. `draw.points` draws from a `gpu_list`, and the particle demo goes to 1M.

That means: the parser word, the check-level subset rules, `Stmt::ParGpuFor` plus the implicit contract, the `kestrel/gpu` crate (layout, Kestrel-meaning helpers, emitter), runtime dispatch and buffers in `kestrel/window` (GL 4.3 detection and function loading), the fallback, and tests. Every GPU test program runs on both paths (GPU and `KESTREL_GPU=off`) and compares outputs within a stated tolerance, plus exact matches for integer-only kernels.

**Expected:** at 1M particles, frame work time goes from about 41 ms (measured) to about 2 to 4 ms (expected). Against an honest best-CPU baseline (`par for` plus instanced drawing, about 4 to 6 ms expected), that is roughly 2 times. The gap should widen at 5 to 10M particles, where the CPU step alone reaches 15 to 30 ms. Below about 50,000 items, the GPU's fixed cost (dispatch, barriers) probably makes it no faster.

**How to measure it honestly:**

- Use a hidden window (no vsync), as in `demos/particles/results.md`, at 100k, 1M and 5M.
- Compare three programs from the same build: today's demo, the new demo with `KESTREL_GPU=off`, and the new demo on the GPU.
- Give the median and range of frame work time.
- Read GPU time from GL timer queries (`GL_TIME_ELAPSED`), because CPU-side ticks around a queued dispatch measure only the queueing.
- Run only on a quiet machine with the user's say-so, never while they game.
- Report the losses (small counts, copy mode) in the same table as the wins.

### Open question for the user

Is the particle demo the right first target? The alternative is to put the terrain first (round 2's `i64` and `u8` work moved earlier), because that is where the engine actually waits on the CPU today. The particle round is smaller and proves the plumbing. The terrain round is the one the voxel engine would feel.
