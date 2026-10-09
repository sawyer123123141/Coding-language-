//! Functions compiled Kestrel code calls into. Every export is
//! `extern "C"` with a fixed name; codegen declares matching imports.

pub mod panic;
pub mod print;
pub mod start;

pub use panic::{PANIC_DIV_ZERO, PANIC_OVERFLOW};
