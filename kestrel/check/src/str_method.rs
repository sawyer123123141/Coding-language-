//! What a string field, index or method call refers to.

/// What a string field, index or method call refers to (`Res::Str`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StrMethod {
    /// `s.len`: the number of characters.
    Len,
    /// `s[i]`: the character at `i`, as a new string.
    At,
    /// `s.has(sub)`: `bool`.
    Has,
    /// `s.find(sub)`: the character index of the first match, as an `int?`.
    Find,
    /// `s.starts_with(sub)`: `bool`.
    StartsWith,
    /// `s.ends_with(sub)`: `bool`.
    EndsWith,
    /// `s.slice(from, to)`: a new string.
    Slice,
    /// `s.split(sep)`: a new `list[string]`.
    Split,
    /// `s.upper()`: a new string.
    Upper,
    /// `s.lower()`: a new string.
    Lower,
    /// `s.trim()`: a new string.
    Trim,
    /// `s.to_int()`: an `int?`.
    ToInt,
    /// `s.to_float()`: a `float?`.
    ToFloat,
}
