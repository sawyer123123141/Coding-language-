//! What the checker records about closure literals (see `body::closure` for how they are checked).

use kestrel_syntax::Span;

use crate::sig::SigId;
use crate::tables::{FnId, LocalId};
use crate::ty::Ty;

/// A place outside a closure literal that its body reads: a local or parameter of the function or
/// closures around it, followed through fields as far as the body goes (see `body::captures`).
/// The closure's environment holds a copy of its value, made when the closure is.
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    /// The local the place is rooted at, as written: a loop item, a `match` binding or a
    /// `let ... else` view stays itself (its value is read where the closure is made).
    pub root: LocalId,
    /// The fields followed from `root`: indexes into a struct's fields, or into the fields of the
    /// variant a `match` binding names.
    pub path: Vec<u32>,
    /// The type of the captured value.
    pub ty: Ty,
    /// The variable the place belongs to, with views expanded to the place they read (a loop
    /// item `b` of `for b in w.bricks` is `w.bricks[_]`), and the steps into it: a field's name,
    /// or `None` for an element (any element). For the rules about what a call may change.
    pub source_root: LocalId,
    pub source_path: Vec<Option<String>>,
    /// Where the body first reads it.
    pub span: Span,
}

/// One closure literal, by the `ExprId` of the literal (`Tables::closures`).
#[derive(Clone, Debug)]
pub struct ClosureInfo {
    /// The closure's type: its parameters, result and purity.
    pub sig: SigId,
    /// The outside places the body reads, in the order it first reads them; empty for a closure
    /// with nothing to capture. None is inside another (`p.pos` covers `p.pos.x`).
    pub captures: Vec<Capture>,
    /// The body is pure: no `print`, no random numbers, no call that isn't pure. The type is `pure` too.
    pub is_pure: bool,
    /// The locals the closure owns, parameters first in order, then the ones its body declares.
    /// (A local of a closure belongs to it and not to the function around it, so a field default
    /// or a constant, which have no function, can hold closures.)
    pub locals: Vec<LocalId>,
    /// The body assigns to or passes as `var` one of the closure's own `var` parameters.
    pub mutates_params: bool,
    /// The function the literal is written in; `None` in a field default or a constant.
    pub encl_fn: Option<FnId>,
}
