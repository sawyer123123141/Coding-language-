//! Where relative paths in a program point: the runtime's rule (`kestrel_runtime::natives::paths`),
//! which the `file` module follows too, so `window.screenshot`, `draw.load_texture` and
//! `file.read` find the same files.

pub use kestrel_runtime::natives::paths::{display, resolve, set_program_dir};
