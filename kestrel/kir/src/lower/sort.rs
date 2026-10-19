//! `xs.sort()` and `xs.sort_by(key)`: `Stmt::List(ListOp::Sort, ..)`, which the runtime does with a
//! generated compare function.
//!
//! **The compare function** is `(env, a, b) -> u8`, called by the runtime with the addresses of two
//! items: 0 when `a` goes before `b`, 1 when they are equal, 2 when it goes after. The order is
//! total: numbers by value with a float NaN after every number, strings by code point, `false`
//! before `true`. Both parameters are by pointer (`RefParam`), whatever the item.
//! - `sort()` has one per item type; `env` is unused.
//! - `sort_by(key)` has one per (item type, key), and its `env` is the address of the key's
//!   function value, so it calls `Call(code, [FnEnv(key), item])` for a literal's code or a named
//!   function's thunk, or calls the value indirectly. It computes both keys inside a scratch arena
//!   that is freed before it returns, since a key may build strings (`e.name.lower()`) and runs
//!   about twice per comparison;
//!   `strip_idle_scratch` takes the arena out again when the literal's code, or a named function's
//!   thunk (and what it calls), can't build anything.
//!
//! **The call site.** A literal key's `MakeFn` (with a stack environment) must be directly an
//! argument of a call, so `sort_by` calls a generated function `(xs, key)` that holds the
//! `ListOp::Sort`; `key` is passed on to the runtime by address. The key is `pure` (the checker
//! sees to that), so none of this calls into a built-in function that stores a line.

use std::collections::HashSet;

use kestrel_check::{ListMethod, Tables, Ty};
use kestrel_syntax::ast;

use super::fnvalue::ksig;
use super::helpers::{elem_of, expr, Helpers};
use super::list_fns::Callee;
use super::{kty, FnLowerer};
use crate::ir::*;
use crate::visit::{walk, Node};

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

    /// The compare function for `sort_by` on items of type `item`, with keys of type `key` given by
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
        let call = |item: &Place| {
            let arg = Arg::Value(load(item, item_ty(&locals, item)));
            match callee {
                Callee::Code { code, .. } => expr(ExprKind::Call(code, vec![Arg::Value(expr(ExprKind::FnEnv(f.clone()), KTy::I64)), arg]), key),
                Callee::Value(sig) => expr(ExprKind::CallIndirect { callee: f.clone(), args: vec![arg], sig: ksig(t, sig) }, key),
            }
        };
        let mut body = vec![Stmt::PushArena(None), Stmt::Let(ka, call(&a)), Stmt::Let(kb, call(&b))];
        body.extend(order(key, &Place::local(ka), &Place::local(kb), r));
        body.push(Stmt::PopArena { free: true });
        body.push(Stmt::Return(Some(load(&Place::local(r), KTy::U8))));
        self.compare_fn(idx, locals, body);
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
            native: None,
        };
    }

    /// The function `sort_by(var xs, key)` calls for a list or array of type `list_ty`: it sorts
    /// `xs` with the compare function for `callee`.
    fn sort_by_call_fn(&mut self, list_ty: KTy, key: KTy, callee: Callee, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.sort_calls.get(&(list_ty, callee)) {
            return f;
        }
        let idx = self.reserve();
        self.sort_calls.insert((list_ty, callee), idx);
        let compare = self.sort_by_fn(elem_of(list_ty, t), key, callee, t);
        let xs = Place::local(LocalIdx(0));
        let sort = Stmt::List(ListOp::Sort, xs, vec![expr(ExprKind::FnRef(compare), KTy::Fn), load(&Place::local(LocalIdx(1)), KTy::Fn)]);
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
            native: None,
        };
        idx
    }
}

/// The type of the local `p` names (a plain local).
fn item_ty(locals: &[Local], p: &Place) -> KTy {
    locals[p.root.0 as usize].ty
}

impl FnLowerer<'_> {
    /// `xs.sort()` or `xs.sort_by(key)`, as a statement.
    pub(crate) fn sort_stmt(&mut self, method: ListMethod, receiver: &ast::Expr, args: &[ast::Arg], out: &mut Vec<Stmt>) {
        let list_ty = kty(self.t.ty(receiver.id));
        let list = self.place_or_temp(receiver, out, true);
        if method == ListMethod::Sort {
            let compare = self.helpers.sort_items_fn(elem_of(list_ty, self.t));
            out.push(Stmt::List(ListOp::Sort, list, vec![expr(ExprKind::FnRef(compare), KTy::Fn)]));
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
        out.push(Stmt::Call(helper, vec![Arg::Ref(list), Arg::Value(value)]));
    }
}

/// Takes the scratch arena out of each `sort_by` compare function whose key, a literal or a named
/// function, can't build anything: `idle` pairs a compare function with the code it calls (the
/// literal's code, or the function's thunk).
pub(super) fn strip_idle_scratch(funcs: &mut [Func], idle: &[(FuncIdx, FuncIdx)]) {
    for &(compare, code) in idle {
        if !builds_memory(funcs, code, &mut HashSet::new()) {
            funcs[compare.0 as usize].body.retain(|s| !matches!(s, Stmt::PushArena(None) | Stmt::PopArena { .. }));
        }
    }
}

/// Whether running function `f` may build memory in the current arena: it makes a string, array,
/// list entry, map entry, copy or environment, or calls something that does (or something unknown,
/// like a function value or a built-in function).
fn builds_memory(funcs: &[Func], f: FuncIdx, seen: &mut HashSet<u32>) -> bool {
    if !seen.insert(f.0) {
        return false;
    }
    let func = &funcs[f.0 as usize];
    if func.native.is_some() {
        return true;
    }
    let mut calls = Vec::new();
    let mut builds = false;
    walk(&func.body, &mut |n| match n {
        Node::Stmt(s) => builds |= stmt_builds(s, &mut calls),
        Node::Expr(e) => builds |= expr_builds(e, &mut calls),
        Node::Place(..) => {}
    });
    builds || calls.into_iter().any(|g| builds_memory(funcs, g, seen))
}

/// Whether the statement itself builds memory in the current arena (the statements and
/// expressions inside it are walked on their own). A direct call goes into `calls`. Every kind is
/// named, so a new one has to be sorted here.
fn stmt_builds(s: &Stmt, calls: &mut Vec<FuncIdx>) -> bool {
    match s {
        Stmt::Dup(_) | Stmt::ListAdd(..) | Stmt::MapSet(..) | Stmt::CopyFnEnv(_) | Stmt::ParFor { .. } | Stmt::CallIndirect { .. } => true,
        Stmt::List(op, ..) => match op {
            ListOp::Insert => true,
            ListOp::RemoveAt | ListOp::Clear | ListOp::Compact | ListOp::Sort => false,
        },
        Stmt::Call(g, _) => {
            calls.push(*g);
            false
        }
        Stmt::Let(..)
        | Stmt::Assign(..)
        | Stmt::If { .. }
        | Stmt::While { .. }
        | Stmt::ForRange { .. }
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Return(_)
        | Stmt::Print(_)
        | Stmt::Write(_)
        | Stmt::RandSeed(_)
        | Stmt::SetLen(..)
        | Stmt::Bind(..)
        | Stmt::BindGlobal(..)
        | Stmt::BindEnv { .. }
        | Stmt::PushArena(_)
        | Stmt::PopArena { .. }
        | Stmt::FreeArena(_)
        | Stmt::DropGpu(_)
        | Stmt::MapUnlock(..)
        | Stmt::SetLine(_)
        | Stmt::Trap => false,
    }
}

/// Whether the expression itself builds memory in the current arena, as `stmt_builds` does for a
/// statement.
fn expr_builds(e: &Expr, calls: &mut Vec<FuncIdx>) -> bool {
    match &e.kind {
        ExprKind::Format(_) | ExprKind::ArrayNew(_) | ExprKind::ArrayLit(_) | ExprKind::NewArena | ExprKind::StaticArena | ExprKind::EnvDup(_) | ExprKind::CallIndirect { .. } => {
            true
        }
        ExprKind::MakeFn { env, .. } => match env {
            EnvKind::Heap(_) => true,
            EnvKind::None | EnvKind::Stack(_) => false,
        },
        ExprKind::Str(op, _) => match op {
            StrOp::At | StrOp::Piece | StrOp::Slice | StrOp::Trim | StrOp::Upper | StrOp::Lower | StrOp::Join | StrOp::Split => true,
            StrOp::Chars | StrOp::CharLen | StrOp::Find | StrOp::Starts | StrOp::Cmp | StrOp::Ends | StrOp::ToInt | StrOp::ToFloat => false,
        },
        ExprKind::Map(op, ..) => match op {
            MapOp::Get | MapOp::Has | MapOp::Remove | MapOp::Used | MapOp::Live | MapOp::Lock => false,
        },
        ExprKind::Call(g, _) => {
            calls.push(*g);
            false
        }
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::StrLit(_)
        | ExprKind::ListNew
        | ExprKind::MapNew
        | ExprKind::Len(_)
        | ExprKind::Some(_)
        | ExprKind::NoneLit
        | ExprKind::Move(_)
        | ExprKind::Load(_)
        | ExprKind::Unary(..)
        | ExprKind::Binary(..)
        | ExprKind::And(..)
        | ExprKind::Or(..)
        | ExprKind::Cast(_)
        | ExprKind::Math(..)
        | ExprKind::RandFloat
        | ExprKind::RandInt(..)
        | ExprKind::FnRef(_)
        | ExprKind::FnEnv(_)
        | ExprKind::StructLit(..)
        | ExprKind::VariantLit(..)
        | ExprKind::Tag(_)
        | ExprKind::FieldOf(..)
        | ExprKind::Seq(..)
        | ExprKind::If { .. } => false,
    }
}
