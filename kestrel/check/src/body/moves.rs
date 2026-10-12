//! Moves of owning values (ones that hold an `Arena` or a GPU object): which locals may have
//! been moved at each point of a function. The state follows the code's paths: each branch of
//! an `if` or `match` starts from the state before it, and a loop's body must leave every local
//! declared outside the loop as it found it, unless the path leaves the loop.

use std::collections::HashMap;

use kestrel_syntax::ast::{Block, Expr, ExprKind};
use kestrel_syntax::Span;

use super::regions::tail;
use super::{BodyChecker, Expect};
use crate::tables::{LocalId, Res};
use crate::ty::Ty;

/// Where a local was moved, and whether every path to this point moved it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MoveAt {
    pub(crate) span: Span,
    pub(crate) every: bool,
}

/// The owning locals that may have been moved at a point in the code.
pub(crate) type Moved = HashMap<LocalId, MoveAt>;

/// A loop being checked: the state where it starts, and the state at each of its `break`s.
pub(crate) struct LoopMoves {
    entry: Moved,
    breaks: Vec<Moved>,
}

/// The state where paths meet: a local is moved if some path moved it, and moved on every
/// path only if each one did. `None` when no path arrives.
pub(crate) fn merge(states: Vec<Moved>) -> Option<Moved> {
    let mut states = states.into_iter();
    let mut out = states.next()?;
    for s in states {
        for (l, at) in out.iter_mut() {
            if !s.get(l).is_some_and(|m| m.every) {
                at.every = false;
            }
        }
        for (l, at) in s {
            out.entry(l).or_insert(MoveAt { every: false, ..at });
        }
    }
    Some(out)
}

impl<'a> BodyChecker<'a> {
    /// The state after branches that each started from `before`, given each branch's type and
    /// its state at its end: the branches that can finish (not `Never`) meet here.
    pub(crate) fn join(&mut self, before: Moved, ends: Vec<(Ty, Moved)>) {
        let finishing = ends.into_iter().filter(|(ty, _)| *ty != Ty::Never).map(|(_, m)| m).collect();
        self.moved = merge(finishing).unwrap_or(before);
    }

    /// Checks `e`, which runs only sometimes (the right side of `and` / `or`): what it moves
    /// may have been moved afterwards.
    pub(crate) fn check_sometimes(&mut self, e: &Expr, want: Option<Ty>) -> Ty {
        let before = self.moved.clone();
        let ty = self.check_value(e, want);
        let after = std::mem::take(&mut self.moved);
        self.moved = merge(vec![before, after]).expect("two states");
        ty
    }

    /// Checks a loop's body, which runs any number of times. `entry` is the state where the
    /// loop starts (before a `while` loop's condition, which runs each time around): a local
    /// moved since then and not given a value again is moved inside the loop. `exit_at_start`
    /// is the state where the loop can end without a `break` (a `while` condition false, the
    /// state after the condition ran), or `None` when only a `break` leaves. Returns whether a
    /// `break` leaves the loop.
    pub(crate) fn loop_body(&mut self, entry: Moved, exit_at_start: Option<Moved>, body: &Block) -> bool {
        self.loop_breaks.push(false);
        self.loop_moves.push(LoopMoves { entry, breaks: Vec::new() });
        if self.check_block(body, Expect::Discard) != Ty::Never {
            self.back_edge();
        }
        let broke = self.loop_breaks.pop().expect("pushed above");
        let frame = self.loop_moves.pop().expect("pushed above");
        let mut exits = frame.breaks;
        exits.extend(exit_at_start);
        if let Some(after) = merge(exits) {
            self.moved = after;
        }
        broke
    }

    /// A `break`: the innermost loop ends here, with the current state.
    pub(crate) fn note_break(&mut self) {
        let now = self.moved.clone();
        if let Some(frame) = self.loop_moves.last_mut() {
            frame.breaks.push(now);
        }
    }

    /// The end of a loop's body, or a `continue`: the body runs again, so every local declared
    /// outside the loop that the loop moved must have a value again.
    pub(crate) fn back_edge(&mut self) {
        let Some(frame) = self.loop_moves.last() else { return };
        let depth = self.loop_breaks.len();
        let mut gone: Vec<(LocalId, Span)> = self
            .moved
            .iter()
            .filter(|(l, _)| !frame.entry.contains_key(l))
            .filter(|(l, _)| self.local_ctx.get(l).is_some_and(|&(_, loops)| loops < depth))
            .map(|(&l, at)| (l, at.span))
            .collect();
        gone.sort_by_key(|&(_, span)| span.start);
        for (l, span) in gone {
            // An inner loop may have reported this move already.
            if !self.loop_move_errors.insert(span) {
                continue;
            }
            let name = self.t.local(l).name.clone();
            self.error_help(
                span,
                format!("`{name}` is moved inside a loop, so the next time around it would already be gone"),
                "declare it inside the loop, give it a new value before the loop goes around, or leave the loop (`break` or `return`) right after moving it",
            );
        }
    }

    /// `v` is stored somewhere new. An owning value (one holding an `Arena` or a GPU object)
    /// can't be copied, only moved, and only whole variables move.
    pub(crate) fn consume(&mut self, v: &Expr) {
        self.consume_on(v, true);
    }

    /// `consume`, where `every` says whether every path through the enclosing `if` / `match`
    /// values reaches `v`.
    fn consume_on(&mut self, v: &Expr, every: bool) {
        let ty = self.t.ty(v.id);
        if !self.t.is_owning(ty) {
            return;
        }
        match &v.kind {
            ExprKind::If { then_block, else_block, .. } => {
                let tails: Vec<&Expr> = std::iter::once(then_block).chain(else_block.as_ref()).filter_map(tail).collect();
                let one = tails.len() == 1;
                for t in tails {
                    self.consume_on(t, every && one);
                }
            }
            ExprKind::Match { arms, .. } => {
                let tails: Vec<&Expr> = arms.iter().filter_map(|a| tail(&a.body)).collect();
                let one = tails.len() == 1;
                for t in tails {
                    self.consume_on(t, every && one);
                }
            }
            ExprKind::Ident(_) => {
                let Some(&Res::Local(l)) = self.t.res.get(&v.id) else { return };
                let info = self.t.local(l);
                let name = info.name.clone();
                if info.is_param {
                    let what = self.t.owns_what(ty);
                    let msg = format!("can't move parameter `{name}`: it belongs to the caller, and it holds {what}, so it can't be copied");
                    self.error(v.span, msg);
                } else if let Some(g) = self.let_guards.iter().find(|g| g.view == l).cloned() {
                    let root = self.t.local(g.root);
                    let inside = if root.is_param {
                        format!("parameter `{}`, which belongs to the caller", root.name)
                    } else {
                        format!("`{}`", root.name)
                    };
                    let what = self.t.owns_what(ty);
                    self.error(v.span, format!("can't move `{name}`: it's inside {inside}, and it holds {what}, so it can't be copied"));
                } else if self.views.contains(&l) {
                    let what = self.t.owns_what(ty);
                    let msg = format!("can't move `{name}` out of its collection: it holds {what}, so it can't be copied");
                    self.error(v.span, msg);
                } else if let Some(g) = self.match_guards.iter().find(|g| g.root == l).cloned() {
                    let binding = self.t.local(g.binding).name.clone();
                    self.error_help(
                        v.span,
                        format!("can't move `{name}` here: this `match` arm's `{binding}` reads from it"),
                        format!("move it after the `match`, or copy what you need out of `{binding}` first"),
                    );
                } else if let Some(g) = self.let_guards.iter().find(|g| g.root == l).cloned() {
                    let view = self.t.local(g.view).name.clone();
                    self.error_help(v.span, format!("can't move `{name}` here: `{view}` reads from it"), format!("move it after the block that declares `{view}`"));
                } else {
                    self.moved.insert(l, MoveAt { span: v.span, every });
                    self.t.moves.insert(v.id);
                }
            }
            // A part of a variable, or of a temporary (a call's result, a construction). `Enum.Variant`
            // is a field access that builds a value; it isn't a part of anything.
            ExprKind::Field { .. } | ExprKind::Index { .. } if !matches!(self.t.res.get(&v.id), Some(Res::Variant(..))) => {
                let what = self.t.owns_what(ty);
                let msg = format!("can't move this out: it holds {what}, so it can't be copied, and only whole variables move");
                let help = match self.place_root(v) {
                    Some(_) => "move the whole variable that holds it",
                    None => "use the part in place, or keep the whole value in a variable",
                };
                self.error_help(v.span, msg, help);
            }
            _ => {}
        }
    }
}
