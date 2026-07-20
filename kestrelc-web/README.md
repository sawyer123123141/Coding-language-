# kestrelc-web

`kestrelc` itself, compiled to WebAssembly, so `kestrel-editor.html` can
compile Kestrel source to a runnable `.wasm` module entirely
client-side — no server, no native `kestrelc` binary involved.

## Building

```sh
rustup target add wasm32-unknown-unknown   # once
cargo build --release --target wasm32-unknown-unknown
# -> target/wasm32-unknown-unknown/release/kestrelc_web.wasm
```

`.github/workflows/pages.yml` builds this automatically on every push to
`main` and publishes the result as `kestrelc.wasm` alongside the editor.

## Interface

No [wasm-bindgen](https://github.com/rustwasm/wasm-bindgen) — matches
the rest of this project's zero-build-step, zero-JS-dependency ethos.
This is a raw C ABI over manually managed linear memory instead:

- `alloc(len: usize) -> ptr: u32` — allocates `len` bytes, returns the
  pointer. The caller (JS) writes data into the module's own memory at
  that address.
- `compile(src_ptr: u32, src_len: usize) -> header_ptr: u32` — compiles
  the UTF-8 Kestrel source at `[src_ptr, src_ptr+src_len)`. Returns a
  pointer to a 9-byte result header: `[ok: u8][len: u32 LE][ptr: u32 LE]`.
  If `ok`, the bytes at `[ptr, ptr+len)` are a compiled `.wasm` module,
  ready for `WebAssembly.instantiate`. If not, they're a UTF-8 error
  message, formatted identically to the native CLI's errors.
- `dealloc(ptr: u32, len: usize)` — frees memory returned by `alloc` or
  produced by `compile`, once the caller is done reading it.

See the `runNative()`/`loadCompiler()` functions in
`kestrel-editor.html` for a complete, working example of driving this
from plain JS with no dependencies.

## Scope

Same as `kestrelc`'s native WASM backend (`kestrelc --wasm`) — see
`../kestrelc/README.md`. No arrays yet. Cranelift and everything it
depends on (native-only: probes the host CPU, needs a real object-file
writer) is excluded from this build entirely via `kestrelc`'s `native`
Cargo feature, not just unused — see `../kestrelc/Cargo.toml`.
