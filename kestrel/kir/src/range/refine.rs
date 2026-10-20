//! Conditions: what `if c`, `while c`, a value `if`'s condition and the right side of `and` and
//! `or` know. A comparison of two integers narrows the range of a local on either side; `and`,
//! `or` and `not` combine their parts in order; anything else teaches nothing.

use super::interval::Interval;
use super::state::State;
use super::{writes, Cx};
use crate::ir::*;

/// Beyond any value of any type: an open end of a range a comparison gives.
const FAR: i128 = 1 << 100;

impl Cx<'_> {
    /// Refines `st` with `cond` having the value `truth`: the parts of the condition in their
    /// order, each forgetting what it writes once it has been refined with. Its checks are decided
    /// as anywhere else; facts from its passed checks are dropped (the callers that want them,
    /// `condition`, add them themselves). This is the hook Round B's `@assume` calls; `If`, `While`
    /// and a value `if` go through `stmt.rs`'s `condition`, which wraps the same parts in the
    /// spec's condition rule.
    #[allow(dead_code)]
    pub(crate) fn refine(&mut self, st: &mut State, cond: &Expr, truth: bool) {
        let mark = self.learned.len();
        let (t, f) = self.cond_parts(std::mem::replace(st, State::dead()), cond, true);
        self.learned.truncate(mark);
        *st = if truth { t } else { f };
    }

    /// The true and false states of `e` evaluated in `st`, each part forgetting what it writes
    /// once it has been refined with (a later part may change what an earlier one compared).
    /// `sure`: `e` runs whenever its statement does.
    pub(super) fn cond_parts(&mut self, st: State, e: &Expr, sure: bool) -> (State, State) {
        if st.is_dead() {
            self.unreachable_expr(e);
            return (State::dead(), State::dead());
        }
        if !self.enter(false) {
            return (st.clone(), st);
        }
        let out = match &e.kind {
            ExprKind::Bool(true) => (st, State::dead()),
            ExprKind::Bool(false) => (State::dead(), st),
            ExprKind::Unary(UnOp::Not, x) => {
                let (t, f) = self.cond_parts(st, x, sure);
                (f, t)
            }
            ExprKind::And(a, b) => {
                let (at, af) = self.cond_parts(st, a, sure);
                if at.is_dead() {
                    self.unreachable_expr(b);
                    (State::dead(), af)
                } else {
                    let (bt, bf) = self.cond_parts(at, b, false);
                    (bt, af.join(&bf))
                }
            }
            ExprKind::Or(a, b) => {
                let (at, af) = self.cond_parts(st, a, sure);
                if af.is_dead() {
                    self.unreachable_expr(b);
                    (at, State::dead())
                } else {
                    let (bt, bf) = self.cond_parts(af, b, false);
                    (at.join(&bt), bf)
                }
            }
            ExprKind::Binary(op, l, r) if op.is_comparison() && l.ty.is_int() => self.compare(st, e, *op, (l, r), sure),
            _ => {
                self.eval(&st, e, sure);
                let mut st = st;
                self.forget(&mut st, &writes::of_expr(e));
                (st.clone(), st)
            }
        };
        self.leave();
        out
    }

    /// `l op r`: each side that is a tracked local is narrowed by the other side's range.
    fn compare(&mut self, st: State, e: &Expr, op: BinOp, (l, r): (&Expr, &Expr), sure: bool) -> (State, State) {
        let (a, b) = (self.eval(&st, l, sure), self.eval(&st, r, sure));
        let mut t = st.clone();
        let mut f = st;
        self.holds(&mut t, op, (l, a), (r, b));
        self.holds(&mut f, negated(op), (l, a), (r, b));
        let w = writes::of_expr(e);
        self.forget(&mut t, &w);
        self.forget(&mut f, &w);
        (t, f)
    }

    /// Narrows `st` to where `l op r` holds, `a` and `b` being the two sides' ranges.
    fn holds(&self, st: &mut State, op: BinOp, (l, a): (&Expr, Interval), (r, b): (&Expr, Interval)) {
        if !can_hold(op, a, b) {
            *st = State::dead();
            return;
        }
        if let Some(x) = self.local_of(l) {
            if let Some(range) = side(op, a, b) {
                st.narrow(&self.t, x, range);
            }
        }
        if let Some(y) = self.local_of(r) {
            if let Some(range) = side(mirrored(op), b, a) {
                st.narrow(&self.t, y, range);
            }
        }
    }
}

/// `not (a op b)` as a comparison.
fn negated(op: BinOp) -> BinOp {
    match op {
        BinOp::Lt => BinOp::Ge,
        BinOp::Ge => BinOp::Lt,
        BinOp::Le => BinOp::Gt,
        BinOp::Gt => BinOp::Le,
        BinOp::Eq => BinOp::Ne,
        _ => BinOp::Eq,
    }
}

/// `a op b` as `b op' a`.
fn mirrored(op: BinOp) -> BinOp {
    match op {
        BinOp::Lt => BinOp::Gt,
        BinOp::Gt => BinOp::Lt,
        BinOp::Le => BinOp::Ge,
        BinOp::Ge => BinOp::Le,
        other => other,
    }
}

/// Whether `x op y` can hold for some `x` in `a` and `y` in `b`.
fn can_hold(op: BinOp, a: Interval, b: Interval) -> bool {
    match op {
        BinOp::Lt => a.lo < b.hi,
        BinOp::Le => a.lo <= b.hi,
        BinOp::Gt => a.hi > b.lo,
        BinOp::Ge => a.hi >= b.lo,
        BinOp::Eq => a.meet(b).is_some(),
        _ => !(a.lo == a.hi && b.lo == b.hi && a.lo == b.lo),
    }
}

/// The values `x` (now in `a`) can have where `x op y` holds for some `y` in `b`, when that says
/// more than `a`. `!=` narrows only an end (`n != 0` with `n` 0 or more makes `n` 1 or more).
fn side(op: BinOp, a: Interval, b: Interval) -> Option<Interval> {
    Some(match op {
        BinOp::Lt => Interval::new(-FAR, b.hi - 1),
        BinOp::Le => Interval::new(-FAR, b.hi),
        BinOp::Gt => Interval::new(b.lo + 1, FAR),
        BinOp::Ge => Interval::new(b.lo, FAR),
        BinOp::Eq => b,
        _ if b.lo == b.hi && a.lo == b.lo && a.hi > a.lo => Interval::new(a.lo + 1, a.hi),
        _ if b.lo == b.hi && a.hi == b.lo && a.hi > a.lo => Interval::new(a.lo, a.hi - 1),
        _ => return None,
    })
}
