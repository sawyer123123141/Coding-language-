//! Arrays (`[T]`) and lists (`list[T]`): literals, creation, indexing,
//! `.len`, `.add` and `for ... in`.

use std::collections::HashMap;

use kestrel_syntax::ast::{Arg, Binding, Block, Expr, ExprKind};
use kestrel_syntax::Span;

use super::loop_vars::LoopVar;
use super::remove::Over;
use super::BodyChecker;
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    /// The element type of an array or list type.
    pub(crate) fn elem_of(&self, ty: Ty) -> Option<Ty> {
        match ty {
            Ty::Array(e) | Ty::List(e) => Some(self.t.elem(e)),
            _ => None,
        }
    }

    /// `[a, b, c]`: the element type comes from `want` when that's an array
    /// or list type (so `[]` works there), else from the first element.
    pub(crate) fn check_array_lit(&mut self, e: &Expr, items: &[Expr], want: Option<Ty>) -> Ty {
        if let Some(want @ (Ty::Array(_) | Ty::List(_))) = want {
            let elem = self.elem_of(want).expect("a collection type");
            for item in items {
                self.check_element(item, Some(elem));
            }
            return want;
        }
        let Some((first, rest)) = items.split_first() else {
            self.error_help(e.span, "can't tell what `[]` holds", "give it a type: `let xs: [int] = []`");
            return Ty::Error;
        };
        let elem = self.check_element(first, None);
        for item in rest {
            let want = if elem == Ty::Error { None } else { Some(elem) };
            let found = self.check_expr(item, want);
            // Two types that share an interface: the program probably wants a list of it.
            if let Some(iface) = self.mixed_by_iface(elem, found) {
                self.error(item.span, format!("give the list a type: `var foes: list[{iface}] = [...]`"));
                continue;
            }
            self.finish_element(item, found, want);
        }
        if elem == Ty::Error {
            return Ty::Error;
        }
        Ty::Array(self.t.intern_elem(elem))
    }

    /// A value stored into a collection: a literal element or an `.add` argument.
    pub(crate) fn check_element(&mut self, item: &Expr, want: Option<Ty>) -> Ty {
        let found = self.check_expr(item, want);
        self.finish_element(item, found, want)
    }

    /// The rest of `check_element`, once `item` has been checked and found to be `found`.
    fn finish_element(&mut self, item: &Expr, found: Ty, want: Option<Ty>) -> Ty {
        let ty = self.finish_value(item, found, want);
        if ty == Ty::Unit {
            self.error(item.span, "this has no value to store");
            return Ty::Error;
        }
        self.consume(item);
        ty
    }

    pub(crate) fn check_index(&mut self, e: &Expr) -> Ty {
        let ExprKind::Index { base, index } = &e.kind else { unreachable!("check_index on non-index") };
        let base_ty = self.check_expr(base, None);
        if let Some(kv) = self.map_of(base_ty) {
            return self.check_map_index(index, kv.0, kv.1);
        }
        if base_ty == Ty::Str {
            return self.check_str_index(e, index);
        }
        self.check_int_index(index);
        match self.elem_of(base_ty) {
            Some(elem) => elem,
            None if base_ty == Ty::Error => Ty::Error,
            None => {
                let msg = format!("can't index {}; only arrays, lists, maps and strings can be indexed", self.tn(base_ty));
                self.error(base.span, msg);
                Ty::Error
            }
        }
    }

    /// `array[T](n)` (zero-filled) and `list[T]()` (empty).
    pub(crate) fn check_type_call(&mut self, e: &Expr) -> Ty {
        let ExprKind::TypeCall { name, name_span, type_args, args } = &e.kind else { unreachable!("not a type call") };
        if !matches!(name.as_str(), "array" | "list" | "map") {
            // A user name with its types written (`best[int](xs)`, `Stack[int]()`).
            return self.check_user_type_call(e);
        }
        if name == "map" {
            return self.check_map_new(e, *name_span, type_args, args);
        }
        let elem = match type_args.as_slice() {
            [ty] => self.resolve(ty),
            _ => {
                self.error(*name_span, format!("`{name}[...]` takes one element type"));
                Ty::Error
            }
        };
        if name == "array" {
            match args.as_slice() {
                [a] if a.label.is_none() && !a.is_var => {
                    let ty = self.check_expr(&a.value, None);
                    let ty = self.reject_never(&a.value, ty);
                    if !ty.is_int() && ty != Ty::Error {
                        let msg = format!("an array length must be an integer, found {}", self.tn(ty));
                        self.error(a.value.span, msg);
                    }
                }
                _ => {
                    self.error(e.span, "`array[T](n)` takes one length");
                    self.check_loose_args(args);
                }
            }
        } else if !args.is_empty() {
            self.error_help(e.span, "`list[T]()` takes no arguments", "add items with `.add(...)`");
            self.check_loose_args(args);
        }
        if elem == Ty::Error {
            return Ty::Error;
        }
        // Zero-filling would leave interface values empty, and calling one would crash.
        // (An element holding a placeholder gets the generic message instead of this one.)
        if name == "array" && !self.array_of_param_error(e, elem) && self.t.zero_fill_breaks(elem) {
            let msg = format!("`array[{}](n)` would start with empty items; build it with a literal or `add`", self.tn(elem));
            self.error(e.span, msg);
        }
        let id = self.t.intern_elem(elem);
        if name == "array" {
            Ty::Array(id)
        } else {
            Ty::List(id)
        }
    }

    pub(crate) fn check_loose_args(&mut self, args: &[Arg]) {
        for a in args {
            self.check_expr(&a.value, None);
        }
    }

    /// `xs.len`, the only field arrays and lists have.
    pub(crate) fn collection_field(&mut self, base_ty: Ty, name: &str, span: Span) -> Ty {
        if name == "len" {
            return Ty::Int;
        }
        let msg = format!("`{}` has no field `{name}`", self.tn(base_ty));
        self.error_help(span, msg, "its length is `.len`");
        Ty::Error
    }

    /// `for v in xs` / `for i, v in xs`. `v` aliases `xs[i]`, so it can
    /// change the collection when the collection can change.
    pub(crate) fn check_for_in(&mut self, par: bool, index: Option<&Binding>, var: &Binding, iter: &Expr, body: &Block) {
        let iter_ty = self.check_expr(iter, None);
        let iter_ty = self.reject_never(iter, iter_ty);
        if let Some(kv) = self.map_of(iter_ty) {
            if par {
                self.error(iter.span, "`par for` loops over an array or a list");
            }
            return self.check_map_for_in(iter, kv, index, var, body);
        }
        if iter_ty == Ty::Str {
            return self.check_str_for_in(par, index, var, iter, body);
        }
        let elem = match self.elem_of(iter_ty) {
            Some(elem) => elem,
            None => {
                if iter_ty != Ty::Error {
                    let msg = format!("can't loop over {}; `for ... in` needs an array, list, map or string", self.tn(iter_ty));
                    self.error(iter.span, msg);
                }
                Ty::Error
            }
        };
        let root = self.place_root(iter);
        let changeable = root.is_some_and(|r| self.immutable(r).is_none());
        self.scopes.push(HashMap::new());
        let first_local = self.t.locals.len() as u32;
        if let Some(i) = index {
            let i = self.declare(i, Ty::Int, false, false, false);
            self.note_loop_var(i, LoopVar::Index);
        }
        let v = self.declare(var, elem, changeable, false, false);
        self.note_loop_var(v, LoopVar::Item { stored: root.is_some(), map: false });
        self.t.locals[v.0 as usize].alias = root;
        self.views.insert(v);
        // An item of an inner `par for` is a view for the outer ones too.
        self.note_view(v, iter);
        if par {
            let collection = self.path_key(iter);
            self.par.push(super::par::ParBody::new(v, collection, root, first_local, self.loop_breaks.len() + 1));
        }
        self.enter_list_loop(v, iter);
        let over = match iter_ty {
            Ty::List(_) => Over::List,
            Ty::Array(_) => Over::Array,
            _ => Over::Other,
        };
        self.enter_item_loop(v, over, iter, body);
        let entry = self.moved.clone();
        self.item_loop_body(Some(v), entry.clone(), Some(entry), body);
        self.leave_item_loop();
        // A `break` right after `remove v` leaves it removed; it's out of scope now anyway.
        self.moved.remove(&v);
        self.leave_list_loop(v);
        if par {
            self.end_par();
        }
        self.scopes.pop();
    }
}
