//! Loops, analysed as `while` loops: the state at the top is found by repeating the body to a
//! fixed point, widening the ranges that still grow; then one more pass, with that state, decides
//! the checks. A check is decided only in the final pass of every loop around it, so an inner
//! loop skips its deciding pass unless every loop around it is deciding.

use super::interval::Interval;
use super::state::State;
use super::{writes, Cx};
use crate::ir::*;

impl Cx<'_> {
    pub(super) fn while_loop(&mut self, entry: State, cond: &Expr, body: &[Stmt]) -> State {
        self.repeat(entry, cond.line, |cx, top| {
            let (t, f) = cx.condition(top.clone(), cond);
            let (end, frame) = cx.body(t, body);
            let back = joined(end, frame.continues);
            (back, joined(f, frame.breaks))
        })
    }

    /// `for var from start to end`: `var = start; end = <the end, worked out once>; while var <
    /// end { body; var = var + 1 }`, where the step wraps (as codegen emits it) and a `continue`
    /// goes through it.
    pub(super) fn for_range(&mut self, entry: State, var: LocalIdx, (start, end): (&Expr, &Expr), body: &[Stmt]) -> State {
        // The start and the end, once, as a simple statement that assigns `var`.
        let mut st = entry;
        let mut w = writes::of_expr(start);
        w.extend(writes::of_expr(end));
        self.forget(&mut st, &w);
        let mark = self.learned.len();
        let first = self.eval(&st, start, true);
        let last = self.eval(&st, end, true);
        let learned: Vec<_> = self.learned.drain(mark..).collect();
        for (l, r) in learned {
            st.narrow(&self.t, l, r);
        }
        self.forget(&mut st, &w);
        st.set(&self.t, var, first);
        self.repeat(st, start.line, |cx, top| {
            // `var < end` decides each round; the end was worked out once, so its range holds.
            let (mut t, mut f) = (top.clone(), top.clone());
            t.narrow(&cx.t, var, Interval::new(i128::MIN / 2, last.hi - 1));
            f.narrow(&cx.t, var, Interval::new(last.lo, i128::MAX / 2));
            let (end, frame) = cx.body(t, body);
            let back = cx.step(joined(end, frame.continues), var);
            (back, joined(f, frame.breaks))
        })
    }

    /// `var = var + 1`, wrapping: a `for` range's step.
    fn step(&self, mut st: State, var: LocalIdx) -> State {
        if let Some(v) = st.get(&self.t, var) {
            let ty = self.f.locals[var.0 as usize].ty;
            let next = Interval::new(v.lo + 1, v.hi + 1);
            if next.fits(ty) {
                st.set(&self.t, var, next);
            } else {
                st.forget(&self.t, var);
            }
        }
        st
    }

    /// Repeats `round` (which runs the condition and the body from the state at the top, giving
    /// the state going back to the top and the state leaving the loop) to a fixed point, then once
    /// more to decide the checks when every loop around is deciding. The state after the loop.
    fn repeat(&mut self, entry: State, line: u32, round: impl Fn(&mut Self, &State) -> (State, State)) -> State {
        if entry.is_dead() {
            // Never reached: in a deciding pass its checks are listed as kept.
            if self.deciding && !self.gave_up {
                self.keep += 1;
                let top = self.t.top();
                self.loop_lines.push(line);
                let _ = round(self, &top);
                self.loop_lines.pop();
                self.keep -= 1;
            }
            return entry;
        }
        self.loop_lines.push(line);
        let outer = self.deciding;
        self.deciding = false;
        let mut top = entry.clone();
        let mut exit = loop {
            let (back, exit) = round(self, &top);
            if self.gave_up {
                break exit;
            }
            let next = top.widen(&self.t, &entry.join(&back));
            if next == top {
                break exit;
            }
            top = next;
        };
        self.deciding = outer;
        if outer && !self.gave_up {
            exit = round(self, &top).1;
        }
        self.loop_lines.pop();
        exit
    }
}

/// `st` joined with what a frame's slot carried (nothing, when no path took it).
fn joined(st: State, other: Option<State>) -> State {
    match other {
        Some(o) => st.join(&o),
        None => st,
    }
}
