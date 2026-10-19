//! What an argument changes while it's evaluated. Earlier arguments that
//! were passed by pointer would see those changes (or point into memory
//! the change moved), so such calls are rejected. The same walk gives the
//! call exclusivity rule (`exclusive`) the places changed and the values
//! moved inside a call's arguments.

use kestrel_syntax::ast::{Arg, Block, Expr, ExprKind, InterpPart, Stmt};

use super::exclusive::Changes;
use super::regions::PathKey;
use super::BodyChecker;
use crate::tables::{LocalId, Res};

impl<'a> BodyChecker<'a> {
    /// The variables changed by calls and assignments anywhere inside `e`.
    pub(crate) fn changed_inside(&self, e: &Expr, out: &mut Vec<LocalId>) {
        let mut changes = Changes::default();
        self.changes_in(e, &mut changes);
        out.extend(changes.paths.iter().map(|p| p.root));
    }

    /// The places changed by calls and assignments anywhere inside `e`, and the values moved
    /// there (see `Changes`).
    pub(crate) fn changes_in(&self, e: &Expr, out: &mut Changes) {
        match &e.kind {
            ExprKind::Call { args, .. } => self.changes_by_args(args, out),
            ExprKind::MethodCall { receiver, args, .. } => {
                let res = self.t.res.get(&e.id);
                // A user method, or an interface method called through an interface value.
                let changes_receiver = match (self.t.callee(res), res) {
                    (Some(f), _) => self.t.func(f).params.first().is_some_and(|p| p.is_var),
                    (None, Some(Res::ListAdd | Res::MapRemove)) => true,
                    (None, Some(Res::List(m))) => m.changes_receiver(),
                    _ => false,
                };
                if changes_receiver {
                    self.note_changed(receiver, out);
                }
                self.changes_in(receiver, out);
                self.changes_by_args(args, out);
            }
            ExprKind::TypeCall { args, .. } => self.changes_by_args(args, out),
            ExprKind::CallValue { callee, args } => {
                self.changes_in(callee, out);
                self.changes_by_args(args, out);
            }
            ExprKind::Unary { expr, .. } => self.changes_in(expr, out),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.changes_in(lhs, out);
                self.changes_in(rhs, out);
            }
            ExprKind::Field { base, .. } => self.changes_in(base, out),
            ExprKind::Index { base, index } => {
                self.changes_in(base, out);
                self.changes_in(index, out);
            }
            ExprKind::ArrayLit(items) => items.iter().for_each(|i| self.changes_in(i, out)),
            ExprKind::MapLit(entries) => {
                for (k, v) in entries {
                    self.changes_in(k, out);
                    self.changes_in(v, out);
                }
            }
            ExprKind::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.changes_in(x, out);
                    }
                }
            }
            ExprKind::If { cond, then_block, else_block } => {
                self.changes_in(cond, out);
                self.changes_in_block(then_block, out);
                if let Some(b) = else_block {
                    self.changes_in_block(b, out);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.changes_in(scrutinee, out);
                for a in arms {
                    self.changes_in_block(&a.body, out);
                }
            }
            // A move of an owning variable (into a `let`, a construction, a list...).
            ExprKind::Ident(_) if self.t.moves.contains(&e.id) => out.moved_roots.extend(self.place_root(e)),
            // Building a closure runs none of its body, and it changes nothing it captures.
            ExprKind::Closure(_) => {}
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::None | ExprKind::Str(_) | ExprKind::Ident(_) => {}
        }
    }

    /// What a call's arguments change: its `var` arguments, and what is changed or moved inside
    /// each argument. An owning value passed to a parameter that isn't `var` counts as moved into
    /// the call (see `exclusive`).
    pub(crate) fn changes_by_args(&self, args: &[Arg], out: &mut Changes) {
        for a in args {
            if a.is_var {
                self.note_changed(&a.value, out);
            } else {
                self.note_passed(&a.value, out);
            }
            self.changes_in(&a.value, out);
        }
    }

    fn changes_in_block(&self, b: &Block, out: &mut Changes) {
        for s in &b.stmts {
            match s {
                Stmt::Let { value, else_block: Some(b), .. } => {
                    self.changes_in(value, out);
                    self.changes_in_block(b, out);
                }
                Stmt::Let { value, .. } | Stmt::Expr(value) | Stmt::Return { value: Some(value), .. } => self.changes_in(value, out),
                Stmt::Assign { target, value, .. } => {
                    self.note_changed(target, out);
                    self.changes_in(value, out);
                }
                Stmt::While { cond, body, .. } => {
                    self.changes_in(cond, out);
                    self.changes_in_block(body, out);
                }
                Stmt::ForRange { start, end, body, .. } => {
                    self.changes_in(start, out);
                    self.changes_in(end, out);
                    self.changes_in_block(body, out);
                }
                Stmt::ForIn { iter, body, .. } => {
                    self.changes_in(iter, out);
                    self.changes_in_block(body, out);
                }
                Stmt::Arena { body, .. } => self.changes_in_block(body, out),
                Stmt::In { arena, body, .. } => {
                    self.changes_in(arena, out);
                    self.changes_in_block(body, out);
                }
                Stmt::Break(_) | Stmt::Continue(_) | Stmt::Return { value: None, .. } => {}
                // Removing the loop's item changes its list: all of the list's variable, since the
                // loop it belongs to has been left.
                Stmt::Remove { span, .. } => {
                    if let Some(root) = self.t.removes.get(span).and_then(|&l| self.t.local(l).alias) {
                        out.paths.push(PathKey { root, steps: Vec::new() });
                    }
                }
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
