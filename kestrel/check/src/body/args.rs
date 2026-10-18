//! Call arguments: matching them to parameters, `var` arguments, and the exclusive-access check.

use kestrel_syntax::ast::{Arg, Expr, ExprKind};

use super::place::Immutable;
use super::regions::Ctx;
use super::BodyChecker;
use crate::natives::may_want_scratch;
use crate::tables::{FnId, LocalId, ParamSig};
use crate::ty::Ty;

/// How a message names a parameter: a function type's parameters have no names.
pub(super) fn param_ref(p: &ParamSig, fname: &str) -> String {
    if p.name.is_empty() {
        format!("this argument of {}", super::fnvalue::quote(fname))
    } else {
        format!("parameter `{}` of `{fname}`", p.name)
    }
}

impl<'a> BodyChecker<'a> {
    /// Checks a call's arguments against `fid`'s parameters after the first
    /// `skip` (1 for a method's `self`). `var_roots` holds variables the call
    /// already changes (a `var self` receiver); `read_root` is the variable a
    /// receiver that isn't `var` is passed from, by pointer.
    pub(crate) fn check_args(
        &mut self,
        e: &Expr,
        fid: FnId,
        args: &[Arg],
        skip: usize,
        mut var_roots: Vec<(LocalId, Option<usize>)>,
        read_root: Option<LocalId>,
    ) -> Ty {
        let info = &self.t.fns[fid.0 as usize];
        let (params, ret, fname) = (info.params[skip..].to_vec(), info.ret, info.name.clone());
        // Text built for a native's arguments may be freed when the call returns (see `wants_scratch`),
        // so memory made while checking them can't be kept past the call.
        let scratch = may_want_scratch(info, args);
        let native = info.is_native;
        let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        let (slots, mut ok) = self.bind_args(&fname, &names, args, "parameter");
        if ok {
            for (pi, slot) in slots.iter().enumerate() {
                if slot.is_none() {
                    self.error(e.span, format!("missing argument `{}` for `{fname}`", params[pi].name));
                    ok = false;
                }
            }
        }
        if scratch {
            self.ctxs.push(Ctx::Scratch(fname.clone()));
        }
        for (pi, slot) in slots.iter().enumerate() {
            let Some(ai) = *slot else { continue };
            let (a, p) = (&args[ai], &params[pi]);
            if p.is_var {
                if let Some(root) = self.check_var_arg(a, p, &fname) {
                    var_roots.push((root, Some(ai)));
                }
            } else {
                if a.is_var {
                    self.error_help(a.span, format!("parameter `{}` of `{fname}` isn't `var`", p.name), "remove `var`");
                }
                if native && matches!(p.ty, Ty::Array(_)) {
                    self.check_native_array_arg(&a.value, p.ty);
                } else {
                    // A closure literal names the parameter in what it says about purity.
                    if matches!(a.value.kind, ExprKind::Closure(_)) {
                        self.arg_name = Some(p.name.clone());
                    }
                    self.check_value(&a.value, Some(p.ty));
                }
            }
        }
        for (ai, a) in args.iter().enumerate() {
            if !slots.contains(&Some(ai)) {
                self.check_expr(&a.value, None);
            }
        }
        if scratch {
            self.ctxs.pop();
        }
        self.check_exclusive(args, &var_roots, read_root);
        // Arguments run in source order; one passed by pointer earlier must
        // not be changed by a later one.
        for (i, a) in args.iter().enumerate() {
            if a.is_var || self.t.ty(a.value.id).is_aggregate() {
                let later: Vec<&Expr> = args[i + 1..].iter().map(|a| &a.value).collect();
                self.check_changed_later(&[&a.value], &later);
            }
        }
        if ok {
            let order = slots.into_iter().map(|s| s.expect("all bound")).collect();
            self.t.call_args.insert(e.id, order);
        }
        ret
    }

    /// A native's `[T]` parameter also takes a `list[T]`: a native reads only the
    /// `{ptr, len}` that both headers start with.
    fn check_native_array_arg(&mut self, e: &Expr, want: Ty) {
        let found = self.check_expr(e, Some(want));
        if found == Ty::Never {
            self.reject_never(e, found);
            return;
        }
        if let (Ty::List(have), Ty::Array(need)) = (found, want) {
            if self.t.elem(have) == self.t.elem(need) {
                return;
            }
        }
        self.coerce(e, found, want);
    }

    /// Matches arguments to named slots (parameters or fields): positional
    /// first, then labels.
    pub(crate) fn bind_args(&mut self, owner: &str, names: &[&str], args: &[Arg], noun: &str) -> (Vec<Option<usize>>, bool) {
        let mut slots: Vec<Option<usize>> = vec![None; names.len()];
        let mut ok = true;
        let mut seen_label = false;
        for (ai, a) in args.iter().enumerate() {
            let pi = match &a.label {
                None if seen_label => {
                    self.error(a.span, "positional arguments must come before labelled ones");
                    ok = false;
                    continue;
                }
                None if ai >= names.len() => {
                    self.error(a.span, format!("`{owner}` takes {} argument(s), got {}", names.len(), args.len()));
                    ok = false;
                    break;
                }
                None => ai,
                Some(label) => {
                    seen_label = true;
                    match names.iter().position(|n| n == label) {
                        Some(pi) => pi,
                        None => {
                            self.error(a.span, format!("`{owner}` has no {noun} named `{label}`"));
                            ok = false;
                            continue;
                        }
                    }
                }
            };
            if slots[pi].is_some() {
                self.error(a.span, format!("{noun} `{}` is given twice", names[pi]));
                ok = false;
                continue;
            }
            slots[pi] = Some(ai);
        }
        (slots, ok)
    }

    /// Checks an argument for a `var` parameter: a changeable variable, field or element
    /// of exactly the parameter's type. Returns the variable it is rooted at.
    pub(super) fn check_var_arg(&mut self, a: &Arg, p: &ParamSig, fname: &str) -> Option<LocalId> {
        if !matches!(a.value.kind, ExprKind::Ident(_) | ExprKind::Field { .. } | ExprKind::Index { .. }) {
            self.error(a.value.span, "only a variable can be passed as `var`");
            self.check_expr(&a.value, None);
            return None;
        }
        if !a.is_var {
            let help = match &a.value.kind {
                ExprKind::Ident(name) => format!("write `var {name}`"),
                _ => "write `var` before the argument".to_string(),
            };
            let msg = format!("{} is `var`, so the call must say so", param_ref(p, fname));
            self.error_help(a.span, msg, help);
        }
        let ty = self.check_expr(&a.value, None);
        if let ExprKind::Index { base, .. } = &a.value.kind {
            if let Ty::Map(..) = self.t.ty(base.id) {
                let msg = "a map entry can't be passed as `var`; read it, change it, and write it back with `m[k] = v`";
                self.error(a.value.span, msg);
                return None;
            }
        }
        let Some(root) = self.place_root(&a.value) else {
            if ty != Ty::Error {
                self.error(a.value.span, "only a variable can be passed as `var`");
            }
            return None;
        };
        self.check_not_matched(&a.value);
        let name = self.t.local(root).name.clone();
        let problem = match self.immutable(root) {
            None => None,
            Some(Immutable::Let) => Some((
                format!("can't pass `{name}` as `var`: it was declared with `let`"),
                "declare it with `var`".to_string(),
            )),
            Some(Immutable::Param) => Some((
                format!("can't pass parameter `{name}` as `var`: it isn't `var`"),
                format!("declare it as `var {name}`"),
            )),
            Some(Immutable::SelfParam) => Some((
                "can't pass `self` as `var`: the method takes `self`, not `var self`".to_string(),
                "declare the method with `var self`".to_string(),
            )),
            Some(Immutable::Captured) => Some((Self::captured_message(&name), String::new())),
        };
        match problem {
            Some((msg, help)) if help.is_empty() => self.error(a.value.span, msg),
            Some((msg, help)) => self.error_help(a.value.span, msg, help),
            None => {
                self.mark_changed(root);
                self.check_par_write(&a.value);
            }
        }
        if ty != p.ty && ty != Ty::Error && p.ty != Ty::Error {
            let which = if p.name.is_empty() { "this `var` argument".to_string() } else { format!("`var` parameter `{}`", p.name) };
            let msg = format!("{which} needs exactly {}, found {}", self.tn(p.ty), self.tn(ty));
            self.error(a.value.span, msg);
        }
        Some(root)
    }

    /// While a call changes a variable, nothing else in the call may refer to
    /// it by reference: no second `var` use, and no aggregate argument (those
    /// are passed by pointer, so they would see the change). Number
    /// arguments are copied before the call, so they're fine.
    pub(super) fn check_exclusive(&mut self, args: &[Arg], var_roots: &[(LocalId, Option<usize>)], read_root: Option<LocalId>) {
        for (i, &(root, ai)) in var_roots.iter().enumerate() {
            if let (true, Some(ai)) = (var_roots[..i].iter().any(|&(r, _)| r == root), ai) {
                let name = self.t.local(root).name.clone();
                let msg = format!("`{name}` is passed as `var` and also used in another argument of the same call");
                self.error(args[ai].span, msg);
            }
        }
        // A receiver that isn't `var` is passed by pointer too, so it would see the change.
        for &(root, ai) in var_roots {
            if let (true, Some(ai)) = (read_root == Some(root), ai) {
                let name = self.t.local(root).name.clone();
                let msg = format!("`{name}` is passed as `var` and is also the receiver of this call");
                self.error_help(args[ai].span, msg, "copy what the method needs into a separate variable first");
            }
        }
        for (bi, b) in args.iter().enumerate() {
            if var_roots.iter().any(|&(_, ai)| ai == Some(bi)) || !self.t.ty(b.value.id).is_aggregate() {
                continue;
            }
            let Some(root) = self.place_root(&b.value) else { continue };
            let Some(&(_, source)) = var_roots.iter().find(|&&(r, _)| r == root) else { continue };
            let name = self.t.local(root).name.clone();
            let msg = match source {
                None => format!("`{name}` is changed by this call, so it can't also be passed to it"),
                Some(_) => format!("`{name}` is passed as `var` and also used in another argument of the same call"),
            };
            self.error(b.span, msg);
        }
    }
}
