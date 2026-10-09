//! What an argument changes while it's evaluated. Earlier arguments that
//! were passed by pointer would see those changes (or point into memory
//! the change moved), so such calls are rejected.

use kestrel_syntax::ast::{Arg, Block, Expr, ExprKind, InterpPart, Stmt};

use super::BodyChecker;
use crate::tables::{LocalId, Res};

impl<'a> BodyChecker<'a> {
    /// The variables changed by calls and assignments anywhere inside `e`.
    pub(crate) fn changed_inside(&self, e: &Expr, out: &mut Vec<LocalId>) {
        match &e.kind {
            ExprKind::Call { args, .. } => self.changed_by_args(args, out),
            ExprKind::MethodCall { receiver, args, .. } => {
                let changes_receiver = match self.t.res.get(&e.id) {
                    Some(Res::Fn(f)) => self.t.func(*f).params.first().is_some_and(|p| p.is_var),
                    Some(Res::ListAdd) => true,
                    _ => false,
                };
                if changes_receiver {
                    out.extend(self.place_root(receiver));
                }
                self.changed_inside(receiver, out);
                self.changed_by_args(args, out);
            }
            ExprKind::TypeCall { args, .. } => self.changed_by_args(args, out),
            ExprKind::Unary { expr, .. } => self.changed_inside(expr, out),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.changed_inside(lhs, out);
                self.changed_inside(rhs, out);
            }
            ExprKind::Field { base, .. } => self.changed_inside(base, out),
            ExprKind::Index { base, index } => {
                self.changed_inside(base, out);
                self.changed_inside(index, out);
            }
            ExprKind::ArrayLit(items) => items.iter().for_each(|i| self.changed_inside(i, out)),
            ExprKind::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.changed_inside(x, out);
                    }
                }
            }
            ExprKind::If { cond, then_block, else_block } => {
                self.changed_inside(cond, out);
                self.changed_in_block(then_block, out);
                if let Some(b) = else_block {
                    self.changed_in_block(b, out);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.changed_inside(scrutinee, out);
                for a in arms {
                    self.changed_in_block(&a.body, out);
                }
            }
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::None | ExprKind::Str(_) | ExprKind::Ident(_) => {}
        }
    }

    fn changed_by_args(&self, args: &[Arg], out: &mut Vec<LocalId>) {
        for a in args {
            if a.is_var {
                out.extend(self.place_root(&a.value));
            }
            self.changed_inside(&a.value, out);
        }
    }

    fn changed_in_block(&self, b: &Block, out: &mut Vec<LocalId>) {
        for s in &b.stmts {
            match s {
                Stmt::Let { value, else_block: Some(b), .. } => {
                    self.changed_inside(value, out);
                    self.changed_in_block(b, out);
                }
                Stmt::Let { value, .. } | Stmt::Expr(value) | Stmt::Return { value: Some(value), .. } => {
                    self.changed_inside(value, out)
                }
                Stmt::Assign { target, value, .. } => {
                    out.extend(self.place_root(target));
                    self.changed_inside(value, out);
                }
                Stmt::While { cond, body, .. } => {
                    self.changed_inside(cond, out);
                    self.changed_in_block(body, out);
                }
                Stmt::ForRange { start, end, body, .. } => {
                    self.changed_inside(start, out);
                    self.changed_inside(end, out);
                    self.changed_in_block(body, out);
                }
                Stmt::ForIn { iter, body, .. } => {
                    self.changed_inside(iter, out);
                    self.changed_in_block(body, out);
                }
                Stmt::Arena { body, .. } => self.changed_in_block(body, out),
                Stmt::In { arena, body, .. } => {
                    self.changed_inside(arena, out);
                    self.changed_in_block(body, out);
                }
                Stmt::Break(_) | Stmt::Continue(_) | Stmt::Return { value: None, .. } => {}
            }
        }
    }

    /// `passed` were passed by pointer (`var`, or a read-only aggregate)
    /// before `later` is evaluated: none of them may be changed by it.
    pub(crate) fn check_changed_later(&mut self, passed: &[&Expr], later: &[&Expr]) {
        let mut changed = Vec::new();
        for e in later {
            self.changed_inside(e, &mut changed);
        }
        for p in passed {
            let Some(root) = self.place_root(p) else { continue };
            if changed.contains(&root) {
                let name = self.t.local(root).name.clone();
                self.error_help(
                    p.span,
                    format!("`{name}` is passed here, but a later argument changes it"),
                    "compute that argument into a variable before the call",
                );
            }
        }
    }
}
