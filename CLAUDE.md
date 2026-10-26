# Kestrel

A compiled language for games (Rust + Cranelift). The compiler is the Cargo workspace in `kestrel/`. The old `kestrelc/` and `kestrel.js` implementations were removed in October 2026; they are in git history only.

## Start here

**Read `docs/superpowers/plans/2026-10-10-handoff.md` first.** Its top "Start here" section has the current status and the next task (generics are done, 2026-10-25; the voxel engine version 1 is built, reviewed and measured, 2026-10-26, in `kestrel/voxel/`, with its spec `docs/superpowers/specs/2026-10-25-voxel-engine-design.md` and plan `docs/superpowers/plans/2026-10-25-voxel-engine.md`; the KIR inliner now works bottom-up, 2026-10-26, so generation runs 1.64 times faster; next are the user's play test of `voxel/main.kes`, then race round 2 and save/load (both the user's call: they need new syntax or natives) or more compiler rounds from the engine's findings: `list.add` and `clear()`, lists in structs, dead functions after inlining); the rest has the user's decisions and preferences and environment gotchas.

## Where things are

- Language design: `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md`
- Compiler design: `docs/superpowers/specs/2026-10-09-kestrel-core-rebuild-design.md`
- Plans and open follow-ups: `docs/superpowers/plans/` (`2026-10-09-core-2a-followups.md` lists known gaps)
- The pipeline is `syntax → check → kir → codegen`. The runtime is linked as a staticlib (built by `cli/build.rs`).

## Commands (run in `kestrel/`)

```bash
cargo test --workspace          # unit, program (JIT + exe) and error tests
cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p kestrel-cli -- run file.kes
```

Program tests are `kestrel/tests/programs/*.kes`, with `// expect:`, `// expect-exit:` and `// expect-stderr:` lines. Error tests are `kestrel/tests/errors/*.kes`, with `// error <line>: <message>` lines.

## Conventions

- Keep source files under about 400 lines; split by responsibility.
- Error messages are plain sentences that say what to do.
- Never call a bare `link.exe`: Git's coreutils `link.exe` shadows the MSVC linker on PATH (`find-msvc-tools` locates the real one).
