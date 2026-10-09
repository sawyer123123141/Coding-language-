use kestrel_syntax::ast::{Arg, Expr, ExprKind, InterpPart};
use kestrel_syntax::Span;

use super::place::Immutable;
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
        if name == "Arena" {
            self.t.res.insert(e.id, Res::NewArena);
            if !args.is_empty() {
                self.error(e.span, "`Arena()` takes no arguments");
                for a in args {
                    self.check_expr(&a.value, None);
                }
            }
            return Ty::Arena;
        }
        if let Some(&(_, to)) = CONVERSIONS.iter().find(|(n, _)| n == name) {
            return self.check_convert(e, to, args);
        }
        if let Some(&sid) = self.t.struct_by_name.get(name) {
            return self.check_construct(e, sid, args);
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
        let ty = self.check_expr(e, None);
        match self.t.res.get(&e.id).copied() {
            Some(Res::Fn(f)) if self.t.fns[f.0 as usize].is_pure => {
                let name = self.t.fns[f.0 as usize].name.clone();
                self.error(e.span, format!("the result of pure function `{name}` is unused, so this line does nothing"));
            }
            Some(Res::Convert(_) | Res::Construct(_) | Res::Variant(..) | Res::NewArena) => {
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
            if let ExprKind::Interp(parts) = &a.value.kind {
                self.t.expr_ty[a.value.id.0 as usize] = Ty::Str;
                for part in parts {
                    if let InterpPart::Expr(x) = part {
                        self.check_printable(x);
                    }
                }
                continue;
            }
            self.check_printable(&a.value);
        }
        Ty::Unit
    }

    fn check_printable(&mut self, e: &Expr) {
        let ty = self.check_expr(e, None);
        let ty = self.reject_never(e, ty);
        if ty == Ty::Unit {
            self.error(e.span, "this has no value to print");
        }
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
            let msg = format!("can't convert {} to {}", self.tn(from), to.name());
            self.error(args[0].value.span, msg);
        }
        to
    }

    pub(crate) fn check_callee_purity(&mut self, span: Span, callee: FnId) {
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
        self.check_args(e, fid, args, 0, Vec::new())
    }

    /// Checks a call's arguments against `fid`'s parameters after the first
    /// `skip` (1 for a method's `self`). `var_roots` holds variables the call
    /// already changes (a `var self` receiver).
    pub(crate) fn check_args(
        &mut self,
        e: &Expr,
        fid: FnId,
        args: &[Arg],
        skip: usize,
        mut var_roots: Vec<(LocalId, Option<usize>)>,
    ) -> Ty {
        let info = &self.t.fns[fid.0 as usize];
        let (params, ret, fname) = (info.params[skip..].to_vec(), info.ret, info.name.clone());
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
                self.check_value(&a.value, Some(p.ty));
            }
        }
        for (ai, a) in args.iter().enumerate() {
            if !slots.contains(&Some(ai)) {
                self.check_expr(&a.value, None);
            }
        }
        self.check_exclusive(args, &var_roots);
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
    fn check_var_arg(&mut self, a: &Arg, p: &ParamSig, fname: &str) -> Option<LocalId> {
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
            let msg = format!("parameter `{}` of `{fname}` is `var`, so the call must say so", p.name);
            self.error_help(a.span, msg, help);
        }
        let ty = self.check_expr(&a.value, None);
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
        };
        match problem {
            Some((msg, help)) => self.error_help(a.value.span, msg, help),
            None => self.mark_changed(root),
        }
        if ty != p.ty && ty != Ty::Error && p.ty != Ty::Error {
            let msg = format!("`var` parameter `{}` needs exactly {}, found {}", p.name, self.tn(p.ty), self.tn(ty));
            self.error(a.value.span, msg);
        }
        Some(root)
    }

    /// While a call changes a variable, nothing else in the call may refer to
    /// it by reference: no second `var` use, and no aggregate argument (those
    /// are passed by pointer, so they would see the change). Number
    /// arguments are copied before the call, so they're fine.
    fn check_exclusive(&mut self, args: &[Arg], var_roots: &[(LocalId, Option<usize>)]) {
        for (i, &(root, ai)) in var_roots.iter().enumerate() {
            if let (true, Some(ai)) = (var_roots[..i].iter().any(|&(r, _)| r == root), ai) {
                let name = self.t.local(root).name.clone();
                let msg = format!("`{name}` is passed as `var` and also used in another argument of the same call");
                self.error(args[ai].span, msg);
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
