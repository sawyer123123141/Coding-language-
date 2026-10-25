//! What a `T` can do in generic code (R1), part two: operators, `has`/`find`/`sort`/`sort_by`,
//! zero-filled arrays, and `T` named as a value. Only the rules `T` carries allow anything:
//! `equatable` compares, `ordered` orders and sorts.

use kestrel_syntax::ast::{BinaryOp, Expr};
use kestrel_syntax::Span;

use super::BodyChecker;
use crate::tables::Tables;
use crate::ty::Ty;

/// Whether values of type `ty` have a sort order: numbers, strings and bools, and a placeholder
/// with the rule `ordered`.
pub(super) fn sortable(t: &Tables, ty: Ty) -> bool {
    ty.is_numeric() || matches!(ty, Ty::Bool | Ty::Str) || matches!(ty, Ty::Param(p) if t.generic.param(p).ordered)
}

impl<'a> BodyChecker<'a> {
    /// A comparison with a placeholder on a side. Two values of one placeholder compare when it
    /// has the rule (`equatable` for `==` and `!=`, `ordered` for the rest); a `T` and another type
    /// never compare. `None` when neither side is a placeholder, or for a mixed ordering, which
    /// today's "needs numbers" message already refuses.
    pub(super) fn param_comparison(&mut self, e: &Expr, op: BinaryOp, lt: Ty, rt: Ty) -> Option<Ty> {
        if !matches!(lt, Ty::Param(_)) && !matches!(rt, Ty::Param(_)) {
            return None;
        }
        let equality = matches!(op, BinaryOp::Eq | BinaryOp::Ne);
        if lt == Ty::Error || rt == Ty::Error {
            return Some(Ty::Bool);
        }
        let sym = op.symbol();
        match (lt, rt) {
            (Ty::Param(p), Ty::Param(q)) if p == q => {
                let info = self.t.generic.param(p);
                let (has, rule) = if equality { (info.equatable, "equatable") } else { (info.ordered, "ordered") };
                if !has {
                    let pn = self.t.param_name(p);
                    let msg = format!("`{sym}` on `{pn}` needs the rule `{rule}`: write `[{pn}: {rule}]`");
                    self.error(e.span, msg);
                }
                Some(Ty::Bool)
            }
            _ if equality => {
                let msg = format!("`{sym}` needs two numbers, two bools or two strings, found {} and {}", self.tn(lt), self.tn(rt));
                self.error(e.span, msg);
                Some(Ty::Bool)
            }
            _ => None,
        }
    }

    /// An operator other than a comparison on two values of one placeholder: nothing is known to
    /// work. Returns whether it was one, after reporting it.
    pub(super) fn param_operator(&mut self, e: &Expr, sym: &str, lt: Ty, rt: Ty) -> bool {
        let (Ty::Param(p), Ty::Param(q)) = (lt, rt) else { return false };
        if p != q {
            return false;
        }
        self.param_operator_error(e, sym, p);
        true
    }

    /// A unary operator on a placeholder.
    pub(super) fn param_unary(&mut self, e: &Expr, sym: &str, ty: Ty) -> bool {
        let Ty::Param(p) = ty else { return false };
        self.param_operator_error(e, sym, p);
        true
    }

    fn param_operator_error(&mut self, e: &Expr, sym: &str, p: crate::generic::ParamId) {
        let pn = self.t.param_name(p);
        let msg = format!("`{sym}` doesn't work on `{pn}`, which can be any type; take a function that does it, like `add: ({pn}, {pn}) -> {pn}`");
        self.error(e.span, msg);
    }

    /// `has`, `find` (needing `equatable`), `sort` and a `sort_by` key (needing `ordered`) over
    /// items of type `elem`: when `elem` is a placeholder without the rule, reports it and returns
    /// true. (Other element types are judged as before.)
    pub(super) fn list_param_rule(&mut self, word: &str, elem: Ty, span: Span) -> bool {
        let Ty::Param(p) = elem else { return false };
        let info = self.t.generic.param(p);
        let (has, rule) = match word {
            "has" | "find" => (info.equatable, "equatable"),
            _ => (info.ordered, "ordered"),
        };
        if has {
            return false;
        }
        let pn = self.t.param_name(p);
        let msg = format!("`{word}` needs `{pn}` to be `{rule}`: write `[{pn}: {rule}]`");
        self.error(span, msg);
        true
    }

    /// `array[X](n)` where `X` holds a placeholder: the zero fill of a `T` may be no value of it.
    /// Reports it and returns true (this replaces the plain zero-fill error, never joins it).
    pub(super) fn array_of_param_error(&mut self, e: &Expr, elem: Ty) -> bool {
        if self.t.is_concrete(elem) || !self.t.zero_fill_breaks(elem) {
            return false;
        }
        let x = self.tn(elem);
        let msg = format!("`array[{x}](n)` fills its items with zeros, which a `{x}` may not have; build a `list[{x}]` with `add` instead");
        self.error(e.span, msg);
        true
    }
}
