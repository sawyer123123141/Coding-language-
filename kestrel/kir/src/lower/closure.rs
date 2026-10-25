//! Closure literals. Each literal under one substitution (`LiteralKey`: its `ExprId` and the
//! active substitution, P4) is one KIR function, `(env: i64, params...) -> ret`, however many
//! times it is lowered (a field default is lowered at each construction that leaves the field
//! out). Plain code has no substitution, so a literal there is one function; a copy of a generic,
//! or a field default of a generic struct's instance, gets one per copy or instance, lowered in
//! that substitution's types. Its body is lowered by a `FnLowerer` of its own, with its own
//! scopes, so `return` leaves the closure and a `break` can't reach a loop outside it.
//!
//! The literal itself is a `MakeFn`: the code's address and the environment. A closure that
//! captures nothing has none (`EnvKind::None`), so it allocates nothing; one that captures
//! something gets an environment holding copies of the captured values (see `env`): a stack
//! environment of shallow copies when the literal is directly a call's argument, else a heap one
//! of deep copies.

use kestrel_check::{ClosureInfo, FnId, Subst, Tables, Ty};
use kestrel_syntax::ast::{self, ClosureLit, Program};
use kestrel_syntax::LineIndex;

use super::helpers::{Helpers, LiteralKey};
use super::{kty, FnLowerer};
use crate::ir::*;

impl Helpers {
    /// The code of closure literal `key`, lowered by `lower` the first time it is asked for.
    pub(crate) fn closure_code(&mut self, key: LiteralKey, lower: impl FnOnce(&mut Helpers) -> Func) -> FuncIdx {
        if let Some(&code) = self.closures.get(&key) {
            return code;
        }
        // Reserved first, so the body may name its own literal (a constant that holds a closure
        // that calls itself through the constant).
        let idx = self.reserve();
        self.closures.insert(key, idx);
        let mut func = lower(self);
        // An annotated loop in the literal's body is in this function, not the one around it.
        self.repoint(&func.body, idx);
        func.name = format!("__kestrel_closure_{}", idx.0);
        self.funcs[(idx.0 - self.base) as usize] = func;
        idx
    }
}

impl<'a> FnLowerer<'a> {
    /// A closure literal as a value.
    pub(crate) fn make_closure(&mut self, e: &ast::Expr, lit: &ClosureLit) -> ExprKind {
        let (program, t, lines) = (self.program, self.t, self.lines);
        // The closure's code is lowered under the substitution it is made in (a copy's, or the
        // instance's while a field default is lowered), and in the function around it: the copy
        // itself, not its generic. A literal in a field default or a constant has none.
        let subst = self.active_subst();
        let encl_fn = self.closure(e.id).encl_fn.and(self.fn_id);
        let code = self.helpers.closure_code((e.id, subst), |helpers| lower_closure(program, t, lines, (encl_fn, subst), e, lit, helpers));
        let line = self.line(e.span);
        let env = if self.closure(e.id).captures.is_empty() {
            EnvKind::None
        } else if t.stack_closures.contains(&e.id) {
            self.stack_env(e.id, line)
        } else {
            self.heap_env(e.id, line)
        };
        ExprKind::MakeFn { code, env }
    }
}

/// The function of closure literal `e`: the environment word, then the parameters. Its body
/// starts by binding a reference local to each capture's place in the environment.
fn lower_closure(
    program: &Program,
    t: &Tables,
    lines: &LineIndex,
    (encl_fn, subst): (Option<FnId>, Option<Subst>),
    e: &ast::Expr,
    lit: &ClosureLit,
    helpers: &mut Helpers,
) -> Func {
    let mut cx = FnLowerer::new(program, t, lines, encl_fn, helpers);
    cx.subst = subst;
    let info: &ClosureInfo = cx.closure(e.id);
    cx.in_builtin = t.in_builtin(e.span);
    cx.locals.push(Local { name: "env".into(), ty: KTy::I64, kind: LocalKind::Value });
    cx.declare_locals(&info.locals);
    let sig = cx.sig(info.sig);
    let params = (0..=sig.params.len() as u32).map(LocalIdx).collect();
    let ret = (sig.ret != Ty::Unit).then(|| kty(sig.ret));
    let binds = cx.bind_closure_captures(e.id);
    let lines = cx.lines_of(e.span);
    let mut body = cx.lower_body(&lit.body, ret);
    body.splice(0..0, binds);
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
        lines,
        native: None,
    }
}
