//! Functions compiled Kestrel code calls into. Every export is
//! `extern "C"` with a fixed name; codegen declares matching imports.

pub mod arena;
pub mod panic;
pub mod print;
pub mod start;

pub use panic::{PANIC_DIV_ZERO, PANIC_NARROW_INT, PANIC_NARROW_U8, PANIC_OVERFLOW, PANIC_SHIFT};
