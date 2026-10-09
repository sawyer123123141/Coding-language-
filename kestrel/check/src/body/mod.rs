mod call;
mod enums;
mod expr;
mod matching;
mod place;
mod structs;

pub(crate) use structs::check_field_defaults;

use std::collections::HashMap;

use kestrel_syntax::ast::{Binding, Block, Expr, ExprKind, Item, Program, Stmt};
use kestrel_syntax::{Diagnostic, Span};

use crate::collect::resolve_type;
use crate::tables::{ConstId, FnId, LocalId, LocalInfo, Tables};
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
    loop_depth: u32,
    ret: Ty,
    is_pure: bool,
}

pub(crate) fn check_consts(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let consts = program.items.iter().filter_map(|it| match it {
        Item::Const(c) => Some(c),
        Item::Fn(_) | Item::Struct(_) | Item::Enum(_) => None,
    });
    for (i, c) in consts.enumerate() {
        let mut bc = BodyChecker::new(t, diags, None, Ty::Unit, false);
        let want = c.ty.as_ref().map(|ty| resolve_type(ty, bc.t, bc.diags));
        let ty = if is_const_expr(&c.value) {
            bc.check_value(&c.value, want)
        } else {
            bc.error(c.value.span, "constants can only use literals, operators and earlier constants");
            Ty::Error
        };
        t.consts[i].ty = ty;
        t.const_by_name.insert(c.bind.name.clone(), ConstId(i as u32));
    }
}

pub(crate) fn is_const_expr(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Ident(_) => true,
        ExprKind::Unary { expr, .. } => is_const_expr(expr),
        ExprKind::Binary { lhs, rhs, .. } => is_const_expr(lhs) && is_const_expr(rhs),
        ExprKind::Str(_)
        | ExprKind::Call { .. }
        | ExprKind::If { .. }
        | ExprKind::Field { .. }
        | ExprKind::MethodCall { .. }
        | ExprKind::Match { .. } => false,
    }
}

pub(crate) fn check_fn(program: &Program, fn_id: FnId, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let info = &t.fns[fn_id.0 as usize];
    let decl = info.decl(program);
    let (ret, is_pure, params) = (info.ret, info.is_pure, info.params.clone());
    let mut bc = BodyChecker::new(t, diags, Some(fn_id), ret, is_pure);
    bc.scopes.push(HashMap::new());
    for (p, sig) in decl.params.iter().zip(&params) {
        bc.declare(&p.bind, sig.ty, p.is_var, true, p.is_var);
    }
    let expect = if ret == Ty::Unit { Expect::Discard } else { Expect::Value(Some(ret)) };
    let body = bc.check_block(&decl.body, expect);
    // An unresolved return type was already reported; don't pile on.
    if !matches!(ret, Ty::Unit | Ty::Error) && body == Ty::Unit {
        let msg = format!("`{}` must return {}, but its body can finish without a value", decl.name, bc.tn(ret));
        bc.diags.push(Diagnostic::error(decl.name_span, msg).with_help("end the body with a value or a `return`"));
    }
}

impl<'a> BodyChecker<'a> {
    fn new(t: &'a mut Tables, diags: &'a mut Vec<Diagnostic>, fn_id: Option<FnId>, ret: Ty, is_pure: bool) -> Self {
        BodyChecker { t, diags, fn_id, scopes: Vec::new(), loop_depth: 0, ret, is_pure }
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

    fn declare(&mut self, bind: &Binding, ty: Ty, mutable: bool, is_param: bool, is_var_param: bool) -> LocalId {
        let name = &bind.name;
        let taken = self.lookup_local(name).is_some()
            || self.t.const_by_name.contains_key(name)
            || self.t.fn_by_name.contains_key(name)
            || self.t.struct_by_name.contains_key(name)
            || self.t.enum_by_name.contains_key(name);
        if taken {
            self.error_help(
                bind.span,
                format!("`{name}` is already defined; Kestrel doesn't allow shadowing"),
                "pick a different name",
            );
        }
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
        let msg = format!("expected {}, found {}", self.tn(to), self.tn(from));
        if from.is_numeric() && to.is_numeric() {
            self.error_help(e.span, msg, format!("convert it explicitly: `{}(...)`", to.name()));
        } else {
            self.error(e.span, msg);
        }
    }

    pub(crate) fn check_block(&mut self, block: &Block, expect: Expect) -> Ty {
        self.scopes.push(HashMap::new());
        let mut ty = Ty::Unit;
        let last = block.stmts.len().saturating_sub(1);
        for (i, stmt) in block.stmts.iter().enumerate() {
            ty = match (stmt, expect) {
                (Stmt::Expr(e), Expect::Value(want)) if i == last => self.check_tail(e, want),
                _ => self.check_stmt(stmt),
            };
        }
        self.scopes.pop();
        ty
    }

    /// Checks a statement; returns `Never` when control can't continue past it.
    fn check_stmt(&mut self, s: &Stmt) -> Ty {
        match s {
            Stmt::Let { is_var, bind, ty, value, .. } => {
                let ann = ty.as_ref().map(|t| resolve_type(t, self.t, self.diags));
                let mut local_ty = self.check_value(value, ann);
                if matches!(local_ty, Ty::Unit | Ty::Never) {
                    self.error(value.span, "this expression has no value to store");
                    local_ty = Ty::Error;
                }
                self.declare(bind, local_ty, *is_var, false, false);
                Ty::Unit
            }
            Stmt::Assign { target, op, value, .. } => {
                self.check_assign(target, *op, value);
                Ty::Unit
            }
            Stmt::While { cond, body, .. } => {
                self.check_value(cond, Some(Ty::Bool));
                self.loop_depth += 1;
                self.check_block(body, Expect::Discard);
                self.loop_depth -= 1;
                Ty::Unit
            }
            Stmt::ForRange { var, start, end, body, .. } => {
                let st = self.check_expr(start, None);
                let st = self.reject_never(start, st);
                let et = self.check_expr(end, if st.is_int() { Some(st) } else { None });
                let et = self.reject_never(end, et);
                let var_ty = match Ty::common(st, et) {
                    _ if st == Ty::Error || et == Ty::Error => Ty::Error,
                    Some(t) if t.is_int() => {
                        self.coerce(start, st, t);
                        self.coerce(end, et, t);
                        t
                    }
                    _ => {
                        let msg = format!("`for ... from ... to` needs integers, found {} and {}", self.tn(st), self.tn(et));
                        self.error(start.span.to(end.span), msg);
                        Ty::Error
                    }
                };
                self.scopes.push(HashMap::new());
                self.declare(var, var_ty, false, false, false);
                self.loop_depth += 1;
                self.check_block(body, Expect::Discard);
                self.loop_depth -= 1;
                self.scopes.pop();
                Ty::Unit
            }
            Stmt::Break(span) | Stmt::Continue(span) => {
                if self.loop_depth == 0 {
                    let word = if matches!(s, Stmt::Break(_)) { "break" } else { "continue" };
                    self.error(*span, format!("`{word}` outside a loop"));
                }
                Ty::Never
            }
            Stmt::Return { value, span } => {
                match (value, self.ret) {
                    (Some(v), Ty::Unit) => {
                        self.check_expr(v, None);
                        self.error(v.span, "this function doesn't return a value");
                    }
                    (Some(v), r) => {
                        self.check_value(v, Some(r));
                    }
                    (None, Ty::Unit | Ty::Error) => {}
                    (None, r) => {
                        let msg = format!("this function must return {}", self.tn(r));
                        self.error(*span, msg);
                    }
                }
                Ty::Never
            }
            Stmt::Expr(e) => self.check_discarded(e),
        }
    }

    /// An expression whose value nobody uses.
    fn check_discarded(&mut self, e: &Expr) -> Ty {
        let ty = match &e.kind {
            ExprKind::If { .. } => self.check_if(e, Expect::Discard),
            ExprKind::Match { .. } => self.check_match(e, Expect::Discard),
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } => self.check_discarded_call(e),
            _ => {
                let ty = self.check_expr(e, None);
                if ty != Ty::Error {
                    self.error(e.span, "this line has no effect; did you mean to continue the line above?");
                }
                ty
            }
        };
        if ty == Ty::Never {
            Ty::Never
        } else {
            Ty::Unit
        }
    }
}
