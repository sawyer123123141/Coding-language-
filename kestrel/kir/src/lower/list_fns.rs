//! The loops behind `xs.filter(f)`, `xs.map(f)`, `xs.any(f)` and `xs.all(f)`.
//!
//! Each is a generated function `(xs, f) -> result`, a `ForRange` over `xs` with unchecked
//! indexing (the loop's bound is the list's length, and `f` cannot change `xs`) that calls `f`
//! for each item, read-only, in order:
//! - `filter` copies each kept item into a new list (as an indexed read would);
//! - `map` adds each result, a fresh value, to a new list;
//! - `any` and `all` return at the first item that decides.
//!
//! **A literal gets a loop of its own** (one per literal's code): the loop calls the literal's
//! code directly, `Call(code, [FnEnv(f), item])`, so `inline_small` can inline the closure body
//! into the loop. The call site passes the literal itself as the loop's `f` argument, a
//! `MakeFn` with a stack environment that is directly the `Call`'s argument (see
//! `check_stack_envs`). **Any other function value** goes through a loop per function type, which
//! calls it indirectly. The result is built in the current arena, like any list.

use kestrel_check::{ListMethod, SigId, Tables, Ty};
use kestrel_syntax::ast;

use super::fnvalue::ksig;
use super::helpers::{elem_of, expr, has_heap, with, Helpers};
use super::{kty, FnLowerer};
use crate::ir::*;

/// What a generated loop calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Callee {
    /// A closure literal's code, called with the environment word of the loop's function value.
    Code { code: FuncIdx, pure: bool },
    /// Any function value of this type, called through the loop's function value.
    Value(SigId),
}

fn local(name: &str, ty: KTy, kind: LocalKind) -> Local {
    Local { name: name.into(), ty, kind }
}

impl Helpers {
    /// The loop for `method` over a list or array of type `list_ty`, calling `callee`. `ret` is
    /// the type of its result: the new list for `filter` and `map`, a bool otherwise.
    pub(crate) fn list_fn(&mut self, method: ListMethod, list_ty: KTy, callee: Callee, ret: KTy, t: &Tables) -> FuncIdx {
        let key = (method, list_ty, callee);
        if let Some(&f) = self.list_fns.get(&key) {
            return f;
        }
        let idx = self.reserve();
        self.list_fns.insert(key, idx);
        let elem = elem_of(list_ty, t);
        // Local 0 is the list and local 1 the function value (both passed by pointer), local 2
        // the index, and local 3 the new list when there is one.
        let (xs, f, i, out) = (Place::local(LocalIdx(0)), Place::local(LocalIdx(1)), LocalIdx(2), LocalIdx(3));
        let mut locals = vec![
            local("xs", list_ty, LocalKind::RefParam),
            local("f", KTy::Fn, LocalKind::RefParam),
            local("i", KTy::I64, LocalKind::Value),
        ];
        // The loop never changes the list, so `i < xs.len` holds throughout.
        let at = expr(ExprKind::Load(Place::local(i)), KTy::I64);
        let item_at = with(&xs, Proj::Index { at, checked: false });
        let item = expr(ExprKind::Load(item_at.clone()), elem);
        let call = match callee {
            Callee::Code { code, .. } => {
                let env = expr(ExprKind::FnEnv(f), KTy::I64);
                ExprKind::Call(code, vec![Arg::Value(env), Arg::Value(item)])
            }
            Callee::Value(sig) => {
                let sig = ksig(t, sig);
                ExprKind::CallIndirect { callee: f, args: vec![Arg::Value(item)], sig }
            }
        };
        let result_of = |ty: KTy| expr(call.clone(), ty);
        let truth = |b: bool| Stmt::Return(Some(expr(ExprKind::Bool(b), KTy::Bool)));
        let mut body = Vec::new();
        let per_item = match method {
            ListMethod::Filter => {
                locals.push(local("kept", ret, LocalKind::Addressed));
                body.push(Stmt::Let(out, expr(ExprKind::ListNew, ret)));
                let mut keep = Vec::new();
                let value = if has_heap(elem, t) {
                    // The item is copied byte for byte, then its strings and lists get their own.
                    let copy = LocalIdx(locals.len() as u32);
                    locals.push(local("copy", elem, LocalKind::Addressed));
                    keep.push(Stmt::Let(copy, expr(ExprKind::Load(item_at.clone()), elem)));
                    keep.extend(self.fixup(Place::local(copy), elem, t));
                    expr(ExprKind::Load(Place::local(copy)), elem)
                } else {
                    expr(ExprKind::Load(item_at), elem)
                };
                keep.push(Stmt::ListAdd(Place::local(out), value));
                Stmt::If { cond: result_of(KTy::Bool), then_body: keep, else_body: Vec::new() }
            }
            ListMethod::Map => {
                locals.push(local("mapped", ret, LocalKind::Addressed));
                body.push(Stmt::Let(out, expr(ExprKind::ListNew, ret)));
                Stmt::ListAdd(Place::local(out), result_of(elem_of(ret, t)))
            }
            ListMethod::Any => Stmt::If { cond: result_of(KTy::Bool), then_body: vec![truth(true)], else_body: Vec::new() },
            ListMethod::All => {
                let no = expr(ExprKind::Unary(UnOp::Not, Box::new(result_of(KTy::Bool))), KTy::Bool);
                Stmt::If { cond: no, then_body: vec![truth(false)], else_body: Vec::new() }
            }
            other => unreachable!("`{other:?}` takes no function"),
        };
        body.push(Stmt::ForRange {
            var: i,
            start: expr(ExprKind::Int(0), KTy::I64),
            end: expr(ExprKind::Len(xs), KTy::I64),
            body: vec![per_item],
        });
        body.push(match method {
            ListMethod::Filter | ListMethod::Map => Stmt::Return(Some(expr(ExprKind::Load(Place::local(out)), ret))),
            ListMethod::Any => truth(false),
            _ => truth(true),
        });
        let pure = match callee {
            Callee::Code { pure, .. } => pure,
            Callee::Value(sig) => t.sig(sig).is_pure,
        };
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_list_fn_{}", idx.0),
            params: vec![LocalIdx(0), LocalIdx(1)],
            ret: Some(ret),
            locals,
            body,
            is_pure: pure,
            io: !pure,
            mutates_params: false,
            exported: false,
            native: None,
        };
        idx
    }
}

impl FnLowerer<'_> {
    /// `xs.filter(f)`, `xs.map(f)`, `xs.any(f)` or `xs.all(f)`: a call of the generated loop with
    /// the list and the function value. A literal is passed as it is, with its stack environment
    /// (see the module comment).
    pub(crate) fn list_fn_call(&mut self, method: ListMethod, receiver: &ast::Expr, f: &ast::Expr, ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let list_ty = kty(self.t.ty(receiver.id));
        let list = self.place_or_temp(receiver, &mut pre, true);
        let value = self.expr(f);
        let callee = match (&f.kind, &value.kind) {
            (ast::ExprKind::Closure(_), ExprKind::MakeFn { code, .. }) => Callee::Code { code: *code, pure: self.t.closures[&f.id].is_pure },
            _ => match self.t.ty(f.id) {
                Ty::Fn(sig) => Callee::Value(sig),
                other => unreachable!("the checker gives `{other:?}` to a function argument"),
            },
        };
        let helper = self.helpers.list_fn(method, list_ty, callee, ty, self.t);
        let args = vec![Arg::Value(Expr { kind: ExprKind::Load(list), ty: list_ty, line }), Arg::Value(value)];
        Self::with_pre(pre, ExprKind::Call(helper, args), ty, line)
    }
}
