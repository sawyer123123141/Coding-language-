//! Working out a generic's types where none are written (pick 2, G7): a call of a generic
//! function, a construction of a generic struct, a variant of a generic enum. `infer` checks each
//! argument once and gives its type, so the use is then finished against the filled-in signature
//! without checking the arguments again (P3). The steps, in order:
//!
//! 1. **Values,** in source order (P33), except the deferred ones: each is checked with its
//!    parameter's type as the hint once that type is known, else with none, and its type meets the
//!    parameter's (`Bindings::meet`). A non-empty array literal for an open `list[...]` decides
//!    through its items and becomes a list (P11).
//! 2. **The expected type** fills only the placeholders still open.
//! 3. **Deferred arguments,** in source order: closure literals with an untyped parameter, `none`,
//!    `[]` and `{}`. A closure's parameter types must be known by now (else nothing can be told,
//!    P10); its result, if still open, comes from its body and decides exactly. `none`, `[]` and
//!    `{}` decide nothing: they are checked once every placeholder is known.
//! 4. **Every placeholder decided,** else "can't tell"; each meets R3 and its rules (R7), at the
//!    argument that decided it (G20).
//!
//! One error per use (P10): the first conflict as it happens, or the first undecided placeholder
//! in declaration order; deferred arguments not yet checked are then skipped.

use kestrel_syntax::ast::{Arg, Expr, ExprKind};
use kestrel_syntax::Span;

use super::moves::Moved;
use super::BodyChecker;
use crate::collect_ty::TypeScope;
use crate::generic::infer::{Binding, Bindings, Conflict};
use crate::generic::{GenericOwner, TyListId};
use crate::sig::{Sig, SigParam};
use crate::tables::ParamSig;
use crate::ty::Ty;

/// The use was refused, and the error reported.
pub(crate) struct Reported;

/// How a use of a generic is written, for the examples in messages.
#[derive(Clone, Copy)]
pub(crate) enum Form<'n> {
    /// `best(...)`
    Call,
    /// `Stack(...)`
    Construct,
    /// `Maybe.Nothing`
    Unit(&'n str),
    /// `Result.Ok(...)`
    Variant(&'n str),
}

/// What is inferred: whose placeholders, the parameters (or fields) the arguments are bound to and
/// the result, in the callee's terms.
pub(crate) struct Callee<'n> {
    pub(crate) owner: GenericOwner,
    /// The generic as written: `pick`, `Stack`, `Result`.
    pub(crate) name: &'n str,
    pub(crate) form: Form<'n>,
    /// Where an error about the whole use goes.
    pub(crate) at: Span,
    pub(crate) params: Vec<ParamSig>,
    pub(crate) ret: Ty,
}

/// Whether an argument waits for step 3 whatever the parameter: a closure literal with an untyped
/// parameter, `none`, `[]` or `{}`. (A closure whose result is still open waits too, `waits`.)
pub(super) fn deferred(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Closure(lit) => lit.params.iter().any(|p| p.ty.is_none()),
        ExprKind::None => true,
        ExprKind::ArrayLit(items) => items.is_empty(),
        ExprKind::MapLit(entries) => entries.is_empty(),
        _ => false,
    }
}

impl<'a> BodyChecker<'a> {
    /// The types for `callee`'s placeholders, worked out from `args` (bound to its parameters by
    /// `slots`, every one given or defaulted) and the expected type, and each argument's checked
    /// type, by argument.
    pub(crate) fn infer(&mut self, callee: &Callee, args: &[Arg], slots: &[Option<usize>], expected: Option<Ty>) -> Result<(TyListId, Vec<Ty>), Reported> {
        let mut b = Bindings::new(self.t, callee.owner);
        let param_of: Vec<Option<usize>> = (0..args.len()).map(|ai| slots.iter().position(|s| *s == Some(ai))).collect();
        let mut found = vec![Ty::Error; args.len()];
        // An argument that failed to check was reported: nothing more is said about the use.
        let mut quiet = false;
        // A construction or variant moves each argument once it is checked, in source order, as
        // the written form does: a later argument may not use what an earlier one moved. A
        // deferred argument is checked with the moves made before it in source order.
        let moves = matches!(callee.form, Form::Construct | Form::Variant(_));
        let mut waiting = Vec::new();
        for (ai, a) in args.iter().enumerate() {
            let Some(pi) = param_of[ai] else { continue };
            let p = &callee.params[pi];
            if self.waits(&b, &a.value, p) {
                waiting.push((ai, moves.then(|| self.moved.clone())));
                continue;
            }
            let ty = self.infer_arg(&b, &a.value, p);
            (found[ai], quiet) = (ty, quiet || ty == Ty::Error);
            if moves {
                self.consume(&a.value);
            }
            if let Err(c) = b.meet(self.t, p.ty, ty, p.is_var, Some(ai)) {
                self.report_conflict(callee, &b, c, args, &param_of);
                // The rest are checked as written; the deferred ones aren't (P10).
                for a in args[ai + 1..].iter().filter(|a| !deferred(&a.value)) {
                    self.check_expr(&a.value, None);
                }
                return Err(Reported);
            }
        }
        if let Some(x) = expected {
            b.fill(self.t, callee.ret, x);
        }
        let mut later = Vec::new();
        for (ai, before) in waiting {
            let (value, p) = (&args[ai].value, &callee.params[param_of[ai].expect("bound")]);
            if b.known(self.t, p.ty) {
                found[ai] = self.as_if_before(before, |this| this.check_hinted(&b, value, p));
            } else if !matches!(value.kind, ExprKind::Closure(_)) {
                later.push((ai, before));
            } else if let Some(ty) = self.as_if_before(before, |this| this.infer_closure(&b, value, p)) {
                (found[ai], quiet) = (ty, quiet || ty == Ty::Error);
                if let Err(c) = b.meet(self.t, p.ty, ty, false, Some(ai)) {
                    self.report_conflict(callee, &b, c, args, &param_of);
                    return Err(Reported);
                }
            } else {
                // Its parameter types are still open (P10).
                self.cant_tell(callee, &b);
                return Err(Reported);
            }
        }
        if b.first_open().is_some() {
            if !quiet {
                self.cant_tell(callee, &b);
            }
            return Err(Reported);
        }
        self.check_worked_out(callee, &b, args)?;
        for (ai, before) in later {
            let p = &callee.params[param_of[ai].expect("bound")];
            found[ai] = self.as_if_before(before, |this| this.check_hinted(&b, &args[ai].value, p));
        }
        Ok((b.list(self.t), found))
    }

    /// Whether the argument `value` for `p` waits for step 3: it is `deferred`, or it is a closure
    /// literal whose result is still open (closure results come after the expected type, G7).
    fn waits(&self, b: &Bindings, value: &Expr, p: &ParamSig) -> bool {
        if p.is_var {
            return false;
        }
        let open_result = matches!(p.ty, Ty::Fn(s) if !b.known(self.t, self.t.sig(s).ret));
        deferred(value) || (open_result && matches!(value.kind, ExprKind::Closure(_)))
    }

    /// `check` run with the moves as they were at `before` (a deferred argument's place in source
    /// order), then the moves put back as they are now. A deferred argument moves nothing itself.
    fn as_if_before<R>(&mut self, before: Option<Moved>, check: impl FnOnce(&mut Self) -> R) -> R {
        let Some(before) = before else { return check(self) };
        let now = std::mem::replace(&mut self.moved, before);
        let r = check(self);
        self.moved = now;
        r
    }

    /// Step 1 for one argument.
    fn infer_arg(&mut self, b: &Bindings, value: &Expr, p: &ParamSig) -> Ty {
        // A `var` argument is a place, checked as written and matched exactly.
        if p.is_var {
            return self.check_expr(value, None);
        }
        if matches!(value.kind, ExprKind::Closure(_)) {
            // Every parameter type is written and the result known (others wait), so it can
            // always be checked.
            return self.infer_closure(b, value, p).unwrap_or(Ty::Error);
        }
        if b.known(self.t, p.ty) {
            return self.check_hinted(b, value, p);
        }
        let ty = self.check_expr(value, None);
        // P11: an array literal given for an open list is that list.
        if let (ExprKind::ArrayLit(_), Ty::List(_), Ty::Array(x)) = (&value.kind, p.ty, ty) {
            let list = Ty::List(x);
            self.t.expr_ty[value.id.0 as usize] = list;
            return list;
        }
        ty
    }

    /// `value` checked with its parameter's type, which is known, filled in as the hint.
    fn check_hinted(&mut self, b: &Bindings, value: &Expr, p: &ParamSig) -> Ty {
        let hint = b.fill_in(self.t, p.ty);
        if matches!(value.kind, ExprKind::Closure(_)) {
            self.arg_name = Some(p.name.clone());
        }
        self.check_expr(value, Some(hint))
    }

    /// A closure literal given for `p`: checked with the function type `p` gives where its types
    /// are known, its result worked out from the body when that is open. `None` when one of its
    /// untyped parameters has an open type (nothing can be told then, P10).
    fn infer_closure(&mut self, b: &Bindings, value: &Expr, p: &ParamSig) -> Option<Ty> {
        let ExprKind::Closure(lit) = &value.kind else { unreachable!("a closure literal") };
        if b.known(self.t, p.ty) {
            return Some(self.check_hinted(b, value, p));
        }
        let Ty::Fn(sig) = p.ty else {
            // A closure for an open `T` is a value like any other, if its parameters say their types.
            if lit.params.iter().any(|q| q.ty.is_none()) {
                return None;
            }
            return Some(self.check_expr(value, None));
        };
        let sig = self.t.sig(sig).clone();
        if sig.params.len() != lit.params.len() {
            // Wrong whatever the placeholders are: said with the parameter's own type.
            self.arg_name = Some(p.name.clone());
            return Some(self.check_expr(value, Some(p.ty)));
        }
        let mut params = Vec::with_capacity(sig.params.len());
        for (q, sp) in lit.params.iter().zip(&sig.params) {
            let ty = if b.known(self.t, sp.ty) {
                b.fill_in(self.t, sp.ty)
            } else if q.ty.is_some() {
                // Its own written type is used; what stands here doesn't matter.
                Ty::Error
            } else {
                return None;
            };
            params.push(SigParam { ty, is_var: sp.is_var });
        }
        let open = !b.known(self.t, sig.ret);
        let ret = if open { Ty::Error } else { b.fill_in(self.t, sig.ret) };
        let want = Ty::Fn(self.t.intern_sig(Sig { params, ret, is_pure: sig.is_pure }));
        self.arg_name = Some(p.name.clone());
        // (As for `map`: the result type comes from the body.)
        self.infer_result = open;
        Some(self.check_expr(value, Some(want)))
    }

    /// Step 4's rules: each decided type meets R3 and the placeholder's rules (R7), as a written
    /// one does, reported at the argument that decided it, else at the use (G20).
    fn check_worked_out(&mut self, callee: &Callee, b: &Bindings, args: &[Arg]) -> Result<(), Reported> {
        let own = self.own.filter(|o| !matches!(o, GenericOwner::Fn(_)));
        let scope = TypeScope { params: &self.type_scope, own };
        let mut refused = Vec::new();
        for (slot, &param) in b.slots.iter().zip(&b.params) {
            let bind = slot.expect("every placeholder is decided");
            let at = bind.from.map_or(callee.at, |ai| args[ai].value.span);
            if let Some(msg) = self.t.check_type_arg(bind.ty, param, callee.name, at, &scope) {
                refused.push((at, msg));
            }
        }
        if refused.is_empty() {
            return Ok(());
        }
        for (at, msg) in refused {
            self.error(at, msg);
        }
        Err(Reported)
    }

    /// "can't tell what `T` is in `best`; write it, like `best[int](...)`", for the first open
    /// placeholder, the example writing the decided ones and `int` for the rest (G26).
    fn cant_tell(&mut self, callee: &Callee, b: &Bindings) {
        let open = b.first_open().expect("a placeholder is open");
        let p = self.t.param_name(b.params[open]).to_string();
        let example = self.example(callee, b, None);
        self.error(callee.at, format!("can't tell what `{p}` is in `{}`; write it, like `{example}`", callee.name));
    }

    /// "`T` in `pick` can't be both `int` (from `a`) and `string` (from `b`)", at the later value.
    fn report_conflict(&mut self, callee: &Callee, b: &Bindings, c: Conflict, args: &[Arg], param_of: &[Option<usize>]) {
        let side = |this: &Self, bind: Binding| {
            let (from, exact) = match bind.from {
                Some(ai) => (format!("from `{}`", callee.params[param_of[ai].expect("bound")].name), bind.exact),
                None => ("from the expected type".to_string(), false),
            };
            let exact = if exact { ", which must match exactly" } else { "" };
            format!("`{}` ({from}{exact})", this.tn(bind.ty))
        };
        let msg = format!("`{}` in `{}` can't be both {} and {}", self.t.param_name(c.param), callee.name, side(self, c.old), side(self, c.new));
        let at = c.new.from.map_or(callee.at, |ai| args[ai].value.span);
        if c.old.exact || c.new.exact {
            let index = b.params.iter().position(|&p| p == c.param).expect("a placeholder of the callee");
            let example = self.example(callee, b, Some((index, c.new.ty)));
            self.error_help(at, msg, format!("write the type, like `{example}`"));
        } else {
            self.error(at, msg);
        }
    }

    /// The use with every type written: the decided ones (with `replace` put in), `int` for the rest.
    fn example(&self, callee: &Callee, b: &Bindings, replace: Option<(usize, Ty)>) -> String {
        let tys: Vec<String> = (0..b.slots.len())
            .map(|i| match (replace, b.slots[i]) {
                (Some((r, ty)), _) if r == i => self.tn(ty),
                (_, Some(bind)) => self.tn(bind.ty),
                (_, None) => "int".to_string(),
            })
            .collect();
        let types = format!("{}[{}]", callee.name, tys.join(", "));
        match callee.form {
            Form::Call => format!("{types}(...)"),
            // A construction's help for a conflict has arguments to write; the "can't tell" one, none.
            Form::Construct if replace.is_some() => format!("{types}(...)"),
            Form::Construct => format!("{types}()"),
            Form::Unit(v) => format!("{types}.{v}"),
            Form::Variant(v) => format!("{types}.{v}(...)"),
        }
    }
}
