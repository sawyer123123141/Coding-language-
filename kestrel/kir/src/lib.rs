pub mod inline;
pub mod ir;
mod layout;
mod leaves;
mod lower;
mod native;
mod ops;
mod rename;
pub mod scalarize;
mod split;
pub mod visit;
pub mod visit_mut;

pub use ir::*;
pub use layout::*;
pub use lower::lower;
pub use native::native_symbol;

/// Inlines small functions, then keeps plain structs in registers (see `scalarize`). The driver
/// runs it on every program.
pub fn optimize(m: &mut Module) {
    inline::inline_small(m);
    scalarize::scalarize(m);
}
