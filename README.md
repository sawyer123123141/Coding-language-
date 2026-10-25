# Kestrel

A compiled language for games: plain values, arenas instead of a garbage
collector, and parallel loops (`par for`) over `pure` code.
It compiles through Cranelift to native code, either JIT-run in place or
built into an executable.

```kestrel
struct Chunk {
    x: int
    mem: Arena = Arena()
    heights: [int] = []

    generate(var self) {
        in self.mem {
            self.heights = array[int](32 * 32)
        }
        for i, h in self.heights {
            self.heights[i] = self.x + i % 32
        }
    }
}

main() {
    var world = list[Chunk]()
    for x from 0 to 4 { world.add(Chunk(x: x)) }
    for chunk in world { chunk.generate() }
    print("chunks: {world.len}, first: {world[0].heights[1]}")
}
```

Lists and strings have the everyday methods, named after GDScript's (`remove_at`, `pop`, `insert`, `clear`, `has`, `find`, `join`, `split`, `slice`, `upper`, `trim`, `to_int`). `remove b` inside a `for b in xs` loop removes the current item without skipping the next one, which is where `erase` in a GDScript loop goes wrong:

```kestrel
main() {
    var bricks: list[int] = [1, 2, 3, 4, 5, 6]
    var score = 0
    for b in bricks {
        if b % 2 == 0 {
            score += b
            remove b
        }
    }
    print(bricks, score)    // [1, 3, 5] 12
}
```

Strings are UTF-8, and lengths and indexes count characters, so `"héllo".len` is 5. Methods that can fail, like `find` and `to_int`, return an optional (`none`) where GDScript returns -1 or 0:

```kestrel
main() {
    var words: list[string] = []
    for part in "go, stop ,  jump".split(",") {
        words.add(part.trim().upper())
    }
    print(words.join("-"), words.len)       // GO-STOP-JUMP 3
    let name = "héllo"
    print(name.len, name[1], name.slice(1, 4), name.find("l"))    // 5 é éll 2
    let n = " 42 ".to_int() else { return }
    print(n + 1)                            // 43
}
```

The full list is in the "Lists and strings" section of `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md`.

Functions are values, and closures capture by value, as in GDScript. Lists have `filter`, `map`, `any`, `all`, `sort` and `sort_by`. A closure literal passed straight to one of these (or to a function) allocates nothing, and the compiler inlines it into the loop. A closure can't change what it captured, so `for` is still how you accumulate:

```kestrel
make_adder(k: int) -> (int) -> int {
    (x) { x + k }
}

main() {
    let add2 = make_adder(2)
    var hp: list[int] = [30, 0, 12, 7]
    let alive = hp.filter((h) { h > 0 })
    print(alive.map(add2))                  // [32, 14, 9]
    hp.sort_by((h) { -h })                  // largest first
    print(hp, hp.any((h) { h == 0 }))       // [30, 12, 7, 0] true
}
```

Sorting is stable, strings sort by code point, and `sort_by` keys must be `pure`. Methods, built-ins and natives aren't values yet; wrap them in a closure. The details are in the "Closures and function values" section of the same file.

Interfaces name a set of methods that several structs or enums can promise to have, so one list can hold goblins and barrels and a call runs the right method for each (a method table, like a vtable). The compiler checks the promise, where GDScript's duck typing (`has_method`) checks it when the line runs. A struct passed straight to an interface parameter is lent, not copied; storing one makes a copy in the current arena, like a string:

```kestrel
interface Damageable {
    take_damage(var self, amount: int)
    pure hp(self) -> int
}

struct Goblin: Damageable {
    health: int = 10
    take_damage(var self, amount: int) { self.health -= amount }
    pure hp(self) -> int { self.health }
}

struct Barrel: Damageable {
    oil: int
    take_damage(var self, amount: int) { self.oil -= amount * 2 }
    pure hp(self) -> int { self.oil }
}

show(d: Damageable) { print(d.hp()) }       // d is the caller's Goblin, not a copy

main() {
    var foes: list[Damageable] = [Goblin(), Barrel(oil: 4)]
    for f in foes { f.take_damage(2) }
    print(foes.map((f) { f.hp() }))         // [8, 0]
    print(foes)                             // [Goblin(health: 8), Barrel(oil: 0)]
    let g = Goblin()
    show(g)                                 // 10
}
```

A struct that owns an `Arena` or a GPU object can be lent to an interface but not stored as one. There is no `is` test to get the struct back out yet. A call through an interface is an indirect call that isn't inlined: about 5 ns where a direct call is under 1, and about 15% behind Rust's `Box<dyn Trait>` (`kestrel/bench/kernels/results.md`). The details are in the "Interfaces" section of the syntax file.

## Use

Needs Rust (stable) and, on Windows, Visual Studio's C++ build tools (for `link.exe`).

```bash
cd kestrel
cargo build --release
target/release/kestrel run program.kes        # compile in memory and run
target/release/kestrel build program.kes -o program.exe
target/release/kestrel check program.kes      # errors only
target/release/kestrel check --report program.kes   # what is left in each function and loop
```

`kestrel build --release` turns off overflow and narrowing checks (they wrap instead). `kestrel run --release` does the same.

**The compiler leaves out each check it can prove won't fail.** `for i from 0 to xs.len { total += xs[i] }` has no bounds check on `xs[i]`, and `h & 0x7FFFFFFF` times a 31-bit constant has no overflow check, in every build. `kestrel check --report` shows what stayed, per function and loop, with the line and the reason, and the allocations and other ways to fail (`--report --release` for a `--release` build):

```
sum.kes
  function `sum_all` (line 1): its own copy is never called; its checks are listed where it is inlined
  function `main` (line 9): 1 check left (2 removed), 1 allocation
    loop at line 3 (in `sum_all`)
      line 4 (in `sum_all`): overflow check: `total + xs[i]` may exceed `int` (`total` can be any `int`; `xs[i]` can be any `int`)
    line 10: allocation: `[...]` makes a new list
total: 3 checks, 2 removed, 1 left; 1 allocation; no other ways to fail
```

A list or an array holds at most 2,147,483,647 items.

**`@prove` and `@assume`.** `@prove` is a promise the compiler checks, and a build that breaks it fails with the chain of calls that broke it. `@assume` tells the compiler a fact it can use, and checks it where it is written.

```kestrel
@prove(no_alloc, no_checks)
pure step(var p: Particle, dt: float) { ... }

pure at(xs: [int], i: int) -> int {
    @assume(i >= 0 and i < xs.len)
    xs[i]
}
```

- `@prove` takes `no_alloc` (nothing allocates), `no_checks` (no overflow, shift, division, narrowing or bounds check is left), `no_panic` (no check and no other way to fail) and `inline` (every call was inlined). `print` isn't counted by `no_alloc` (the runtime makes a thread's output buffer on its first print, and in the Godot add-on output is captured and grows on every call). It goes before a function, a method, an interface method, or a `for`, `par for` or `while` loop, and covers what the code calls and every place it was inlined. `kestrel check --report` lists what is left in it.
- `@assume(cond)` holds to the end of its block. A checked build checks it and a false one stops the program ("assumption `i < xs.len` is false on line 3 of f.kes"); under `--release` one that only removes arithmetic checks is trusted, and one that removes a bounds check stays one check. `kestrel check` warns when an assumption removes nothing.
- `@prove` and `@assume` make `kestrel check` run the whole analysis, so it takes about 50-80 ms longer on the voxel world, and about 250 ms longer with an `@assume` around a big block (the inlined noise code: `check` then analyses both builds, running the block twice in each); that assumption makes the build itself 65-70% slower there, so put one near what it helps. `kestrel/bench/voxels3d/results.md` has the numbers.

**Vectors and wrapping operators.** `vec2`, `vec3`, `vec4` and `ivec2`, `ivec3`, `ivec4` are Godot's `Vector2`/`Vector3`/`Vector4` and `Vector2i`/`Vector3i`/`Vector4i`, with one set of rules. `vec4` and `ivec4` are real SIMD values: one 16-byte register, and `+`, `*`, `^`, `>>` and the rest do all four lanes in one instruction (the CPU needs SSE4.1, which every x64 CPU since about 2011 has). `vec4(0.5)` fills all four components, `vec4(v3, w)` and `vec4(a.xy, b.zw)` join parts, `v.wzyx` swizzles, `v.sum()` adds the lanes, and `min`, `max`, `clamp`, `abs` and `sqrt` work lane by lane. `==` gives one `bool` (every lane equal), and `<` doesn't exist on vectors. An `ivec4` has no `/` or `%` (the CPU can't divide integer lanes at once: use `>>` for a power of two, or divide each lane). An `ivec4` is checked per lane like an `int`: a checked build stops on an overflowing lane, `--release` wraps, and range analysis removes the checks it can prove. **`*% +% -%` wrap and are never checked**, in any build, which is what a hash wants (GDScript's `int` is 64-bit and wraps silently; Kestrel's `*` stops on overflow, so `*%` is how you ask for the wrap). Hex literals may use their type's whole width, so `0x9E3779B1` is an `int`:

```kestrel
hash(h: ivec4) -> ivec4 {
    let a = (h ^ (h >> 15)) *% 0x2C1B3C6D
    (a ^ (a >> 12)) *% 0x297A2D39
}

main() {
    let pos = vec4(1, 2, 3, 4)
    let moved = pos + vec4(0.5) * 2.0
    print(moved, moved.sum(), moved.wzyx)   // vec4(x: 2, y: 3, z: 4, w: 5) 14 vec4(x: 5, y: 4, z: 3, w: 2)
    let h = hash(ivec4(10, 20, 30, 40))     // four hashes at once
    print(h.x != 0)                         // true
    var n = 1_000_003
    n *%= 0x9E3779B1                        // wraps, never checked
    print(n)                                // -683443373
}
```

On a 2D value noise, four samples at once in `vec4`/`ivec4` run 2.5 times faster per sample than the scalar version, and take 1.08 times as long as a hand-written Rust SSE4.1 version (SSE, not AVX; Rust's own four-at-once version is 2.6 times its scalar one), and a checked build costs nothing measurable because the analysis proves every lane check (`kestrel/bench/kernels/results.md`, "SIMD vectors"). Four corners of one sample at once gain only about 1.25 times. Natives can't take `vec4` or `ivec4` yet, and Godot exports of them aren't built. The rules are in the "Vectors" bullet of the syntax file.

## Editor

VS Code (or any editor that speaks the Language Server Protocol) gets Kestrel's real errors as you type, the type of what is under the cursor, go to definition (into module files and the built-in modules too), and the **cost lens**: every check, allocation and other way to fail that the compiler leaves in a line, shown on that line with the reason, and the `@assume` or wrapping operator that removes it when there is a simple one. It is `kestrel check --report` in the editor, and runs when a file is opened or saved.

```text
pub get(xs: list[int], k: int) -> int {
    xs[k]    bounds check: `xs[k]` may be outside `xs` (`k` can be any `int`)
}
```

To install it locally (it isn't on the marketplace), from `kestrel/`:

```bash
cargo build --release -p kestrel-lsp
cd editors/vscode
npm install
npx @vscode/vsce package --skip-license --allow-missing-repository
code --install-extension kestrel-0.1.0.vsix
```

Then set `kestrel.server.path` to the built `kestrel/target/release/kestrel-lsp.exe` (or put that folder on `PATH`) and reload VS Code. The settings `kestrel.costLens.enabled`, `kestrel.costLens.build` (`checked` or `release`) and `kestrel.costLens.show` (`hints`, `diagnostics` or `both`) control the lens. A file with `pub` items and no `main` is checked as a module. [`kestrel/editors/vscode/README.md`](kestrel/editors/vscode/README.md) has the details and what isn't built yet.

## A window

Four modules are built into the compiler (raylib underneath), and work under both `kestrel run` and `kestrel build`. Windows only for now.

- `use window`: open the window, run the frame loop, time, screenshots.
- `use draw`: shapes (`draw.rect`, or `draw.fill` for a `Rect`), text, textures and sprites, and `draw.rects` to draw thousands of rectangles in one call.
- `use draw3d`: a camera, meshes you build in Kestrel (`MeshData`, like filling an `ArrayMesh`) and upload to the GPU once, and a few shapes.
- `use input`: keyboard and mouse.

```kestrel
use window
use draw

main() {
    window.open(640, 360, "Hello")
    while window.frame() {
        draw.clear(draw.BLACK)
        draw.circle(vec2(320, 180), 40, draw.ORANGE)
        draw.text("fps {window.fps()}", vec2(10, 10), 20, draw.WHITE)
    }
}
```

The showcase is a particle demo: tens of thousands of dots pulled toward the mouse, moved by a `par for`. Run it from `kestrel/`:

```bash
target/release/kestrel run --release demos/particles/particles.kes
```

Move the mouse to pull the particles. Space adds 1,000 more. The window shows the update and draw times, and the demo prints them once a second. It draws all the dots with one `draw.rects` call: 200,000 dots draw in about 4.6 ms (a frame takes 5.4 ms), against about 29 ms when each dot was its own `draw.rect` call. `kestrel/demos/particles/results.md` has the numbers, and `kestrel/bench/particles/results.md` compares the demo with a Rust port on the same raylib (Kestrel is 2-7% behind there).

The other showcase is a voxel viewer: fly through the 8x4x8-chunk world of `bench/voxels3d` (463,749 faces) in 3D. The top block of each column is grass (snow on the peaks), with dirt under it and rock deeper down, and soft shading in the corners where blocks meet, all baked into the vertex colours (there is no lighting).

```bash
target/release/kestrel run --release demos/voxels/voxels.kes
```

- Hold the right mouse button and drag to look, WASD to move, Space and Shift to go up and down, the mouse wheel to change speed.
- At start a `par for` builds every chunk's mesh on all cores (about 80 ms, estimated) and a loop copies them to the GPU (about 55 ms). Then each frame draws all 248 meshes, with an overlay of the fps, frame time and face count.
- At the default size it holds 60 fps, the display's rate. That number is capped: every hidden window was held at 60 fps while the viewer was measured, so the headroom was found by growing the world (the constant `WIDTH` in `voxels.kes`). 60 fps (capped) still holds at 4.2 million faces, 9 times the world, and drops just below at 7.3 million. `kestrel/demos/voxels/results.md` has the numbers and what limits it.

Two environment variables help with testing:

- `KESTREL_HEADLESS=<frames>` opens no window and runs that many frames, printing each `draw.*` call as a line instead (`KESTREL_HEADLESS_QUIET=1` silences them).
- `KESTREL_HIDDEN=1` opens a real window that isn't shown.

The API is in `docs/superpowers/specs/2026-10-10-window-design.md` (window, shapes, text, input) and `docs/superpowers/specs/2026-10-10-3d-textures-design.md` (textures, `draw.rects`, `draw3d`).

## Layout

- `kestrel/`: the compiler, a Cargo workspace.
  - `syntax`: lexer, parser, AST and diagnostics.
  - `check`: names, types, purity, moves, exclusivity and the arena escape check.
  - `kir`: a typed, structured middle layer, plus lowering and inlining.
  - `codegen`: Cranelift JIT and object files.
  - `runtime`: arenas, printing and panics.
  - `window`: the window, drawing and input natives (raylib, vendored in `window/raylib`).
  - `driver`: loading modules and compiling a program, shared by the CLI and the Godot add-on. The built-in `window`, `draw`, `draw3d` and `input` modules are in `driver/std`.
  - `cli`: the `kestrel` command.
  - `lsp`: the language server, `kestrel-lsp`.
  - `editors/vscode`: the VS Code extension (highlighting, and the client that starts `kestrel-lsp`).
  - `godot`: the Godot add-on and its demo project (built separately).
- `kestrel/demos/particles/`: the particles demo and its measured times.
- `kestrel/demos/voxels/`: the voxel viewer (`voxels.kes`, `world.kes`, `fly.kes`) and its measured times.
- `kestrel/tests/programs/` and `kestrel/tests/errors/`: example programs with their expected output, and expected errors.
- `kestrel/bench/`: benchmarks against Rust and C, with results.
- `docs/superpowers/specs/`: the designs: the language (syntax), the compiler (core), the window, and textures and 3D.
- `docs/superpowers/plans/`: the implementation plans, done so far.

## Status

The core language works: functions, structs with methods, enums and `match`, vectors, strings, arrays, lists, `for in`, optionals, modules, bit operators, arenas, owning values that move, and checked arithmetic.

It also runs inside Godot 4: see [`kestrel/godot`](kestrel/godot/README.md). GDScript calls Kestrel functions, and edits hot-reload.

Also built: closures and function values with the list methods that take them, interfaces (checked method sets with dynamic dispatch), `map` (the hash map), `par for` (loops that run on every core over `pure` code), range analysis (checks the compiler proves can't fail are left out, and `kestrel check --report` lists the rest), `@prove` performance contracts and `@assume`, global math and random numbers, and the window above with textures, 3D and the voxel viewer. Small structs now stay in registers, and `vec4`/`ivec4` are SIMD values with the wrapping operators `*% +% -%` (built 2026-10-24). Next: the full test suite over that work (its final review and fixes are done), the Cranelift upgrade, and a no-rules engine version of the voxel world.

Speed versus Rust is between 1.0x and 1.5x on loops and structs, and about 3x on the voxel heightmap milestone. The particle demo's update step is close now that small structs stay in registers: on one thread it takes 1.07 times Rust's time (1.15 times with the `pow` hoisted out, down from 1.5 and 3.4), but its copy loop is still about 4 times behind, and a whole frame with bulk drawing was only 2-7% behind before that change. Leaving out the checks the compiler proves can't fail makes the 3D voxel world with checks on 2.2 times faster (1.4 s to 0.64 s, now 1.09 times its `--release` time, was 2.4 times) and a `for i` loop over `parts[i]` 26% faster; it adds 40-70 ms to the build of a big program like that one. The numbers are in `kestrel/bench/*/results.md`.
