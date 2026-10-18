//! What the checker records about closure literals (see `body::closure` for how they are checked).

use crate::sig::SigId;
use crate::tables::{FnId, LocalId};

/// A local or parameter of a function or closure around a closure literal that its body names.
/// The checker doesn't find captures yet: a literal that names one is refused.
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    /// The local the captured place is rooted at.
    pub local: LocalId,
}

/// One closure literal, by the `ExprId` of the literal (`Tables::closures`).
#[derive(Clone, Debug)]
pub struct ClosureInfo {
    /// The closure's type: its parameters, result and purity.
    pub sig: SigId,
    /// The outside places the body names; empty for a closure with nothing to capture.
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
