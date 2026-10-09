use kestrel_syntax::ast::{Arg, Expr, ExprKind};
use kestrel_syntax::Span;

use super::BodyChecker;
use crate::tables::{FnId, LocalId, ParamSig, Res};
use crate::ty::Ty;

const CONVERSIONS: &[(&str, Ty)] = &[
    ("int", Ty::Int),
    ("i64", Ty::I64),
    ("u8", Ty::U8),
    ("float", Ty::Float),
    ("f64", Ty::F64),
];

impl<'a> BodyChecker<'a> {
    pub(crate) fn check_call(&mut self, e: &Expr, _want: Option<Ty>) -> Ty {
        let ExprKind::Call { name, name_span, args } = &e.kind else { unreachable!("check_call on non-call") };
        if name == "print" {
            return self.check_print(e, args);
        }
        if let Some(&(_, to)) = CONVERSIONS.iter().find(|(n, _)| n == name) {
            return self.check_convert(e, to, args);
        }
        let Some(&fid) = self.t.fn_by_name.get(name) else {
            self.error(*name_span, format!("unknown function `{name}`"));
            for a in args {
                self.check_expr(&a.value, None);
            }
            return Ty::Error;
        };
        self.t.res.insert(e.id, Res::Fn(fid));
        self.check_callee_purity(*name_span, fid);
        self.check_user_call(e, fid, args)
    }

    /// A call statement whose result is thrown away.
    pub(crate) fn check_discarded_call(&mut self, e: &Expr) -> Ty {
        let ty = self.check_call(e, None);
        match self.t.res.get(&e.id).copied() {
            Some(Res::Fn(f)) if self.t.fns[f.0 as usize].is_pure => {
                let name = self.t.fns[f.0 as usize].name.clone();
                self.error(e.span, format!("the result of pure function `{name}` is unused, so this line does nothing"));
            }
            Some(Res::Convert(_)) => {
                self.error(e.span, "this line has no effect; did you mean to continue the line above?");
            }
            _ => {}
        }
        ty
    }

    fn check_print(&mut self, e: &Expr, args: &[Arg]) -> Ty {
        self.t.res.insert(e.id, Res::Print);
        if self.is_pure {
            self.error(e.span, "pure functions can't call `print`");
        }
        if let Some(f) = self.fn_id {
            self.t.fns[f.0 as usize].io = true;
        }
        for a in args {
            if a.label.is_some() || a.is_var {
                self.error(a.span, "`print` arguments can't be labelled or `var`");
            }
            if let ExprKind::Str(_) = a.value.kind {
                self.t.expr_ty[a.value.id.0 as usize] = Ty::Str;
                continue;
            }
            let ty = self.check_expr(&a.value, None);
            let ty = self.reject_never(&a.value, ty);
            if ty == Ty::Unit {
                self.error(a.value.span, "this has no value to print");
            }
        }
        Ty::Unit
    }

    fn check_convert(&mut self, e: &Expr, to: Ty, args: &[Arg]) -> Ty {
        self.t.res.insert(e.id, Res::Convert(to));
        if args.len() != 1 || args[0].label.is_some() || args[0].is_var {
            self.error(e.span, format!("`{}(...)` takes exactly one value", to.name()));
            for a in args {
                self.check_expr(&a.value, None);
            }
            return to;
        }
        let from = self.check_expr(&args[0].value, Some(to));
        if !from.is_numeric() && from != Ty::Error {
            self.error(args[0].value.span, format!("can't convert {} to {}", from.name(), to.name()));
        }
        to
    }

    fn check_callee_purity(&mut self, span: Span, callee: FnId) {
        let info = &self.t.fns[callee.0 as usize];
        if self.is_pure && !info.is_pure {
            let name = info.name.clone();
            self.error_help(
                span,
                format!("pure functions can only call pure functions; `{name}` isn't pure"),
                format!("mark `{name}` as `pure` if it has no effects"),
            );
        }
    }

    fn check_user_call(&mut self, e: &Expr, fid: FnId, args: &[Arg]) -> Ty {
        let info = &self.t.fns[fid.0 as usize];
        let (params, ret, fname) = (info.params.clone(), info.ret, info.name.clone());
        let (slots, mut ok) = self.bind_args(&fname, &params, args);
        if ok {
            for (pi, slot) in slots.iter().enumerate() {
                if slot.is_none() {
                    self.error(e.span, format!("missing argument `{}` for `{fname}`", params[pi].name));
                    ok = false;
                }
            }
        }
        let mut var_args: Vec<(LocalId, usize)> = Vec::new();
        for (pi, slot) in slots.iter().enumerate() {
            let Some(ai) = *slot else { continue };
            let (a, p) = (&args[ai], &params[pi]);
            if p.is_var {
                if let Some(l) = self.check_var_arg(a, p, &fname) {
                    var_args.push((l, ai));
                }
            } else {
                if a.is_var {
                    self.error_help(a.span, format!("parameter `{}` of `{fname}` isn't `var`", p.name), "remove `var`");
                }
                self.check_value(&a.value, Some(p.ty));
            }
        }
        for (ai, a) in args.iter().enumerate() {
            if !slots.contains(&Some(ai)) {
                self.check_expr(&a.value, None);
            }
        }
        self.check_exclusive(args, &var_args);
        if ok {
            let order = slots.into_iter().map(|s| s.expect("all bound")).collect();
            self.t.call_args.insert(e.id, order);
        }
        ret
    }

    /// Matches arguments to parameters: positional first, then labels.
    fn bind_args(&mut self, fname: &str, params: &[ParamSig], args: &[Arg]) -> (Vec<Option<usize>>, bool) {
        let mut slots: Vec<Option<usize>> = vec![None; params.len()];
        let mut ok = true;
        let mut seen_label = false;
        for (ai, a) in args.iter().enumerate() {
            let pi = match &a.label {
                None if seen_label => {
                    self.error(a.span, "positional arguments must come before labelled ones");
                    ok = false;
                    continue;
                }
                None if ai >= params.len() => {
                    self.error(a.span, format!("`{fname}` takes {} argument(s), got {}", params.len(), args.len()));
                    ok = false;
                    break;
                }
                None => ai,
                Some(label) => {
                    seen_label = true;
                    match params.iter().position(|p| &p.name == label) {
                        Some(pi) => pi,
                        None => {
                            self.error(a.span, format!("`{fname}` has no parameter named `{label}`"));
                            ok = false;
                            continue;
                        }
                    }
                }
            };
            if slots[pi].is_some() {
                self.error(a.span, format!("parameter `{}` is given twice", params[pi].name));
                ok = false;
                continue;
            }
            slots[pi] = Some(ai);
        }
        (slots, ok)
    }

    /// Checks an argument for a `var` parameter; returns the variable passed.
    fn check_var_arg(&mut self, a: &Arg, p: &ParamSig, fname: &str) -> Option<LocalId> {
        let ExprKind::Ident(name) = &a.value.kind else {
            self.error(a.value.span, "only a variable can be passed as `var`");
            self.check_expr(&a.value, None);
            return None;
        };
        if !a.is_var {
            self.error_help(
                a.span,
                format!("parameter `{}` of `{fname}` is `var`, so the call must say so", p.name),
                format!("write `var {name}`"),
            );
        }
        let ty = self.check_expr(&a.value, None);
        let Some(Res::Local(l)) = self.t.res.get(&a.value.id).copied() else {
            if ty != Ty::Error {
                self.error(a.value.span, "only a variable can be passed as `var`");
            }
            return None;
        };
        let info = self.t.locals[l.0 as usize].clone();
        if !info.mutable {
            self.error_help(
                a.value.span,
                format!("can't pass `{name}` as `var`: it was declared with `let`"),
                "declare it with `var`",
            );
        }
        if ty != p.ty && ty != Ty::Error && p.ty != Ty::Error {
            let msg = format!("`var` parameter `{}` needs exactly {}, found {}", p.name, p.ty.name(), ty.name());
            self.error(a.value.span, msg);
        }
        if info.is_var_param {
            let f = self.fn_id.expect("inside a function");
            self.t.fns[f.0 as usize].mutates_params = true;
        } else {
            self.t.locals[l.0 as usize].addressed = true;
        }
        Some(l)
    }

    /// A variable passed as `var` can't also be another argument of the same call.
    fn check_exclusive(&mut self, args: &[Arg], var_args: &[(LocalId, usize)]) {
        for &(l, ai) in var_args {
            for (bi, b) in args.iter().enumerate() {
                let reported_from_other_side = var_args.iter().any(|&(_, x)| x == bi) && bi < ai;
                if bi == ai || reported_from_other_side {
                    continue;
                }
                if self.t.res.get(&b.value.id) == Some(&Res::Local(l)) {
                    let name = self.t.locals[l.0 as usize].name.clone();
                    self.error(b.span, format!("`{name}` is passed as `var` and also used in another argument of the same call"));
                }
            }
        }
    }
}
