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

`kestrel build --release` turns off overflow and narrowing checks (they wrap instead).

## Layout

- `kestrel/`: the compiler, a Cargo workspace.
  - `syntax`: lexer, parser, AST and diagnostics.
  - `check`: names, types, purity, moves, exclusivity and the arena escape check.
  - `kir`: a typed, structured middle layer, plus lowering and inlining.
  - `codegen`: Cranelift JIT and object files.
  - `runtime`: arenas, printing and panics.
  - `cli`: the `kestrel` command.
- `kestrel/tests/programs/` and `kestrel/tests/errors/`: example programs with their expected output, and expected errors.
- `kestrel/bench/`: benchmarks against Rust and C, with results.
- `docs/superpowers/specs/`: the language (syntax spec) and compiler (core spec) designs.
- `docs/superpowers/plans/`: the implementation plans, done so far.

## Status

The core language works: functions, structs with methods, enums and `match`, vectors, strings, arrays, lists, `for in`, arenas, owning values that move, and checked arithmetic.

Next:
1. Modules, optional values (`T?`) and `map`.
2. A Godot bridge.
3. `par for`.

Speed versus Rust is between 1.0x and 1.5x on loops and structs, and about 3x on the voxel heightmap milestone. The numbers are in `kestrel/bench/*/results.md`.
