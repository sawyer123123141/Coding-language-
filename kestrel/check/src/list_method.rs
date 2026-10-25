//! The list methods besides `add`: what each one does to its receiver.

/// The list methods besides `add` (which is `Res::ListAdd`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListMethod {
    RemoveAt,
    Pop,
    Insert,
    Clear,
    Has,
    Find,
    /// `xs.join(sep)` on a list or array of strings.
    Join,
    /// `xs.filter(f)`: a new list of the items `f` is true for.
    Filter,
    /// `xs.map(f)`: a new list of `f`'s results.
    Map,
    /// `xs.any(f)`: whether `f` is true for some item.
    Any,
    /// `xs.all(f)`: whether `f` is true for every item.
    All,
    /// `xs.sort()`: sorts a list or array of numbers, strings or bools in place.
    Sort,
    /// `xs.sort_by(key)`: sorts in place by a pure key function's number, string or bool.
    SortBy,
}

impl ListMethod {
    /// Whether the method changes its receiver; `has`, `find`, `join` and the four that take a
    /// function and give a result only read it.
    pub fn changes_receiver(self) -> bool {
        !matches!(self, ListMethod::Has | ListMethod::Find | ListMethod::Join | ListMethod::Filter | ListMethod::Map | ListMethod::Any | ListMethod::All)
    }

    /// Whether the method takes a function and calls it for the items.
    pub fn takes_fn(self) -> bool {
        matches!(self, ListMethod::Filter | ListMethod::Map | ListMethod::Any | ListMethod::All | ListMethod::SortBy)
    }

    /// Whether the method changes a list's order but not its length, so it works on an array.
    pub fn reorders(self) -> bool {
        matches!(self, ListMethod::Sort | ListMethod::SortBy)
    }
}
