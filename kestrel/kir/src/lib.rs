pub mod inline;
pub mod ir;
mod ir_expr;
mod ir_stmt;
mod layout;
mod leaves;
mod lower;
mod native;
mod ops;
pub mod promote;
mod rename;
pub mod scalarize;
mod split;
pub mod visit;
pub mod visit_mut;

pub use ir::*;
pub use layout::*;
pub use lower::lower;
pub use native::native_symbol;

/// Inlines small functions, then keeps plain structs and the fields of `var` parameters and loop
/// elements in registers (see `scalarize` and `promote`). The driver runs it on every program.
pub fn optimize(m: &mut Module) {
    inline::inline_small(m);
    scalarize::scalarize(m);
    promote::promote(m);
}
