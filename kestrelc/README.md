# kestrelc

A native compiler for Kestrel, using [Cranelift](https://cranelift.dev/)
to emit a real standalone executable — no VM, no interpreter loop at
runtime at all. This is a separate Rust program from `kestrel.js`; it
doesn't run in the browser editor.

**Status: in progress.** This file will be updated with what's actually
supported and real, measured benchmark numbers once there's a working
end-to-end pipeline (compile a `.kes` file, link it, run the resulting
binary, verify its output matches `Kestrel.run`/`Kestrel.runFast`). Until
then, treat this directory as under construction — see
`kestrel-DESIGN.md` for the reasoning behind building this at all.
