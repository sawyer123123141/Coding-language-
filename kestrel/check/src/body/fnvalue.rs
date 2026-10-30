//! Function values: a named function used as a value, and calls through a value (a local, a
//! struct field, or any expression of function type).

use kestrel_syntax::ast::{Arg, Expr, ExprKind};
use kestrel_syntax::Span;

use super::args::param_ref;
use super::closure::Impure;
use super::exclusive::CalleePlace;
use super::lists::written;
use super::BodyChecker;
use crate::builtins::Builtin;
use crate::generic::GenericOwner;
use crate::sig::SigId;
use crate::tables::{ConstId, FnId, LocalId, ParamSig, Res};
use crate::ty::Ty;

/// The closure that wraps built-in `name`, for the message that says it can't be a value:
/// `(a: int, b: int) { min(a, b) }`. The functions about whole numbers (`min`, `abs`,
/// `randi_range`...) are shown with `int`, the math functions with `float`, and `print` with `int`.
pub(super) fn builtin_wrapper(name: &str) -> String {
    let b = Builtin::global(name);
    let whole = b.is_some_and(|b| {
        matches!(b, Builtin::Min | Builtin::Max | Builtin::Abs | Builtin::Sign | Builtin::Clamp | Builtin::RandiRange | Builtin::RandSeed | Builtin::Checksum)
    });
    let ty = if whole || name == "print" { "int" } else { "float" };
    let names: &[&str] = match (name, b.map_or(1, Builtin::arity)) {
        ("clamp", _) => &["x", "lo", "hi"],
        ("lerp", _) => &["a", "b", "t"],
        (_, 0) => &[],
        (_, 1) => &["x"],
        (_, 2) => &["a", "b"],
        _ => &["a", "b", "c"],
    };
    let params: Vec<String> = names.iter().map(|n| format!("{n}: {ty}")).collect();
    format!("({}) {{ {name}({}) }}", params.join(", "), names.join(", "))
}

impl<'a> BodyChecker<'a> {
    /// A user function named as a value: `let f = score`.
    pub(crate) fn fn_name_value(&mut self, e: &Expr, fid: FnId) -> Ty {
        if self.t.func(fid).is_native {
            self.error(e.span, "native functions can't be used as values yet; wrap it in a closure");
            return Ty::Error;
        }
        // A generic function is a value once its types are fixed (R8).
        if self.is_generic_fn(fid) {
            let name = self.t.func(fid).name.clone();
            let example = self.int_types(GenericOwner::Fn(fid), &name);
            self.error(e.span, format!("`{name}` is generic; give its types to use it as a value, like `{example}`"));
            return Ty::Error;
        }
        self.t.res.insert(e.id, Res::FnValue(fid));
        self.note_fn_value(fid, e.span);
        Ty::Fn(self.t.fn_sig(fid))
    }

    /// `p.len2` where `len2` is a method (or a vector's built-in): methods aren't values yet.
    /// Returns whether it was one, after reporting it.
    pub(crate) fn method_value_error(&mut self, base_ty: Ty, name: &str, span: Span) -> bool {
        let is_method = match base_ty {
            Ty::Struct(sid) => {
                self.t.strukt(sid).methods.contains_key(name) || (self.t.vector(base_ty).is_some() && Builtin::vector_method(name).is_some())
            }
            Ty::Enum(id) => self.t.enm(id).methods.contains_key(name),
            Ty::Iface(id) => self.iface_method(id, name).is_some(),
            Ty::Param(p) => self.t.generic.param(p).ifaces.iter().any(|&i| self.iface_method(i, name).is_some()),
            _ => self.t.vector(base_ty).is_some() && Builtin::vector_method(name).is_some(),
        };
        if is_method {
            self.error(span, format!("methods can't be used as values yet; wrap it: `(e) {{ e.{name}() }}`"));
        }
        is_method
    }

    /// The field of `receiver_ty` called `name`, when it has function type and no method has
    /// that name: `b.on_click()` calls it. Its index and signature.
    pub(crate) fn fn_field(&self, receiver_ty: Ty, name: &str) -> Option<(usize, SigId)> {
        let Ty::Struct(sid) = receiver_ty else { return None };
        let info = self.t.strukt(sid);
        if info.methods.contains_key(name) {
            return None;
        }
        let i = info.fields.iter().position(|f| f.name == name)?;
        match info.fields[i].ty {
            Ty::Fn(sig) => Some((i, sig)),
            _ => None,
        }
    }

    /// `expr(args)` where `expr` is anything but a bare name or a method: `handlers[i](3)`.
    pub(crate) fn check_call_value(&mut self, e: &Expr) -> Ty {
        let ExprKind::CallValue { callee, args } = &e.kind else { unreachable!("check_call_value on another expression") };
        let ty = self.check_expr(callee, None);
        let shown = shown_callee(callee);
        match ty {
            Ty::Fn(sig) => {
                let place = self.source_key(callee).map(|key| CalleePlace { key, shown: shown.clone() });
                self.call_through(e, &shown, sig, callee.span, args, place)
            }
            other => {
                if other != Ty::Error {
                    let msg = format!("{} has type {}, which can't be called", quote(&shown), self.tn(other));
                    self.error(callee.span, msg);
                }
                self.check_loose_args(args);
                Ty::Error
            }
        }
    }

    /// `f(args)` where `f` is a local or parameter.
    pub(crate) fn check_local_call(&mut self, e: &Expr, local: LocalId, name: &str, name_span: Span, args: &[Arg]) -> Ty {
        self.t.call_local.insert(e.id, local);
        self.check_local_use(local, name, name_span);
        self.note_local_call_capture(e, local, name_span);
        match self.t.local(local).ty {
            Ty::Fn(sig) => {
                let place = CalleePlace { key: self.local_key(local), shown: name.to_string() };
                self.call_through(e, name, sig, name_span, args, Some(place))
            }
            other => {
                if other != Ty::Error {
                    let msg = format!("`{name}` is a variable of type {}, not a function", self.tn(other));
                    self.error(name_span, msg);
                }
                self.check_loose_args(args);
                Ty::Error
            }
        }
    }

    /// `RULE(args)` where `RULE` is a constant of function type (`let RULE = square`).
    pub(crate) fn check_const_call(&mut self, e: &Expr, c: ConstId, sig: SigId, name_span: Span, args: &[Arg]) -> Ty {
        self.t.call_const.insert(e.id, c);
        let name = self.t.consts[c.0 as usize].name.clone();
        self.call_through(e, &name, sig, name_span, args, None)
    }

    /// `b.on_click(args)` where `on_click` is a field of function type.
    pub(crate) fn check_field_call(&mut self, e: &Expr, receiver: &Expr, name: &str, field: usize, sig: SigId, args: &[Arg]) -> Ty {
        self.t.field_index.insert(e.id, field as u32);
        self.note_field_call_capture(e, receiver, field as u32, name, Ty::Fn(sig));
        let shown = format!("{}.{name}", written(receiver));
        let place = self.source_key(receiver).map(|mut key| {
            key.steps.push(Some(name.to_string()));
            CalleePlace { key, shown: shown.clone() }
        });
        self.call_through(e, &shown, sig, e.span, args, place)
    }

    /// A call through a function value of signature `sig`, written as `shown`. `callee` is the
    /// place the value is read from, which the call can't change (see `exclusive`).
    fn call_through(&mut self, e: &Expr, shown: &str, sig: SigId, span: Span, args: &[Arg], callee: Option<CalleePlace>) -> Ty {
        self.t.res.insert(e.id, Res::CallValue(sig));
        self.check_value_purity(sig, span, shown);
        self.check_value_args(e, shown, sig, args, callee)
    }

    /// A non-pure function value can't be called in a `pure` function or a `par for` body.
    pub(crate) fn check_value_purity(&mut self, sig: SigId, span: Span, shown: &str) {
        if self.t.sig(sig).is_pure {
            return;
        }
        if self.closure_impure(span, Impure::Value(&quote(shown))) {
            return;
        }
        let pure_type = format!("pure {}", self.t.sig_name(sig));
        if self.is_pure {
            let msg = format!(
                "{} isn't a pure function type, so a `pure` function can't call it; declare the parameter as `{pure_type}`",
                quote(shown)
            );
            self.error(span, msg);
        } else if !self.par.is_empty() {
            let msg = format!("`par for` bodies can only call `pure` functions; {} isn't a pure function type", quote(shown));
            self.error_help(span, msg, format!("declare it as `{pure_type}`"));
        }
    }

    /// The arguments of a call through a value: positional, one per parameter, `var` where the
    /// signature says so.
    pub(crate) fn check_value_args(&mut self, e: &Expr, shown: &str, sig: SigId, args: &[Arg], callee: Option<CalleePlace>) -> Ty {
        let sig = self.t.sig(sig).clone();
        let params: Vec<ParamSig> = sig.params.iter().map(|p| ParamSig { name: String::new(), ty: p.ty, is_var: p.is_var }).collect();
        let mut ok = true;
        if let Some(a) = args.iter().find(|a| a.label.is_some()) {
            self.error_help(a.span, "calls through a function value take no labels", "pass the arguments in order, without names");
            ok = false;
        }
        if args.len() != params.len() {
            self.error(e.span, format!("{} takes {} argument(s), got {}", quote(shown), params.len(), args.len()));
            ok = false;
        }
        let mut var_roots = Vec::new();
        for (i, a) in args.iter().enumerate() {
            match params.get(i) {
                Some(p) if p.is_var => {
                    if let Some(root) = self.check_var_arg(a, p, shown, None) {
                        var_roots.push((root, Some(i)));
                    }
                }
                Some(p) => {
                    if a.is_var {
                        self.error_help(a.span, format!("{} isn't `var`", param_ref(p, shown)), "remove `var`");
                    }
                    self.note_stack_closure(a);
                    self.going_into(&a.value, super::narrow::Site::Parameter, |c| c.check_arg_value(&a.value, p.ty));
                }
                None => {
                    self.check_expr(&a.value, None);
                }
            }
        }
        self.check_exclusive(args, &var_roots, None);
        let changes = self.call_changes(args, None);
        self.check_call_captures(e.span, args, &changes, callee);
        // Arguments run in source order; one passed by pointer earlier must not be changed by a later one.
        for (i, a) in args.iter().enumerate() {
            if a.is_var || self.t.ty(a.value.id).is_aggregate() {
                let later: Vec<&Expr> = args[i + 1..].iter().map(|a| &a.value).collect();
                self.check_changed_later(&[&a.value], &later);
            }
        }
        if ok {
            self.t.call_args.insert(e.id, (0..args.len()).collect());
        }
        sig.ret
    }
}

/// A callee as a message writes it: code in backticks, or the plain words for one that isn't a place.
pub(super) fn quote(shown: &str) -> String {
    if shown.starts_with("this ") {
        shown.to_string()
    } else {
        format!("`{shown}`")
    }
}

/// How a message names the function value a postfix call calls.
pub(super) fn shown_callee(callee: &Expr) -> String {
    match callee.kind {
        ExprKind::Ident(_) | ExprKind::Field { .. } | ExprKind::Index { .. } => written(callee),
        ExprKind::TypeApply { ref name, .. } => name.clone(),
        _ => "this function value".to_string(),
    }
}
