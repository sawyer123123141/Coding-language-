# The Kestrel voxel engine

A voxel engine written in Kestrel (a Kestrel program, not a Cargo crate). It is built in
steps; this file grows with it. The design is in
`docs/superpowers/specs/2026-10-25-voxel-engine-design.md`.

The files are flat because Kestrel finds a module only next to the file that uses it.

## Files so far

- `grid.kes`: `Grid[T]`, a typed 3D array over one list, and the `Solid` interface (a library).
- `noise.kes`: the frozen world's 3D value noise, copied bit for bit (a library).
- `blocks.kes`: block ids, the world's sizes (`SIZE`, `LAYERS`, `HEIGHT`, `PAD`, `DIRT_DEPTH`, `SNOW_LINE`), colours and solidity (a library).
- `column.kes`: `Column`, `ColumnState`, column keys and the world-to-column maths, which is right for negative x (a library).
- `gen.kes`: the world's density, `solid_at`, `block_type` and `generate`, which fills a column's grid in place (a library).
- `ring.kes`: `Ring[T]`, a fixed-size first-in first-out queue (a library).
- `pool.kes`: `Pool[T]` and `Handle`, fixed slots handed out with generation-checked handles (a library).
- `parity.kes` and `parity_*.kes`: the frozen voxel benchmark rebuilt on `Grid[T]` (a benchmark module, not a library), with its entry files; timed by `bench/engine/run_parity.ps1`, results in `bench/engine/results.md`.
- `tests/`: test programs, one per library or engine part (`tests/grid.kes`, ...), plus the
  runner's own self-tests (`tests/runner_*.kes`, with `tests/fixtures/`).

## Running the tests

Run these in `kestrel/`:

```
cargo test -p kestrel-cli --test voxel_engine
KESTREL_TEST_ONLY=grid cargo test -p kestrel-cli --test voxel_engine    # tests whose file name starts with grid
```

`cli/tests/voxel_engine.rs` runs each `tests/<name>.kes` in a scratch folder under `target/tmp`
(`voxel_<name>` for the `kestrel run` pass, `voxel_<name>_exe` and `voxel_<name>_release` for the
others), as `test_<name>.kes` beside copies of every `kestrel/voxel/*.kes`: once with
`kestrel run`, once as an executable (`kestrel build`), and with `kestrel run --release` when the
file says `// release` (the release run is compared with the same `// expect...` lines). The directives at the end of a test are the program tests' (`// expect:`,
`// expect-exit:`, `// expect-stderr:`, `// env:`) and these:

- `// copy: <repo-relative file>`: copies another file into the scratch folder.
- `// replace: <file> <old> => <new>`: replaces literal text in that scratch copy only.
- `// release`: also runs the test with `--release`.
- `// only-jit`: skips the executable run (say why in the file).

A program that doesn't compile exits non-zero before it runs; the runner compares that exit code
and the messages with `// expect-exit:` and `// expect-stderr:`, so a test can expect a compile
error.
