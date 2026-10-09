pub mod inline;
pub mod ir;
mod lower;
mod rename;
pub mod visit;

pub use ir::*;
pub use lower::lower;
