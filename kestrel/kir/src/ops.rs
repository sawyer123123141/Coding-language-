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
    /// `[compare]` or `[compare, key]` (`sort`, and `sort_by` with a string key): sorts the items
    /// stably, smallest first. `compare` is a
    /// `FnRef` to a generated function `(env, a, b) -> u8` that says how the item at the pointer
    /// `a` stands to the one at `b`: 0 for before, 1 for equal, 2 for after (a total order). Its
    /// environment word is the address of a copy of `key`, a function value, when there is one, else
    /// 0. The items move as bytes.
    Sort,
    /// `[key_fn, key]`: sorts the items stably, smallest first, by a number or bool key, as `Sort`
    /// does. `key_fn` is a `FnRef` to a generated function `(env, a) -> i64` (`SortKey::Int`) or
    /// `-> f64` (`SortKey::Float`) that gives the key of the item at the pointer `a`, widened; the
    /// runtime calls it once per item, in order. Its environment word is as for `Sort`.
    SortBy(SortKey),
}

/// The widened type of a `ListOp::SortBy` key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SortKey {
    /// An integer or bool key, as an `i64`.
    Int,
    /// A float key, as an `f64`: by value, with a NaN after every number.
    Float,
}

/// What `ExprKind::Str` does with its string operand (`args[0]`, a string value; for `Join` a list
/// or array of strings). A string may be zero-filled (null pointer, length 0), which is the empty
/// string. Characters are Unicode code points; every string made is new, in the current arena.
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
    /// `[s, sub]`: the character index (i64) of the first `sub` in `s`, or -1. An empty `sub` is 0.
    Find,
    /// `[s, sub]`: whether `s` starts with `sub` (bool).
    Starts,
    /// `[a, b]`: how `a` stands to `b` by code point (the order of their UTF-8 bytes): a u8, 0 for
    /// before, 1 for equal, 2 for after.
    Cmp,
    /// `[s, sub]`: whether `s` ends with `sub` (bool).
    Ends,
    /// `[s, from, to]`: a new string of the characters `from..to` (any integers); the runtime
    /// panics unless `0 <= from <= to <= chars`.
    Slice,
    /// `[s]`: a new string without whitespace at either end.
    Trim,
    /// `[s]`: a new string in upper case.
    Upper,
    /// `[s]`: a new string in lower case.
    Lower,
    /// `[list, sep]`: a new string of the list's (or array's) strings with `sep` between them.
    Join,
    /// `[s, sep]`: a new `list[string]` of the pieces between the separators, all in the current
    /// arena; the runtime panics when `sep` is empty.
    Split,
    /// `[s]`: an `int?`, `none` unless `s` is a whole number that fits.
    ToInt,
    /// `[s]`: a `float?`, `none` unless `s` is a finite number.
    ToFloat,
}
