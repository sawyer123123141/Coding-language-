//! Function values: a named function used as a value, and calls through a value.
//!
//! A function value is `{code, env}` (see `KTy::Fn`). The code takes the environment first, then
//! the parameters. A named function has no environment, so its value points at a generated thunk
//! with that shape, which ignores the environment and calls the function.

use kestrel_check::{ConstId, FnId, LocalId, Res, SigId, Tables, Ty};
use kestrel_syntax::ast;

use super::consts::hoisted;
use super::helpers::{expr, Helpers};
use super::{kty, FnLowerer};
use crate::ir::*;

/// The signature of a call through a value of the checker's function type `sig`.
pub(super) fn ksig(t: &Tables, sig: SigId) -> KSig {
    let sig = t.sig(sig);
    let params = sig
        .params
        .iter()
        .map(|p| {
            let ty = kty(p.ty);
            (ty, p.is_var || ty.is_aggregate())
        })
        .collect();
    KSig { params, ret: (sig.ret != Ty::Unit).then(|| kty(sig.ret)) }
}

impl Helpers {
    /// The thunk of function `f`, made at the first use: `(env: i64, params...) -> ret`, which
    /// calls `f` with the parameters and ignores the environment. One per function.
    pub(crate) fn thunk_for(&mut self, f: FuncIdx, t: &Tables) -> FuncIdx {
        if let Some(&thunk) = self.thunks.get(&f) {
            return thunk;
        }
        let idx = self.reserve();
        self.thunks.insert(f, idx);
        let info = &t.fns[f.0 as usize];
        let mut locals = vec![Local { name: "env".into(), ty: KTy::I64, kind: LocalKind::Value }];
        let mut args = Vec::with_capacity(info.params.len());
        for p in &info.params {
            let ty = kty(p.ty);
            let local = LocalIdx(locals.len() as u32);
            // The same rule as the function's own parameters (see `FnLowerer::lower`).
            let by_ref = p.is_var || ty.is_aggregate();
            locals.push(Local { name: p.name.clone(), ty, kind: if by_ref { LocalKind::RefParam } else { LocalKind::Value } });
            let at = Place::local(local);
            args.push(if p.is_var { Arg::Ref(at) } else { Arg::Value(expr(ExprKind::Load(at), ty)) });
        }
        let params = (0..locals.len() as u32).map(LocalIdx).collect();
        let ret = (info.ret != Ty::Unit).then(|| kty(info.ret));
        let body = match ret {
            Some(r) => vec![Stmt::Return(Some(expr(ExprKind::Call(f, args), r)))],
            None => vec![Stmt::Call(f, args)],
        };
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_fn_{}", idx.0),
            params,
            ret,
            locals,
            body,
            is_pure: info.is_pure,
            io: info.io,
            mutates_params: info.mutates_params,
            exported: false,
            native: None,
        };
        idx
    }
}

impl FnLowerer<'_> {
    /// The value of function `f` named as a value: its thunk, with no environment.
    pub(crate) fn fn_ref(&mut self, f: FnId) -> ExprKind {
        ExprKind::FnRef(self.helpers.thunk_for(FuncIdx(f.0), self.t))
    }

    /// The place of a local, which is its loop element's place when it names one.
    fn local_place(&self, l: LocalId) -> Place {
        match self.elems.get(&l) {
            Some(p) => p.clone(),
            None => Place::local(self.local_map[&l]),
        }
    }

    /// The place holding the function value that call `e` calls (a local by name, a constant by
    /// name, a field of function type, or any expression). A value that isn't a place is put in
    /// a temporary by the statements added to `pre`.
    fn callee_place(&mut self, e: &ast::Expr, pre: &mut Vec<Stmt>) -> Place {
        // A captured function value, or a function field of a capture (`b.on(1)` in a closure).
        if let Some(p) = self.capture_place(e.id) {
            return p;
        }
        let t = self.t;
        match &e.kind {
            ast::ExprKind::Call { .. } => match (t.call_local.get(&e.id), t.call_const.get(&e.id)) {
                (Some(&l), _) => self.local_place(l),
                (None, Some(&c)) => self.const_callee(c, pre),
                (None, None) => unreachable!("a call by name through a value names a local or a constant"),
            },
            ast::ExprKind::MethodCall { receiver, .. } => {
                let field = Proj::Field(t.field_index[&e.id]);
                if let Some(mut p) = self.place(receiver) {
                    p.path.push(field);
                    return p;
                }
                // `(if c { w } else { v }).cb(var w)`: the receiver is held as it is, and the
                // function value is copied out of it (see `own_callee`).
                let mut p = self.place_or_temp(receiver, pre, false);
                p.path.push(field);
                let ty = KTy::Fn;
                let v = Expr { kind: ExprKind::Load(p), ty, line: self.line(e.span) };
                self.own_callee(v, pre)
            }
            ast::ExprKind::CallValue { callee, .. } => match self.place(callee) {
                Some(p) => p,
                None => {
                    let v = self.expr(callee);
                    self.own_callee(v, pre)
                }
            },
            _ => unreachable!("not a call through a function value"),
        }
    }

    /// The callee `v`, read from something that isn't a place (an `if` or a `match` that yields
    /// `w.cb`), as a temporary with its own copy of its environment in the current arena. The
    /// checker's rule about the place a called value is read from (`exclusive`) only sees
    /// places, so a shallow temporary could share an environment that the call frees (by
    /// replacing `w`). A fresh value (a call's result) is the call's own already and isn't copied.
    fn own_callee(&mut self, v: Expr, pre: &mut Vec<Stmt>) -> Place {
        let v = self.owned(v, false);
        self.temp_place(v, pre)
    }

    /// A constant of function type, called by name: the global of a hoisted one, or its
    /// initializer (`square`) written out in a temporary.
    fn const_callee(&mut self, c: ConstId, pre: &mut Vec<Stmt>) -> Place {
        if hoisted(self.t, c) {
            return Place::local(self.const_local(c));
        }
        let v = self.const_value(c);
        self.temp_place(v, pre)
    }

    /// The parts of a call through a function value: the statements to run first, the callee,
    /// the arguments (positional; `var` ones by place) and the signature.
    fn call_value_parts(&mut self, e: &ast::Expr) -> (Vec<Stmt>, Place, Vec<Arg>, KSig) {
        let Some(&Res::CallValue(sig)) = self.t.res.get(&e.id) else { unreachable!("not a call through a function value") };
        let (ast::ExprKind::Call { args, .. } | ast::ExprKind::MethodCall { args, .. } | ast::ExprKind::CallValue { args, .. }) = &e.kind
        else {
            unreachable!("not a call through a function value")
        };
        let mut pre = Vec::new();
        let callee = self.callee_place(e, &mut pre);
        let params = &self.t.sig(sig).params;
        let lowered = args
            .iter()
            .zip(params)
            .map(|(a, p)| {
                if p.is_var {
                    let place = self.place(&a.value).expect("checker requires a place for a `var` argument");
                    self.var_arg(&a.value, place)
                } else {
                    let v = self.expr(&a.value);
                    Arg::Value(self.held(v))
                }
            })
            .collect();
        (pre, callee, lowered, ksig(self.t, sig))
    }

    /// A call through a function value, as a value of type `ty`.
    pub(crate) fn call_value(&mut self, e: &ast::Expr, ty: KTy, line: u32) -> ExprKind {
        let (pre, callee, args, sig) = self.call_value_parts(e);
        Self::with_pre(pre, ExprKind::CallIndirect { callee, args, sig }, ty, line)
    }

    /// A call through a function value whose result is discarded. An owning result is held
    /// until the block ends, as for a direct call.
    pub(crate) fn call_value_stmt(&mut self, e: &ast::Expr, out: &mut Vec<Stmt>) {
        let (pre, callee, args, sig) = self.call_value_parts(e);
        out.extend(pre);
        match sig.ret {
            Some(ret) if self.t.is_owning(self.t.ty(e.id)) => {
                let call = Expr { kind: ExprKind::CallIndirect { callee, args, sig }, ty: ret, line: self.line(e.span) };
                self.keep(call, out);
            }
            _ => out.push(Stmt::CallIndirect { callee, args, sig }),
        }
    }
}
