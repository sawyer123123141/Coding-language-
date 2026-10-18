//! `xs.filter(f)`, `xs.map(f)`, `xs.any(f)` and `xs.all(f)`: the list methods that call a function
//! for the items. None of them changes `xs`.
//!
//! `f` is any function value of type `(T) -> bool` (`(T) -> R` for `map`, where `R` is taken from
//! the function): a literal, a named function, a stored closure or a parameter. A literal
//! that is directly the argument gets a stack environment, like one passed to a user function
//! (`Tables::stack_closures`), so the call exclusivity rule applies to it. In a `pure` function
//! or a `par for` body the function must be pure: a literal is checked by its frame, and any
//! other function value must have a `pure` type.

use kestrel_syntax::ast::{Arg, Expr, ExprKind};
use kestrel_syntax::Span;

use super::closure::Impure;
use super::fnvalue::shown_callee;
use super::lists::written;
use super::BodyChecker;
use crate::sig::{Sig, SigParam};
use crate::tables::ListMethod;
use crate::ty::Ty;

/// The method as it is written.
fn word(method: ListMethod) -> &'static str {
    match method {
        ListMethod::Filter => "filter",
        ListMethod::Map => "map",
        ListMethod::Any => "any",
        ListMethod::All => "all",
        other => unreachable!("`{other:?}` takes no function"),
    }
}

impl<'a> BodyChecker<'a> {
    /// The call `e` of `method` (one of the four) on `receiver`, which has type `recv_ty`.
    /// `args` is the one argument, already known to be unlabelled and not `var`.
    pub(super) fn list_fn(&mut self, e: &Expr, receiver: &Expr, recv_ty: Ty, method: ListMethod, name_span: Span, args: &[Arg]) -> Ty {
        let elem = self.elem_of(recv_ty).expect("a collection type");
        let name = word(method);
        let f = &args[0].value;
        // The copy `filter` makes of each kept item can't hold an owner.
        let refused = method == ListMethod::Filter && self.t.is_owning(elem);
        if refused {
            let what = self.t.owns_what(elem).replace('`', "");
            let msg = format!("`filter` would copy the items of `{}`, which own {what}; use a `for` loop with `remove`", written(receiver));
            self.error(name_span, msg);
        }
        let literal = matches!(f.kind, ExprKind::Closure(_));
        // The function takes an item, read-only. `map` leaves the result open: it is the closure's.
        let item = SigParam { ty: elem, is_var: false };
        let result = if method == ListMethod::Map { Ty::Error } else { Ty::Bool };
        let want = Ty::Fn(self.t.intern_sig(Sig { params: vec![item], ret: result, is_pure: false }));
        self.note_stack_closure(&args[0]);
        let found = if method == ListMethod::Map {
            self.infer_result = literal;
            let found = self.check_expr(f, Some(want));
            self.reject_never(f, found)
        } else {
            self.check_value(f, Some(want));
            self.t.ty(f.id)
        };
        let given = match method {
            ListMethod::Map => match self.map_result(f, found, item, elem, literal) {
                Some(r) => Some(r),
                None => return Ty::Error,
            },
            _ => None,
        };
        // What the function does must suit the code around.
        match found {
            Ty::Fn(_) if literal => {
                if !self.t.closures.get(&f.id).is_some_and(|c| c.is_pure) {
                    let shown = format!("the closure passed to `{name}`");
                    self.closure_impure(f.span, Impure::Value(&shown));
                }
            }
            Ty::Fn(sig) => self.check_value_purity(sig, f.span, &shown_callee(f)),
            _ => {}
        }
        // The function runs while `xs` is read, so it can't be given anything the call changes.
        let changes = self.call_changes(args, Some((receiver, false)));
        self.check_call_captures(e.span, args, &changes, None);
        if refused {
            return Ty::Error;
        }
        match method {
            ListMethod::Filter => Ty::List(self.t.intern_elem(elem)),
            ListMethod::Map => Ty::List(self.t.intern_elem(given.expect("map has a result"))),
            _ => Ty::Bool,
        }
    }

    /// The type of the items `map`'s function `f` (of type `found`) gives, or `None` after
    /// reporting why it can't be used.
    fn map_result(&mut self, f: &Expr, found: Ty, item: SigParam, elem: Ty, literal: bool) -> Option<Ty> {
        let Ty::Fn(id) = found else {
            if found != Ty::Error {
                let msg = format!("`map` needs a function that takes `{}` and gives a value, found {}", self.tn(elem), self.tn(found));
                self.error(f.span, msg);
            }
            return None;
        };
        let sig = self.t.sig(id).clone();
        if sig.params != [item] {
            let msg = format!("`map` needs a function that takes `{}`, found `{}`", self.tn(elem), self.t.sig_name(id));
            self.error(f.span, msg);
            return None;
        }
        match sig.ret {
            Ty::Unit => {
                let what = if literal { "closure" } else { "function" };
                self.error(f.span, format!("`map` needs a {what} that gives a value"));
                None
            }
            // Said already.
            Ty::Error => None,
            ret => Some(ret),
        }
    }
}
