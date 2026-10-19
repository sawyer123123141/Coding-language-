//! Functions compiled Kestrel code calls into. Every export is
//! `extern "C"` with a fixed name; codegen declares matching imports.

pub mod arena;
mod blocks;
pub mod gpu;
pub mod list;
pub mod map;
pub mod math;
pub mod panic;
pub mod par;
pub mod print;
pub mod sort;
pub mod start;
pub mod string;
pub mod string_find;
pub mod string_make;
pub mod string_parse;

pub use panic::{PANIC_ABSTRACT, PANIC_DIV_ZERO, PANIC_MAP_CHANGED, PANIC_NARROW_INT, PANIC_NARROW_U8, PANIC_OVERFLOW, PANIC_SHIFT};
