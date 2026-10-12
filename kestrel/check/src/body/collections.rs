//! Arrays (`[T]`) and lists (`list[T]`): literals, creation, indexing,
//! `.len`, `.add` and `for ... in`.

use std::collections::HashMap;

use kestrel_syntax::ast::{Arg, Binding, Block, Expr, ExprKind, TypeExpr};
use kestrel_syntax::Span;

use super::{BodyChecker, Expect};
use crate::collect::resolve_type;
use crate::tables::Res;
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
            self.check_element(item, if elem == Ty::Error { None } else { Some(elem) });
        }
        if elem == Ty::Error {
            return Ty::Error;
        }
        Ty::Array(self.t.intern_elem(elem))
    }

    /// A value stored into a collection: a literal element or an `.add` argument.
    pub(crate) fn check_element(&mut self, item: &Expr, want: Option<Ty>) -> Ty {
        let ty = self.check_value(item, want);
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
        let index_ty = self.check_expr(index, None);
        let index_ty = self.reject_never(index, index_ty);
        if !index_ty.is_int() && index_ty != Ty::Error {
            let msg = format!("an index must be an integer, found {}", self.tn(index_ty));
            self.error(index.span, msg);
        }
        match self.elem_of(base_ty) {
            Some(elem) => elem,
            None if base_ty == Ty::Error => Ty::Error,
            None => {
                let msg = format!("can't index {}; only arrays, lists and maps can be indexed", self.tn(base_ty));
                self.error(base.span, msg);
                Ty::Error
            }
        }
    }

    /// `array[T](n)` (zero-filled) and `list[T]()` (empty).
    pub(crate) fn check_type_call(&mut self, e: &Expr) -> Ty {
        let ExprKind::TypeCall { name, name_span, type_args, args } = &e.kind else { unreachable!("not a type call") };
        if name == "map" {
            return self.check_map_new(e, *name_span, type_args, args);
        }
        let elem = match type_args.as_slice() {
            [ty] => resolve_type(ty, self.t, self.diags),
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

    /// `xs.add(v)`: lists only, and `xs` must be changeable.
    pub(crate) fn collection_method(&mut self, e: &Expr, receiver: &Expr, recv_ty: Ty, args: &[Arg]) -> Ty {
        let ExprKind::MethodCall { name, name_span, .. } = &e.kind else { unreachable!("not a method call") };
        let elem = self.elem_of(recv_ty).expect("a collection type");
        let problem = match (recv_ty, name.as_str()) {
            (Ty::List(_), "add") => None,
            (Ty::Array(_), "add") => {
                Some(format!("arrays have a fixed length; use a `list[{}]` to add items", self.tn(elem)))
            }
            _ => Some(format!("`{}` has no method `{name}`", self.tn(recv_ty))),
        };
        if let Some(msg) = problem {
            self.error(*name_span, msg);
            self.check_loose_args(args);
            return Ty::Error;
        }
        self.t.res.insert(e.id, Res::ListAdd);
        self.receiver_root(receiver, name);
        match args {
            [a] if a.label.is_none() && !a.is_var => {
                self.check_element(&a.value, Some(elem));
                self.check_store(receiver, &a.value);
            }
            _ => {
                self.error(e.span, "`add` takes one value");
                self.check_loose_args(args);
            }
        }
        Ty::Unit
    }

    /// `let x = opt else { ... }`: `x` is the value inside the optional;
    /// the `else` block runs when it's `none`, and must leave.
    pub(crate) fn check_let_else(&mut self, is_var: bool, bind: &Binding, ty: Option<&TypeExpr>, value: &Expr, otherwise: &Block) {
        let ann = ty.map(|t| resolve_type(t, self.t, self.diags));
        let want = ann.map(|t| if t == Ty::Error { t } else { Ty::Optional(self.t.intern_elem(t)) });
        let found = self.check_value(value, want);
        let inner = match found {
            Ty::Optional(e) => self.t.elem(e),
            Ty::Error => Ty::Error,
            other => {
                let msg = format!("`let ... else` unwraps an optional (`T?`), but this is {}", self.tn(other));
                self.error(value.span, msg);
                Ty::Error
            }
        };
        // The `else` block runs when `value` is `none`: nothing has been unwrapped or moved yet.
        if self.check_block(otherwise, Expect::Discard) != Ty::Never {
            self.error_help(otherwise.span, "the `else` block has to leave", "end it with `return`, `break` or `continue`");
        }
        self.consume(value);
        self.declare(bind, inner, is_var, false, false);
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
        let elem = match self.elem_of(iter_ty) {
            Some(elem) => elem,
            None => {
                if iter_ty != Ty::Error {
                    let msg = format!("can't loop over {}; `for ... in` needs an array, list or map", self.tn(iter_ty));
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
            self.declare(i, Ty::Int, false, false, false);
        }
        let v = self.declare(var, elem, changeable, false, false);
        self.t.locals[v.0 as usize].alias = root;
        self.views.insert(v);
        // An item of an inner `par for` is a view for the outer ones too.
        self.note_view(v, iter);
        if par {
            let collection = self.path_key(iter);
            self.par.push(super::par::ParBody::new(v, collection, root, first_local, self.loop_breaks.len() + 1));
        }
        let entry = self.moved.clone();
        self.loop_body(entry, body, true);
        if par {
            self.end_par();
        }
        self.scopes.pop();
    }
}
