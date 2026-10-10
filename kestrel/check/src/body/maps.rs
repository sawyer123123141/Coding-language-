//! Maps (`map[K, V]`): literals, `map[K, V]()`, `m[k]`, `m[k] = v`,
//! `get`, `has`, `remove` and `for k, v in m`.

use std::collections::HashMap;

use kestrel_syntax::ast::{Arg, AssignOp, Binding, Block, Expr, ExprKind, TypeExpr};
use kestrel_syntax::Span;

use super::{BodyChecker, Expect};
use crate::collect::{map_ty, resolve_type};
use crate::tables::Res;
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    /// The key and value types of a map type.
    pub(crate) fn map_of(&self, ty: Ty) -> Option<(Ty, Ty)> {
        match ty {
            Ty::Map(k, v) => Some((self.t.elem(k), self.t.elem(v))),
            _ => None,
        }
    }

    /// `{k: v, ...}`: the types come from `want` when that's a map type (so
    /// `{}` works there), else from the first entry.
    pub(crate) fn check_map_lit(&mut self, e: &Expr, entries: &[(Expr, Expr)], want: Option<Ty>) -> Ty {
        if let Some((k, v)) = want.and_then(|w| self.map_of(w)) {
            for (key, value) in entries {
                self.check_value(key, Some(k));
                self.check_element(value, Some(v));
            }
            return want.expect("a map type");
        }
        if want == Some(Ty::Error) {
            for (key, value) in entries {
                self.check_expr(key, None);
                self.check_expr(value, None);
            }
            return Ty::Error;
        }
        let Some(((k0, v0), rest)) = entries.split_first() else {
            self.error_help(e.span, "can't tell what `{}` holds", "give it a type: `var m: map[string, int] = {}`");
            return Ty::Error;
        };
        let k = self.check_expr(k0, None);
        let v = self.check_element(v0, None);
        let known = |t: Ty| (t != Ty::Error).then_some(t);
        for (key, value) in rest {
            self.check_value(key, known(k));
            self.check_element(value, known(v));
        }
        map_ty(k, v, k0.span, e.span, self.t, self.diags)
    }

    /// `map[K, V]()`: an empty map.
    pub(crate) fn check_map_new(&mut self, e: &Expr, name_span: Span, type_args: &[TypeExpr], args: &[Arg]) -> Ty {
        if !args.is_empty() {
            self.error_help(e.span, "`map[K, V]()` takes no arguments", "add entries with `m[k] = v`");
            for a in args {
                self.check_expr(&a.value, None);
            }
        }
        let [k, v] = type_args else {
            self.error(name_span, "`map[...]` takes a key type and a value type");
            return Ty::Error;
        };
        let (kt, vt) = (resolve_type(k, self.t, self.diags), resolve_type(v, self.t, self.diags));
        map_ty(kt, vt, k.span, e.span, self.t, self.diags)
    }

    /// `m[k]`: the value, or `none` when the key is missing.
    pub(crate) fn check_map_index(&mut self, index: &Expr, k: Ty, v: Ty) -> Ty {
        self.check_value(index, Some(k));
        Ty::Optional(self.t.intern_elem(v))
    }

    /// `m[k] = v`, where `target` (`m[k]`) is already checked. Adds `k` or
    /// replaces its value; compound assignment can't work on a value that
    /// may be missing.
    pub(crate) fn check_map_set(&mut self, target: &Expr, op: AssignOp, value: &Expr, v: Ty) {
        if op != AssignOp::Set {
            let msg = format!(
                "a map's value may be missing, so `{}` can't change it in place; write `m[k] = m.get(k, 0) + 1`",
                op.symbol()
            );
            self.error(target.span, msg);
            self.check_expr(value, None);
            return;
        }
        self.check_element(value, Some(v));
        self.check_store(target, value);
    }

    /// `m.get(k, default)`, `m.has(k)` and `m.remove(k)`.
    pub(crate) fn map_method(&mut self, e: &Expr, receiver: &Expr, recv_ty: Ty, args: &[Arg]) -> Ty {
        let ExprKind::MethodCall { name, name_span, .. } = &e.kind else { unreachable!("not a method call") };
        let (k, v) = self.map_of(recv_ty).expect("a map type");
        let (arity, usage) = match name.as_str() {
            "get" => (2, "`get` takes a key and a default value: `m.get(k, 0)`"),
            "has" => (1, "`has` takes one key"),
            "remove" => (1, "`remove` takes one key"),
            _ => {
                let msg = format!("`{}` has no method `{name}`", self.tn(recv_ty));
                self.error_help(*name_span, msg, "maps have `get`, `has`, `remove` and `.len`");
                self.check_loose_args(args);
                return Ty::Error;
            }
        };
        if args.len() != arity || args.iter().any(|a| a.label.is_some() || a.is_var) {
            self.error(e.span, usage);
            self.check_loose_args(args);
            return Ty::Error;
        }
        self.check_value(&args[0].value, Some(k));
        match name.as_str() {
            "get" => {
                self.t.res.insert(e.id, Res::MapRead);
                self.check_element(&args[1].value, Some(v));
                v
            }
            "has" => {
                self.t.res.insert(e.id, Res::MapRead);
                Ty::Bool
            }
            _ => {
                self.t.res.insert(e.id, Res::MapRemove);
                self.receiver_root(receiver, name);
                Ty::Bool
            }
        }
    }

    /// `for k, v in m` (`v` stands for the value, and changes it when `m`
    /// can change) or `for k in m`. Keys are copies.
    pub(crate) fn check_map_for_in(&mut self, iter: &Expr, (k, v): (Ty, Ty), key: Option<&Binding>, var: &Binding, body: &Block) {
        let root = self.place_root(iter);
        let changeable = root.is_some_and(|r| self.immutable(r).is_none());
        self.scopes.push(HashMap::new());
        match key {
            Some(key) => {
                self.declare(key, k, false, false, false);
                let value = self.declare(var, v, changeable, false, false);
                self.t.locals[value.0 as usize].alias = root;
                self.views.insert(value);
            }
            None => {
                self.declare(var, k, false, false, false);
            }
        }
        self.loop_breaks.push(false);
        self.check_block(body, Expect::Discard);
        self.loop_breaks.pop();
        self.scopes.pop();
    }
}
