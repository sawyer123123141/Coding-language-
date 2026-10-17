//! Operations that `ExprKind::Math` and `ExprKind::Map` perform.

/// Float operations: the operands and the result all have the expression's type (F32 or F64).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathOp {
    Sqrt,
    Floor,
    Ceil,
    Round,
    Abs,
    Min,
    Max,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Exp,
    Log,
    Pow,
}

/// What `ExprKind::Map` does with the map at its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapOp {
    /// `[key]`: an optional holding the value (bytes copied).
    Get,
    /// `[key]`: a bool.
    Has,
    /// `[key]`: removes the key; a bool, true when it was there.
    Remove,
    /// `[]`: the number of entries used, removed ones included (i64).
    Used,
    /// `[index]`: whether entry `index` is below `Used` and still there.
    Live,
    /// `[]`: starts a loop over the map; the old lock count (i64), for `MapUnlock`.
    Lock,
}

/// What `Stmt::List` does with the list at its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListOp {
    /// `[index]`: removes that item, moving the later ones down and zeroing the vacated slot. An
    /// owning item is dropped first; the runtime panics when the index is out of range.
    RemoveAt,
    /// `[index, value]`: opens a slot at `index` (`index == len` adds at the end), moving the
    /// later ones up, and stores `value` in it. `value` is evaluated before anything moves.
    Insert,
}
