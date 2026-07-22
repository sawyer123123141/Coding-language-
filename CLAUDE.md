# Kestrel — Coding Language

## Project Summary
Kestrel is a compiled programming language with compile-time purity checking, bounds proof verification, and native compilation.

## Core Concepts
- **Purity System**: Functions marked `@pure` enable fearless parallelism
- **Bounds Proofs**: Compile-time verification of array bounds (eliminates runtime checks)
- **Type System**: Static typing with constraint-based reasoning
- **Backends**: 
  - JS interpreter (kestrel.js)
  - Native Rust compiler (via Cranelift AOT/JIT — `kestrelc`)

(The old WASM backend — `kestrelc-web` + the browser-based `kestrel-editor.html` playground — was removed; `kestrelc-devtool` is the current native dev tool.)

## Directory Structure
```
kestrelc/           - Rust compiler (primary implementation)
kestrelc-devtool/   - Native devtool (AOT/JIT, no WASM)
kestrel.js          - JavaScript interpreter/backend
test/               - Test suite
examples/           - Example programs
docs/               - Documentation
```

## Key Entry Points
- **Compiler**: `kestrelc/main.rs` or `kestrelc/lib.rs`
- **JS Backend**: `kestrel.js`
- **Devtool**: `kestrelc-devtool/`
- **Design Doc**: `kestrel-DESIGN.md` (for architectural details)

## Development Commands
```bash
cargo build          # Build compiler
cargo test           # Run tests
npm run build        # Build JS backend
```

## Common Mistakes
(Add mistakes you encounter frequently here)

## Recent Sessions
(Sessions archived in `.claude/completions/`)
