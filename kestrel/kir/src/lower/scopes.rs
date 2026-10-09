//! Scopes: what must happen when control leaves a block (dropping its
//! owning locals, popping an arena it pushed) on every way out of it.

use kestrel_syntax::ast;

use super::helpers::owning;
use super::FnLowerer;
use crate::ir::*;

#[derive(Clone, Default)]
pub(crate) struct Scope {
    /// Owning locals declared here, dropped on the way out.
    owning: Vec<LocalIdx>,
    /// An arena pushed for this scope: `Some(true)` for `arena { }` (freed
    /// at the end), `Some(false)` for `in x { }`.
    arena: Option<bool>,
    /// A loop body: `break` and `continue` leave it.
    is_loop: bool,
}

impl<'a> FnLowerer<'a> {
    /// The statements that leave `s`: drops (newest first), then the arena pop.
    fn cleanup(&mut self, s: &Scope) -> Vec<Stmt> {
        let mut out = Vec::new();
        for &l in s.owning.iter().rev() {
            let ty = self.locals[l.0 as usize].ty;
            out.extend(self.helpers.drop_stmts(Place::local(l), ty, self.t));
        }
        if let Some(free) = s.arena {
            out.push(Stmt::PopArena { free });
        }
        out
    }

    /// Cleanups for jumping out of every scope up to the innermost loop
    /// body (`break`/`continue`), or out of all of them (`return`).
    fn exit_cleanup(&mut self, to_loop: bool) -> Vec<Stmt> {
        let mut out = Vec::new();
        for i in (0..self.scopes.len()).rev() {
            let s = self.scopes[i].clone();
            out.extend(self.cleanup(&s));
            if to_loop && s.is_loop {
                break;
            }
        }
        out
    }

    fn scoped_block(&mut self, block: &ast::Block, arena: Option<bool>, is_loop: bool) -> Vec<Stmt> {
        self.scopes.push(Scope { owning: Vec::new(), arena, is_loop });
        let mut out = Vec::new();
        for s in &block.stmts {
            self.stmt(s, &mut out);
        }
        let s = self.scopes.pop().expect("scopes balance");
        out.extend(self.cleanup(&s));
        out
    }

    pub(crate) fn block_stmts(&mut self, block: &ast::Block) -> Vec<Stmt> {
        self.scoped_block(block, None, false)
    }

    pub(crate) fn loop_block(&mut self, block: &ast::Block) -> Vec<Stmt> {
        self.scoped_block(block, None, true)
    }

    /// A block used as a value: its statements and its tail value, which is
    /// `None` when the block can't finish normally.
    pub(crate) fn block_value(&mut self, block: &ast::Block) -> (Vec<Stmt>, Option<Expr>) {
        self.scopes.push(Scope::default());
        let mut stmts = Vec::new();
        let mut value = None;
        if let Some((last, init)) = block.stmts.split_last() {
            for s in init {
                self.stmt(s, &mut stmts);
            }
            match last {
                ast::Stmt::Expr(e) if !self.leaves_early(e) => value = Some(self.expr(e)),
                other => self.stmt(other, &mut stmts),
            }
        }
        let s = self.scopes.pop().expect("scopes balance");
        let cleanup = self.cleanup(&s);
        if cleanup.is_empty() {
            return (stmts, value);
        }
        // The value may read what's about to be dropped, so compute it first.
        let value = value.map(|v| {
            let v = self.owned(v, false);
            self.spill(v, &mut stmts)
        });
        stmts.extend(cleanup);
        (stmts, value)
    }

    /// Records a local declared in the current scope; an owning one is
    /// dropped when the scope ends.
    pub(crate) fn declared(&mut self, l: LocalIdx) {
        if owning(self.locals[l.0 as usize].ty, self.t) {
            self.scopes.last_mut().expect("inside a scope").owning.push(l);
        }
    }

    /// `arena { }` (no place: a fresh arena, freed at the end) or `in x { }`.
    pub(crate) fn arena_block(&mut self, arena: Option<&ast::Expr>, body: &ast::Block, out: &mut Vec<Stmt>) {
        let place = arena.map(|a| self.place_or_temp(a, out, true));
        let free = place.is_none();
        out.push(Stmt::PushArena(place));
        let inner = self.scoped_block(body, Some(free), false);
        out.extend(inner);
    }

    pub(crate) fn return_stmt(&mut self, value: Option<&ast::Expr>, out: &mut Vec<Stmt>) {
        let mut v = value.map(|v| {
            let v = self.expr(v);
            self.owned(v, true)
        });
        let cleanup = self.exit_cleanup(false);
        if !cleanup.is_empty() {
            v = v.map(|x| self.spill(x, out));
            out.extend(cleanup);
        }
        out.push(Stmt::Return(v));
    }

    pub(crate) fn jump(&mut self, stmt: Stmt, out: &mut Vec<Stmt>) {
        let cleanup = self.exit_cleanup(true);
        out.extend(cleanup);
        out.push(stmt);
    }

    /// `target = value` where the target holds an `Arena`: the old value is
    /// dropped after the new one is computed (it may read the old one).
    pub(crate) fn assign_owning(&mut self, target: &ast::Expr, value: &ast::Expr, out: &mut Vec<Stmt>) {
        let place = self.place_or_temp(target, out, true);
        let v = self.value(value);
        let v = self.spill(v, out);
        let ty = v.ty;
        out.extend(self.helpers.drop_stmts(place.clone(), ty, self.t));
        out.push(Stmt::Assign(place, v));
    }
}
