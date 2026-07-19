# Kestrel

A toy programming language focused on speed: compile-time purity
checking, compile-time bounds proofs, and (per the design doc) a
persistent cross-run optimization cache and layout polymorphism still
to come. "Kestrel" is a placeholder name.

See [`kestrel-DESIGN.md`](./kestrel-DESIGN.md) for the full design
rationale and status of each idea, and [`docs/SYNTAX.md`](./docs/SYNTAX.md)
for the syntax reference and grammar.

## Structure

- `kestrel.js` — lexer, parser, purity checker, bounds-proof notes, and
  two backends: `Kestrel.run` (tree-walking interpreter) and
  `Kestrel.runFast` (bytecode compiler + stack VM). Zero dependencies;
  runs unmodified in Node or as a browser `<script>`.
- `kestrel-editor.html` — a single-file mobile code editor/IDE (embeds
  `kestrel.js` inline; add to iPhone home screen via Safari for an
  app-like experience). Auto-deployed to GitHub Pages on every push to
  `main` (see `.github/workflows/pages.yml`) — once Pages is enabled in
  repo Settings, it's served live at the repo's Pages URL.
- `docs/SYNTAX.md` — syntax reference and full grammar.
- `examples/` — runnable example programs:
  - `basics.kes` — `pure fn`, arrays, `where`-bounded access.
  - `fibonacci.kes` — recursion.
  - `purity_violation.kes` — a program that's *meant* to fail the
    purity check, for testing the checker itself.
- `test/` — automated test suite (Node's built-in `node:test`, no
  dependencies).

## Running

```sh
node -e 'require("./kestrel.js").run(require("fs").readFileSync("examples/basics.kes", "utf8"))'
```

Swap `.run(` for `.runFast(` to use the bytecode VM instead — same
output, same errors. It's not uniformly faster yet (see Status below),
so `run` is still the safer default.

## Testing

```sh
npm test
```

## Status

Two backends now exist — `run` (tree-walking) and `runFast` (bytecode
VM) — and are semantics-identical, but `runFast` is only faster on
loop/array-heavy code (~40-54% in early benchmarks) and currently
slightly *slower* on deep-recursion-heavy code. See the benchmark table
and honest writeup in `kestrel-DESIGN.md` before picking one for
performance-sensitive code. Next up, in priority order: closing that
recursion gap, then a native (LLVM/Cranelift) backend, the persistent
cross-run optimization cache, layout polymorphism, and a more general
proof system.
