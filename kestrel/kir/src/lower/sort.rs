//! `xs.sort()` and `xs.sort_by(key)`: `Stmt::List(ListOp::Sort, ..)`, which the runtime does with a
//! generated compare function.
//!
//! **The compare function** is `(env, a, b) -> u8`, called by the runtime with the addresses of two
//! items: 0 when `a` goes before `b`, 1 when they are equal, 2 when it goes after. The order is
//! total: numbers by value with a float NaN after every number, strings by code point, `false`
//! before `true`. Both parameters are by pointer (`RefParam`), whatever the item.
//! - `sort()` has one per item type; `env` is unused.
//! - `sort_by(key)` with a string key has one per (item type, key), and its `env` is the address
//!   of the key's function value, so it calls `Call(code, [FnEnv(key), item])` for a literal's code
//!   or a named function's thunk, or calls the value indirectly. It computes both keys inside a
//!   scratch arena that is freed before it returns, since a key may build strings
//!   (`e.name.lower()`) and runs about twice per comparison.
//!
//! **The key function.** `sort_by(key)` with a number or bool key is `ListOp::SortBy` instead: a
//! generated `(env, a) -> i64` (or `-> f64` for a float key) gives the key of one item, widened,
//! and the runtime calls it once per item and sorts by the keys it keeps. It calls the key as the
//! compare function does, in a scratch arena of its own per call.
//!
//! `strip_idle_scratch` (in `scratch`) takes the arena out again when the literal's code, or a
//! named function's thunk (and what it calls), can't build anything.
//!
//! **The call site.** A literal key's `MakeFn` (with a stack environment) must be directly an
//! argument of a call, so `sort_by` calls a generated function `(xs, key)` that holds the
//! `ListOp::Sort` or `ListOp::SortBy`; `key` is passed on to the runtime by address. The key is `pure` (the checker
//! sees to that), so none of this calls into a built-in function that stores a line.

use kestrel_check::{ListMethod, Tables, Ty};
use kestrel_syntax::ast;

use super::fnvalue::ksig;
use super::helpers::{elem_of, expr, Helpers};
use super::list_fns::Callee;
use super::{kty, FnLowerer};
use crate::ir::*;

fn local(name: &str, ty: KTy, kind: LocalKind) -> Local {
    Local { name: name.into(), ty, kind }
}

fn load(p: &Place, ty: KTy) -> Expr {
    expr(ExprKind::Load(p.clone()), ty)
}

fn bin(op: BinOp, a: Expr, b: Expr) -> Expr {
    expr(ExprKind::Binary(op, Box::new(a), Box::new(b)), KTy::Bool)
}

/// Whether `a` goes strictly before `b` in the sort order of `ty`, which is a number, a bool
/// or (compared elsewhere) a string.
fn before(ty: KTy, a: &Place, b: &Place) -> Expr {
    let (x, y) = (load(a, ty), load(b, ty));
    match ty {
        KTy::Bool => expr(ExprKind::And(Box::new(expr(ExprKind::Unary(UnOp::Not, Box::new(x)), KTy::Bool)), Box::new(y)), KTy::Bool),
        // A NaN is after every number, and equal to another NaN.
        KTy::F32 | KTy::F64 => {
            let (not_nan, nan) = (bin(BinOp::Eq, x.clone(), x.clone()), bin(BinOp::Ne, y.clone(), y.clone()));
            let after = expr(ExprKind::And(Box::new(not_nan), Box::new(nan)), KTy::Bool);
            expr(ExprKind::Or(Box::new(bin(BinOp::Lt, x, y)), Box::new(after)), KTy::Bool)
        }
        _ => bin(BinOp::Lt, x, y),
    }
}

/// The statements that put in `r` (a `u8` local) how `a` stands to `b`: both of type `ty`.
fn order(ty: KTy, a: &Place, b: &Place, r: LocalIdx) -> Vec<Stmt> {
    let set = |n: i64| Stmt::Assign(Place::local(r), expr(ExprKind::Int(n), KTy::U8));
    if ty == KTy::Str {
        return vec![Stmt::Let(r, expr(ExprKind::Str(StrOp::Cmp, vec![load(a, ty), load(b, ty)]), KTy::U8))];
    }
    let after = Stmt::If { cond: before(ty, b, a), then_body: vec![set(2)], else_body: Vec::new() };
    vec![
        Stmt::Let(r, expr(ExprKind::Int(1), KTy::U8)),
        Stmt::If { cond: before(ty, a, b), then_body: vec![set(0)], else_body: vec![after] },
    ]
}

impl Helpers {
    /// The compare function for `sort()` on items of type `item`.
    pub(crate) fn sort_items_fn(&mut self, item: KTy) -> FuncIdx {
        if let Some(&f) = self.sort_less.get(&(item, None)) {
            return f;
        }
        let idx = self.reserve();
        self.sort_less.insert((item, None), idx);
        let (a, b, r) = (Place::local(LocalIdx(1)), Place::local(LocalIdx(2)), LocalIdx(3));
        let locals = vec![
            local("env", KTy::I64, LocalKind::Value),
            local("a", item, LocalKind::RefParam),
            local("b", item, LocalKind::RefParam),
            local("r", KTy::U8, LocalKind::Value),
        ];
        let mut body = order(item, &a, &b, r);
        body.push(Stmt::Return(Some(load(&Place::local(r), KTy::U8))));
        self.compare_fn(idx, locals, body);
        idx
    }

    /// The compare function for `sort_by` on items of type `item`, with string keys given by
    /// `callee`.
    fn sort_by_fn(&mut self, item: KTy, key: KTy, callee: Callee, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.sort_less.get(&(item, Some(callee))) {
            return f;
        }
        let idx = self.reserve();
        self.sort_less.insert((item, Some(callee)), idx);
        // Locals: 0 the key function value, 1 and 2 the items, 3 and 4 their keys, 5 the answer.
        let (f, a, b) = (Place::local(LocalIdx(0)), Place::local(LocalIdx(1)), Place::local(LocalIdx(2)));
        let (ka, kb, r) = (LocalIdx(3), LocalIdx(4), LocalIdx(5));
        let key_kind = if key.is_aggregate() { LocalKind::Addressed } else { LocalKind::Value };
        let locals = vec![
            local("key", KTy::Fn, LocalKind::RefParam),
            local("a", item, LocalKind::RefParam),
            local("b", item, LocalKind::RefParam),
            local("ka", key, key_kind),
            local("kb", key, key_kind),
            local("r", KTy::U8, LocalKind::Value),
        ];
        let call = |item: &Place| key_call(callee, &f, load(item, item_ty(&locals, item)), key, t);
        let mut body = vec![Stmt::PushArena(None, 0), Stmt::Let(ka, call(&a)), Stmt::Let(kb, call(&b))];
        body.extend(order(key, &Place::local(ka), &Place::local(kb), r));
        body.push(Stmt::PopArena { free: true });
        body.push(Stmt::Return(Some(load(&Place::local(r), KTy::U8))));
        self.compare_fn(idx, locals, body);
        if let Callee::Code { code, .. } = callee {
            self.sort_idle.push((idx, code));
        }
        idx
    }

    /// The key function for `sort_by` on items of type `item`, with number or bool keys of type
    /// `key` given by `callee`: `(env, a) -> i64` or `-> f64`, the key of the item at `a` widened
    /// (see `ListOp::SortBy`).
    fn sort_key_fn(&mut self, item: KTy, key: KTy, callee: Callee, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.sort_less.get(&(item, Some(callee))) {
            return f;
        }
        let idx = self.reserve();
        self.sort_less.insert((item, Some(callee)), idx);
        // Locals: 0 the key function value, 1 the item, 2 its key.
        let (f, a, k) = (Place::local(LocalIdx(0)), Place::local(LocalIdx(1)), LocalIdx(2));
        let wide = wide_key(key);
        let locals = vec![local("key", KTy::Fn, LocalKind::RefParam), local("a", item, LocalKind::RefParam), local("k", key, LocalKind::Value)];
        let k_now = load(&Place::local(k), key);
        // `false` is 0 and `true` 1; an integer or float widens (exactly).
        let widened = match key {
            KTy::Bool => expr(
                ExprKind::If {
                    cond: Box::new(k_now),
                    then_body: Vec::new(),
                    then_val: Some(Box::new(expr(ExprKind::Int(1), KTy::I64))),
                    else_body: Vec::new(),
                    else_val: Some(Box::new(expr(ExprKind::Int(0), KTy::I64))),
                },
                KTy::I64,
            ),
            _ if key == wide => k_now,
            _ => expr(ExprKind::Cast(Box::new(k_now)), wide),
        };
        let body = vec![
            Stmt::PushArena(None, 0),
            Stmt::Let(k, key_call(callee, &f, load(&a, item), key, t)),
            Stmt::PopArena { free: true },
            Stmt::Return(Some(widened)),
        ];
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_sort_key_{}", idx.0),
            params: vec![LocalIdx(0), LocalIdx(1)],
            ret: Some(wide),
            locals,
            body,
            is_pure: true,
            io: false,
            mutates_params: false,
            exported: false,
            lines: (0, 0),
            native: None,
        };
        if let Callee::Code { code, .. } = callee {
            self.sort_idle.push((idx, code));
        }
        idx
    }

    /// Fills in the reserved compare function `idx`: `(env, a, b) -> u8`, the first three locals.
    fn compare_fn(&mut self, idx: FuncIdx, locals: Vec<Local>, body: Vec<Stmt>) {
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_compare_{}", idx.0),
            params: vec![LocalIdx(0), LocalIdx(1), LocalIdx(2)],
            ret: Some(KTy::U8),
            locals,
            body,
            is_pure: true,
            io: false,
            mutates_params: false,
            exported: false,
            lines: (0, 0),
            native: None,
        };
    }

    /// The function `sort_by(var xs, key)` calls for a list or array of type `list_ty`: it sorts
    /// `xs` with the compare function (a string key) or the key function (any other) for `callee`.
    fn sort_by_call_fn(&mut self, list_ty: KTy, key: KTy, callee: Callee, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.sort_calls.get(&(list_ty, callee)) {
            return f;
        }
        let idx = self.reserve();
        self.sort_calls.insert((list_ty, callee), idx);
        let item = elem_of(list_ty, t);
        let (op, f) = if key == KTy::Str {
            (ListOp::Sort, self.sort_by_fn(item, key, callee, t))
        } else {
            let kind = if key.is_float() { SortKey::Float } else { SortKey::Int };
            (ListOp::SortBy(kind), self.sort_key_fn(item, key, callee, t))
        };
        let xs = Place::local(LocalIdx(0));
        let sort = Stmt::List(op, xs, vec![expr(ExprKind::FnRef(f), KTy::Fn), load(&Place::local(LocalIdx(1)), KTy::Fn)]);
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_sort_by_{}", idx.0),
            params: vec![LocalIdx(0), LocalIdx(1)],
            ret: None,
            locals: vec![local("xs", list_ty, LocalKind::RefParam), local("key", KTy::Fn, LocalKind::RefParam)],
            body: vec![sort],
            is_pure: true,
            io: false,
            mutates_params: true,
            exported: false,
            lines: (0, 0),
            native: None,
        };
        idx
    }
}

/// The type of the local `p` names (a plain local).
fn item_ty(locals: &[Local], p: &Place) -> KTy {
    locals[p.root.0 as usize].ty
}

/// The key of `item` (a value of the item type), a `key`, given by `callee` through the function
/// value at `f`: the literal's code or the thunk directly, any other value indirectly.
fn key_call(callee: Callee, f: &Place, item: Expr, key: KTy, t: &Tables) -> Expr {
    let arg = Arg::Value(item);
    match callee {
        Callee::Code { code, .. } => expr(ExprKind::Call(code, vec![Arg::Value(expr(ExprKind::FnEnv(f.clone()), KTy::I64)), arg]), key),
        Callee::Value(sig) => expr(ExprKind::CallIndirect { callee: f.clone(), args: vec![arg], sig: ksig(t, sig) }, key),
    }
}

/// The type a number or bool key of type `key` is widened to for the runtime.
fn wide_key(key: KTy) -> KTy {
    if key.is_float() {
        KTy::F64
    } else {
        KTy::I64
    }
}

impl FnLowerer<'_> {
    /// `xs.sort()` or `xs.sort_by(key)`, as a statement.
    pub(crate) fn sort_stmt(&mut self, method: ListMethod, receiver: &ast::Expr, args: &[ast::Arg], out: &mut Vec<Stmt>) {
        let list_ty = kty(self.t.ty(receiver.id));
        let list = self.place_or_temp(receiver, out, true);
        if method == ListMethod::Sort {
            let compare = self.helpers.sort_items_fn(elem_of(list_ty, self.t));
            // (The reference carries the line of the call, for the report.)
            let compare = Expr { kind: ExprKind::FnRef(compare), ty: KTy::Fn, line: self.line(receiver.span) };
            out.push(Stmt::List(ListOp::Sort, list, vec![compare]));
            return;
        }
        let f = &args[0].value;
        let value = self.expr(f);
        let Ty::Fn(sig) = self.t.ty(f.id) else { unreachable!("the checker gives a function to `sort_by`") };
        let callee = match (&f.kind, &value.kind) {
            (ast::ExprKind::Closure(_), ExprKind::MakeFn { code, .. }) => Callee::Code { code: *code, pure: true },
            // A named function's thunk has the shape of a closure's code (it ignores the environment).
            (_, ExprKind::FnRef(thunk)) => Callee::Code { code: *thunk, pure: true },
            _ => Callee::Value(sig),
        };
        let key = kty(self.t.sig(sig).ret);
        let helper = self.helpers.sort_by_call_fn(list_ty, key, callee, self.t);
        // The literal is directly the call's argument (see `check_stack_envs`).
        out.push(Stmt::Call(helper, vec![Arg::Ref(list), Arg::Value(value)], self.line(receiver.span)));
    }
}
