//! Stores into `var` parameters, the body's side (P38, see `var_stores`): a store of the caller's
//! memory into the function's own `var` parameter is recorded instead of refused, each direct
//! call records what it would need if its callee stores into a `var` argument, and a function
//! named as a value records where.

use kestrel_syntax::ast::{Arg, Expr};
use kestrel_syntax::{Diagnostic, Span};

use super::args::{CallSig, Receiver};
use super::lists::written;
use super::regions::{bit, Ctx, StoreTarget};
use super::BodyChecker;
use crate::tables::{FnId, LocalId};
use crate::ty::Ty;
use crate::var_stores::PendingCall;

impl<'a> BodyChecker<'a> {
    /// The index of `root` among the parameters of the function being checked, when it is one
    /// of its own `var` parameters and no closure is being checked (a closure is only ever
    /// called through a value, so its own `var` parameters keep the error). `main` has none.
    fn own_var_param(&self, root: LocalId) -> Option<u32> {
        let f = self.fn_id?;
        let info = self.t.local(root);
        if !self.frames.is_empty() || !info.is_var_param || info.owner != Some(f) || info.closure.is_some() {
            return None;
        }
        let func = self.t.func(f);
        let i = func.locals.iter().position(|&l| l == root)?;
        (i < func.params.len()).then_some(i as u32)
    }

    /// Records a store of memory from the caller's current arena into `root`, when it is one of
    /// the function's own `var` parameters: each call checks it instead. Returns whether it was.
    pub(super) fn defer_var_store(&mut self, root: LocalId) -> bool {
        let (Some(f), Some(i)) = (self.fn_id, self.own_var_param(root)) else { return false };
        self.t.var_stores.direct.push((f, i));
        true
    }

    /// For a call `e` whose arguments bound (`slots`, parameter by parameter after the receiver),
    /// records each changeable `var` argument (`var_roots`, as `check_args` collected them) that a
    /// store by the callee would need checked: the caller's current arena and every argument's
    /// memory must be storable in it, judged by `ctx_holds` as a store written here is.
    pub(super) fn note_call_stores(
        &mut self,
        e: &Expr,
        sig: &CallSig,
        args: &[Arg],
        recv: Option<Receiver<'_>>,
        slots: &[Option<usize>],
        var_roots: &[(LocalId, Option<usize>)],
    ) {
        let skip = usize::from(recv.is_some());
        let mut r = self.current();
        for a in args {
            r |= self.region(&a.value, false);
        }
        if let Some(rc) = recv {
            r |= self.region(rc.expr, false);
        }
        let Some(f) = self.t.fns.get(sig.fid.0 as usize).map(|i| i.name.clone()) else { return };
        for (i, p) in sig.params.iter().enumerate() {
            // A type with no heap memory has nothing to store (`swap[int]`, `Cell[int]`).
            if !p.is_var || !self.t.has_heap(p.ty) {
                continue;
            }
            let target = if i < skip {
                recv.filter(|rc| rc.var_root.is_some()).map(|rc| rc.expr)
            } else {
                let ai = slots.get(i - skip).copied().flatten().filter(|&ai| var_roots.iter().any(|&(_, s)| s == Some(ai)));
                ai.and_then(|ai| args.get(ai)).map(|a| &a.value)
            };
            let Some(target) = target else { continue };
            let Some(st) = self.store_target(target) else { continue };
            // A place that can't be changed was already reported.
            if self.immutable(st.root).is_some() {
                continue;
            }
            let (mut caller, mut refused) = (None, None);
            for c in (0..self.ctxs.len()).filter(|&c| r & bit(c) != 0) {
                if self.ctx_holds(c, &st) {
                    continue;
                }
                if let (Ctx::Function, Some(caller_fn), Some(j)) = (&self.ctxs[c], self.fn_id, self.own_var_param(st.root)) {
                    caller = Some((caller_fn, j));
                    continue;
                }
                if refused.is_none() {
                    refused = Some(self.call_store_error(c, &f, target, &st));
                }
            }
            if caller.is_some() || refused.is_some() {
                let call = PendingCall { call: e.id, callee: sig.fid, param: i as u32, caller, refused };
                self.t.var_stores.calls.push(call);
            }
        }
    }

    /// `fid` named as a value at `span`: refused later if it stores into a `var` parameter.
    pub(super) fn note_fn_value(&mut self, fid: FnId, span: Span) {
        self.t.var_stores.values.push((fid, span));
    }

    /// The error for a call to `f` that may store memory of context `c` in `target`, the place
    /// passed for a `var` parameter, which doesn't hold it.
    fn call_store_error(&self, c: usize, f: &str, target: &Expr, st: &StoreTarget) -> Diagnostic {
        let place = written(target);
        let (msg, help) = match &self.ctxs[c] {
            Ctx::Function => (
                format!("`{f}` stores memory in `{place}` that is in the caller's current arena, which may not live as long as `{place}`"),
                self.caller_memory_help(&st.name, st.written_ty),
            ),
            Ctx::Scoped => (
                format!("`{f}` stores memory in `{place}` that is freed at the end of the `arena {{ }}` block, but `{place}` lives longer"),
                "call it outside the block, or pass a variable declared inside".to_string(),
            ),
            Ctx::Scratch(native) => (
                format!("`{f}` stores memory in `{place}` that is freed when the call to `{native}` returns, but `{place}` lives longer"),
                format!("call it before the call to `{native}`, or pass a variable declared inside the argument"),
            ),
            Ctx::Owned(p) => {
                let owner = self.path_name(p);
                (
                    format!("`{f}` stores memory in `{place}` that belongs to `{owner}`, but `{place}` isn't part of `{owner}`"),
                    format!("pass a place inside `{owner}`, or call it outside the `in` block"),
                )
            }
        };
        Diagnostic::error(target.span, msg).with_help(help)
    }

    /// What to do about memory in the caller's current arena that is stored
    /// into `name`, a variable of type `ty`: build it in an arena that `name`
    /// owns, which takes an `Arena` field on its type.
    pub(super) fn caller_memory_help(&self, name: &str, ty: Ty) -> String {
        let (owner, fields) = match ty {
            Ty::Struct(s) => (format!("`{}`", self.t.strukt(s).name), &self.t.strukt(s).fields),
            Ty::Variant(e, v) => {
                let variant = &self.t.enm(e).variants[v as usize];
                (format!("the `{}` variant", variant.name), &variant.fields)
            }
            _ => {
                return format!(
                    "build it in the caller, or give the owner of `{name}` an `Arena` field and build it inside `in <owner>.<that field> {{ }}`"
                )
            }
        };
        match fields.iter().find(|f| f.ty == Ty::Arena) {
            Some(arena) => format!("allocate it with `in {name}.{} {{ }}`", arena.name),
            None => format!("give {owner} an `Arena` field, and build it inside `in {name}.<that field> {{ }}`"),
        }
    }
}
