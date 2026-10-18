//! Closure literals. Each literal (by its `ExprId`) is one KIR function, `(env: i64, params...)
//! -> ret`, however many times the literal is lowered (a field default is lowered at each
//! construction that leaves the field out). Its body is lowered by a `FnLowerer` of its own, with
//! its own scopes, so `return` leaves the closure and a `break` can't reach a loop outside it.
//!
//! The literal itself is a `MakeFn`: the code's address and the environment. A closure that
//! captures nothing has none (`EnvKind::None`), so it allocates nothing.

use kestrel_check::{ClosureInfo, Tables, Ty};
use kestrel_syntax::ast::{self, ClosureLit, Program};
use kestrel_syntax::LineIndex;

use super::helpers::Helpers;
use super::{kty, FnLowerer};
use crate::ir::*;

impl Helpers {
    /// The code of closure literal `id`, lowered by `lower` the first time it is asked for.
    pub(crate) fn closure_code(&mut self, id: ast::ExprId, lower: impl FnOnce(&mut Helpers) -> Func) -> FuncIdx {
        if let Some(&code) = self.closures.get(&id) {
            return code;
        }
        // Reserved first, so the body may name its own literal (a constant that holds a closure
        // that calls itself through the constant).
        let idx = self.reserve();
        self.closures.insert(id, idx);
        let mut func = lower(self);
        func.name = format!("__kestrel_closure_{}", idx.0);
        self.funcs[(idx.0 - self.base) as usize] = func;
        idx
    }
}

impl<'a> FnLowerer<'a> {
    /// A closure literal as a value.
    pub(crate) fn make_closure(&mut self, e: &ast::Expr, lit: &ClosureLit) -> ExprKind {
        let (program, t, lines) = (self.program, self.t, self.lines);
        let code = self.helpers.closure_code(e.id, |helpers| lower_closure(program, t, lines, e, lit, helpers));
        let info = &t.closures[&e.id];
        assert!(info.captures.is_empty(), "closures with captures aren't lowered yet");
        ExprKind::MakeFn { code, env: EnvKind::None }
    }
}

/// The function of closure literal `e`: the environment word, then the parameters.
fn lower_closure(program: &Program, t: &Tables, lines: &LineIndex, e: &ast::Expr, lit: &ClosureLit, helpers: &mut Helpers) -> Func {
    let info: &ClosureInfo = &t.closures[&e.id];
    let mut cx = FnLowerer::new(program, t, lines, info.encl_fn, helpers);
    cx.in_builtin = t.in_builtin(e.span);
    cx.locals.push(Local { name: "env".into(), ty: KTy::I64, kind: LocalKind::Value });
    cx.declare_locals(&info.locals);
    let sig = t.sig(info.sig);
    let params = (0..=sig.params.len() as u32).map(LocalIdx).collect();
    let ret = (sig.ret != Ty::Unit).then(|| kty(sig.ret));
    let body = cx.lower_body(&lit.body, ret);
    Func {
        name: String::new(),
        params,
        ret,
        locals: cx.locals,
        body,
        is_pure: info.is_pure,
        io: !info.is_pure,
        mutates_params: info.mutates_params,
        exported: false,
        native: None,
    }
}
