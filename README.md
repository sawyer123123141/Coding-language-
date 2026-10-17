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

## Use

Needs Rust (stable) and, on Windows, Visual Studio's C++ build tools (for `link.exe`).

```bash
cd kestrel
cargo build --release
target/release/kestrel run program.kes        # compile in memory and run
target/release/kestrel build program.kes -o program.exe
target/release/kestrel check program.kes      # errors only
```

`kestrel build --release` turns off overflow and narrowing checks (they wrap instead). `kestrel run --release` does the same.

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

Also built: `map`, `par for` (loops that run on every core over `pure` code), global math and random numbers, and the window above with textures, 3D and the voxel viewer. Small structs now stay in registers. Next: a stronger (faster) backend.

Speed versus Rust is between 1.0x and 1.5x on loops and structs, and about 3x on the voxel heightmap milestone. The particle demo's update step is close now that small structs stay in registers: on one thread it takes 1.07 times Rust's time (1.15 times with the `pow` hoisted out, down from 1.5 and 3.4), but its copy loop is still about 4 times behind, and a whole frame with bulk drawing was only 2-7% behind before that change. The numbers are in `kestrel/bench/*/results.md`.
