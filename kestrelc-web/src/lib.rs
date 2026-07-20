// The browser-facing shim: exposes kestrelc's front end + WASM backend
// as a WASM module itself, so kestrel-editor.html can compile Kestrel
// source to a runnable .wasm module entirely client-side — no server,
// no native kestrelc binary involved.
//
// No wasm-bindgen on purpose (matches the project's zero-build-step,
// zero-dependency ethos elsewhere): this is a raw C ABI over manually
// managed linear memory. The host (JS) calls `alloc` to get a buffer,
// writes the Kestrel source into it, calls `compile`, and reads a small
// result header back out of memory itself. It's more legwork on the JS
// side than wasm-bindgen's generated glue would be, but it means the
// editor can call this with nothing more than the browser's built-in
// `WebAssembly` API — no bundler, no codegen step, no npm.

use std::mem;

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    mem::forget(buf); // ownership transfers to the caller until dealloc()
    ptr
}

#[no_mangle]
pub extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    unsafe {
        drop(Vec::from_raw_parts(ptr, len, len));
    }
}

/// Compiles the `src_len` bytes of (UTF-8) Kestrel source at `src_ptr`
/// and returns a pointer to a 9-byte result header, itself allocated the
/// same way as `alloc` (the caller must `dealloc` it, and separately
/// `dealloc` the data it points to, once done reading both):
///
/// ```text
/// byte 0:    1 if compilation succeeded, 0 if `data` is an error message
/// bytes 1-4: length of `data`, little-endian u32
/// bytes 5-8: pointer to `data` in this module's memory, little-endian u32
/// ```
///
/// On success, `data` is the compiled .wasm module's bytes — ready to
/// pass straight to `WebAssembly.instantiate`. On failure, `data` is a
/// UTF-8 error message, formatted exactly like kestrelc's CLI errors.
#[no_mangle]
pub extern "C" fn compile(src_ptr: *const u8, src_len: usize) -> *const u8 {
    let src_bytes = unsafe { std::slice::from_raw_parts(src_ptr, src_len) };
    match std::str::from_utf8(src_bytes) {
        Ok(src) => match kestrelc::compile_to_wasm_bytes(src) {
            Ok(wasm_bytes) => make_result(true, &wasm_bytes),
            Err(msg) => make_result(false, msg.as_bytes()),
        },
        Err(_) => make_result(false, b"kestrelc: source is not valid UTF-8"),
    }
}

fn make_result(ok: bool, data: &[u8]) -> *const u8 {
    let mut owned = data.to_vec();
    let ptr = owned.as_mut_ptr();
    let len = owned.len();
    mem::forget(owned);

    let mut header = Vec::with_capacity(9);
    header.push(if ok { 1 } else { 0 });
    header.extend_from_slice(&(len as u32).to_le_bytes());
    header.extend_from_slice(&(ptr as u32).to_le_bytes());
    let header_ptr = header.as_ptr();
    mem::forget(header);
    header_ptr
}
