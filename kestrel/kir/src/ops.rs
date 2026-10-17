//! Operations that `ExprKind::Math`, `ExprKind::Map` and `ExprKind::Str` perform.

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
    /// `[]`: empties the list, zeroing the items (an owning item was dropped first) and keeping the
    /// capacity.
    Clear,
    /// `[index, value]`: opens a slot at `index` (`index == len` adds at the end), moving the
    /// later ones up, and stores `value` in it. `value` is evaluated before anything moves.
    Insert,
    /// `[kept, next]`: finishes a loop that removed items as it went (see `lower::remove`). Items
    /// `[next, len)`, which the loop didn't reach, move down to `kept`, the slots they leave are
    /// zeroed, and the length becomes `kept` plus their count. When `next` is past the length (the
    /// list shrank meanwhile) the length becomes at most `kept`.
    Compact,
}

/// What `ExprKind::Str` does with its string operand (always `args[0]`, a string value). The
/// string may be zero-filled (null pointer, length 0), which is the empty string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrOp {
    /// `[s]`: the number of characters (i64).
    Chars,
    /// `[s, i]`: the character at index `i` (any integer) as a new string in the current arena;
    /// the runtime panics when `i` is out of range.
    At,
    /// `[s, pos]`: the byte length (i64) of the character starting at byte `pos` (i64).
    CharLen,
    /// `[s, pos, n]`: a new string in the current arena of the bytes `pos..pos + n` (i64s).
    Piece,
}
