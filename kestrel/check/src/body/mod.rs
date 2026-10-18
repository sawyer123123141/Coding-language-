mod args;
mod builtins;
mod call;
mod collections;
mod consts;
mod default_cycles;
mod enums;
mod expr;
mod fnvalue;
mod let_else;
mod in_block;
mod lists;
mod maps;
mod match_guard;
mod matching;
mod moves;
mod nested;
mod par;
mod ops;
mod place;
mod pop_owner;
mod regions;
mod remove;
mod stmt;
mod strings;
mod structs;

pub(crate) use consts::{check_consts, is_const_expr};
pub(crate) use default_cycles::check_default_cycles;
pub(crate) use structs::check_field_defaults;

use std::collections::{HashMap, HashSet};

use kestrel_syntax::ast::{Binding, Expr, ExprId, Program, Stmt};
use kestrel_syntax::{Diagnostic, Span};

use crate::sig::sig_accepts;
use crate::tables::{FnId, LocalId, LocalInfo, Tables};
use crate::ty::Ty;

/// What the surrounding code does with a block's or expression's value.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Expect {
    Discard,
    Value(Option<Ty>),
}

pub(crate) struct BodyChecker<'a> {
    t: &'a mut Tables,
    diags: &'a mut Vec<Diagnostic>,
    /// `None` while checking a constant initializer.
    fn_id: Option<FnId>,
    scopes: Vec<HashMap<String, LocalId>>,
    /// One entry per enclosing loop: whether a `break` targets it.
    loop_breaks: Vec<bool>,
    ret: Ty,
    is_pure: bool,
    /// Allocation contexts, outermost (the function's own) first.
    ctxs: Vec<regions::Ctx>,
    /// Per local: the context and loop depth it was declared in.
    local_ctx: HashMap<LocalId, (usize, usize)>,
    /// Views, which read a place without copying it: `for ... in` elements, map loop values and
    /// `let ... else` unwraps (`match` bindings are tracked by `match_guards` instead).
    views: HashSet<LocalId>,
    /// Owning locals that may have been moved by now, following the code's paths (see `moves`).
    moved: moves::Moved,
    /// The loops being checked, outermost first: where each started, and its `break`s.
    loop_moves: Vec<moves::LoopMoves>,
    /// Moves already reported as moved inside a loop, so an enclosing loop doesn't repeat them.
    loop_move_errors: HashSet<Span>,
    /// `match` arms whose bindings are reading a place right now.
    match_guards: Vec<regions::MatchGuard>,
    /// `let ... else` views in scope: nothing may move what they read.
    let_guards: Vec<let_else::LetGuard>,
    /// Enclosing `par for` bodies, outermost first.
    par: Vec<par::ParBody>,
    /// Enclosing `for ... in` loops over lists and arrays that are places, outermost first.
    for_in_lists: Vec<lists::LoopList>,
    /// Enclosing `for ... in` loops, for `remove`.
    removing: remove::Removing,
    /// The call statement being checked, whose result is thrown away.
    discarded: Option<ExprId>,
}

pub(crate) fn check_fn(program: &Program, fn_id: FnId, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let info = &t.fns[fn_id.0 as usize];
    let decl = info.decl(program);
    let (ret, is_pure, params) = (info.ret, info.is_pure, info.params.clone());
    let is_native = info.is_native;
    let mut bc = BodyChecker::new(t, diags, Some(fn_id), ret, is_pure);
    bc.scopes.push(HashMap::new());
    // A native has no body; its parameters are declared so calls and codegen see them, but
    // they are only labels, so their names aren't checked (see `link`).
    let declare = if is_native { BodyChecker::add_local } else { BodyChecker::declare };
    for (p, sig) in decl.params.iter().zip(&params) {
        declare(&mut bc, &p.bind, sig.ty, p.is_var, true, p.is_var);
    }
    if is_native {
        return;
    }
    let expect = if ret == Ty::Unit { Expect::Discard } else { Expect::Value(Some(ret)) };
    let body = bc.check_block(&decl.body, expect);
    if let (Some(Stmt::Expr(tail)), false) = (decl.body.stmts.last(), ret == Ty::Unit) {
        bc.consume(tail);
        bc.check_return_region(tail);
    }
    // An unresolved return type was already reported; don't pile on.
    if !matches!(ret, Ty::Unit | Ty::Error) && body == Ty::Unit {
        let msg = format!("`{}` must return {}, but its body can finish without a value", decl.name, bc.tn(ret));
        bc.diags.push(Diagnostic::error(decl.name_span, msg).with_help("end the body with a value or a `return`"));
    }
}

impl<'a> BodyChecker<'a> {
    fn new(t: &'a mut Tables, diags: &'a mut Vec<Diagnostic>, fn_id: Option<FnId>, ret: Ty, is_pure: bool) -> Self {
        let ctxs = vec![regions::Ctx::Function];
        let (local_ctx, views, moved) = (HashMap::new(), HashSet::new(), HashMap::new());
        let (scopes, loop_breaks, match_guards) = (Vec::new(), Vec::new(), Vec::new());
        let (loop_moves, loop_move_errors) = (Vec::new(), HashSet::new());
        let (par, let_guards, for_in_lists) = (Vec::new(), Vec::new(), Vec::new());
        BodyChecker {
            t,
            diags,
            fn_id,
            scopes,
            loop_breaks,
            ret,
            is_pure,
            ctxs,
            local_ctx,
            views,
            moved,
            loop_moves,
            loop_move_errors,
            match_guards,
            let_guards,
            par,
            for_in_lists,
            removing: remove::Removing::default(),
            discarded: None,
        }
    }

    /// A type as it reads in messages.
    fn tn(&self, ty: Ty) -> String {
        self.t.ty_name(ty)
    }

    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, msg));
    }

    fn error_help(&mut self, span: Span, msg: impl Into<String>, help: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, msg).with_help(help));
    }

    fn lookup_local(&self, name: &str) -> Option<LocalId> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    /// A new local, which can't take a built-in or reserved name or hide another local. (A
    /// clash with one of its module's items or imports is `link`'s to report: here every
    /// module's items are global, and another module's names don't clash.)
    fn declare(&mut self, bind: &Binding, ty: Ty, mutable: bool, is_param: bool, is_var_param: bool) -> LocalId {
        let name = &bind.name;
        if let Some(msg) = crate::collect::reserved_name_error(name, name) {
            self.error(bind.span, msg);
        }
        if self.lookup_local(name).is_some() {
            self.error_help(
                bind.span,
                format!("`{name}` is already defined; Kestrel doesn't allow shadowing"),
                "pick a different name",
            );
        }
        self.add_local(bind, ty, mutable, is_param, is_var_param)
    }

    /// `declare` without the checks on the name.
    fn add_local(&mut self, bind: &Binding, ty: Ty, mutable: bool, is_param: bool, is_var_param: bool) -> LocalId {
        let name = &bind.name;
        let owner = self.fn_id.expect("locals only exist inside functions");
        let id = LocalId(self.t.locals.len() as u32);
        self.t.locals.push(LocalInfo {
            name: name.clone(),
            ty,
            mutable,
            is_param,
            is_var_param,
            addressed: false,
            owner,
            span: bind.span,
            alias: None,
        });
        self.t.fns[owner.0 as usize].locals.push(id);
        self.t.bind_local.insert(bind.id, id);
        self.local_ctx.insert(id, (self.ctxs.len() - 1, self.loop_breaks.len()));
        self.scopes.last_mut().expect("a scope is open").insert(name.clone(), id);
        id
    }

    /// Checks `e` against an expected type, recording a conversion or an error.
    pub(crate) fn check_value(&mut self, e: &Expr, want: Option<Ty>) -> Ty {
        let found = self.check_expr(e, want);
        if found == Ty::Never {
            return self.reject_never(e, found);
        }
        match want {
            Some(w) => {
                self.coerce(e, found, w);
                w
            }
            None => found,
        }
    }

    /// Like `check_value`, for a block's last expression: an `if` whose
    /// branches all leave early is fine there (the block just diverges).
    pub(crate) fn check_tail(&mut self, e: &Expr, want: Option<Ty>) -> Ty {
        let found = self.check_expr(e, want);
        match want {
            Some(w) => {
                self.coerce(e, found, w);
                if found == Ty::Never {
                    Ty::Never
                } else {
                    w
                }
            }
            None => found,
        }
    }

    /// An `if` whose branches all leave early produces nothing to use here.
    pub(crate) fn reject_never(&mut self, e: &Expr, ty: Ty) -> Ty {
        if ty != Ty::Never {
            return ty;
        }
        self.error(e.span, "this `if` always leaves early, so it has no value");
        Ty::Error
    }

    pub(crate) fn coerce(&mut self, e: &Expr, from: Ty, to: Ty) {
        if from == to || matches!(from, Ty::Never | Ty::Error) || to == Ty::Error {
            return;
        }
        if from.coerces_to(to) {
            self.t.coerce.insert(e.id, to);
            return;
        }
        // A pure function value goes where a non-pure one of the same shape is expected; they
        // are the same value, so nothing converts.
        if let (Ty::Fn(got), Ty::Fn(want)) = (from, to) {
            if sig_accepts(self.t, want, got) {
                return;
            }
        }
        // A `T` is a `T?` that has a value.
        if let Ty::Optional(inner) = to {
            if from.coerces_to(self.t.elem(inner)) {
                self.t.coerce.insert(e.id, to);
                return;
            }
        }
        let msg = format!("expected {}, found {}", self.tn(to), self.tn(from));
        if from.is_numeric() && to.is_numeric() {
            self.error_help(e.span, msg, format!("convert it explicitly: `{}(...)`", to.name()));
        } else {
            self.error(e.span, msg);
        }
    }
}
