# Kestrel

A compiled language for games: plain values, arenas instead of a garbage
collector, and (coming in sub-project 3) parallel loops over `pure` code.
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

`use window`, `use draw` and `use input` open a window, draw shapes and text, and read the keyboard and mouse. They are built into the compiler (raylib underneath), and work under both `kestrel run` and `kestrel build`. Windows only for now.

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

Move the mouse to pull the particles. Space adds 1,000 more. The window shows the update and draw times, and the demo prints them once a second. `kestrel/demos/particles/results.md` has the numbers: 60 fps (the display's refresh rate) up to about 100,000 particles, and about 30 fps at 200,000, where drawing one rectangle per call, not the `par for` update, is the bottleneck.

Two environment variables help with testing:

- `KESTREL_HEADLESS=<frames>` opens no window and runs that many frames, printing each `draw.*` call as a line instead (`KESTREL_HEADLESS_QUIET=1` silences them).
- `KESTREL_HIDDEN=1` opens a real window that isn't shown.

The API is in `docs/superpowers/specs/2026-10-10-window-design.md`.

## Layout

- `kestrel/`: the compiler, a Cargo workspace.
  - `syntax`: lexer, parser, AST and diagnostics.
  - `check`: names, types, purity, moves, exclusivity and the arena escape check.
  - `kir`: a typed, structured middle layer, plus lowering and inlining.
  - `codegen`: Cranelift JIT and object files.
  - `runtime`: arenas, printing and panics.
  - `window`: the window, drawing and input natives (raylib, vendored in `window/raylib`).
  - `driver`: loading modules and compiling a program, shared by the CLI and the Godot add-on. The built-in `window`, `draw` and `input` modules are in `driver/std`.
  - `cli`: the `kestrel` command.
  - `godot`: the Godot add-on and its demo project (built separately).
- `kestrel/demos/particles/`: the particles demo and its measured times.
- `kestrel/tests/programs/` and `kestrel/tests/errors/`: example programs with their expected output, and expected errors.
- `kestrel/bench/`: benchmarks against Rust and C, with results.
- `docs/superpowers/specs/`: the language (syntax spec) and compiler (core spec) designs.
- `docs/superpowers/plans/`: the implementation plans, done so far.

## Status

The core language works: functions, structs with methods, enums and `match`, vectors, strings, arrays, lists, `for in`, optionals, modules, bit operators, arenas, owning values that move, and checked arithmetic.

It also runs inside Godot 4: see [`kestrel/godot`](kestrel/godot/README.md). GDScript calls Kestrel functions, and edits hot-reload.

Also built: `map`, `par for` (loops that run on every core over `pure` code), global math and random numbers, and the 2D window above. Next: textures and 3D (the voxel-world viewer).

Speed versus Rust is between 1.0x and 1.5x on loops and structs, and about 3x on the voxel heightmap milestone. The numbers are in `kestrel/bench/*/results.md`.
