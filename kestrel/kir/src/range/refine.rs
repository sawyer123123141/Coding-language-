//! Conditions: what `if c`, `while c`, a value `if`'s condition and the right side of `and` and
//! `or` know. A comparison of two integers adds a fact between their linear forms and narrows the
//! range of a local on either side; `and`, `or` and `not` combine their parts in order; anything
//! else teaches nothing. Each part forgets what it writes before it is evaluated and after it
//! refines (a part may change what it reads, or what an earlier part compared).

use super::bounds::{Lin, Val};
use super::facts::ZERO;
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
    /// before it is evaluated and once it has been refined with. `sure`: `e` runs whenever its
    /// statement does.
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
                    (bt, af.join(&self.t, &bf))
                }
            }
            ExprKind::Or(a, b) => {
                let (at, af) = self.cond_parts(st, a, sure);
                if af.is_dead() {
                    self.unreachable_expr(b);
                    (at, State::dead())
                } else {
                    let (bt, bf) = self.cond_parts(af, b, false);
                    (at.join(&self.t, &bt), bf)
                }
            }
            ExprKind::Binary(op, l, r) if op.is_comparison() && l.ty.is_int() => self.compare(st, e, *op, (l, r), sure),
            _ => {
                let w = writes::of_expr(e);
                let mut st = st;
                self.forget(&mut st, &w);
                self.within(&w, |cx| cx.eval(&st, e, sure));
                self.forget(&mut st, &w);
                (st.clone(), st)
            }
        };
        self.leave();
        out
    }

    /// `l op r`: a fact between the two sides' linear forms, and each side that is a tracked
    /// local narrowed by the other side's range.
    fn compare(&mut self, st: State, e: &Expr, op: BinOp, (l, r): (&Expr, &Expr), sure: bool) -> (State, State) {
        let w = writes::of_expr(e);
        let mut st = st;
        self.forget(&mut st, &w);
        let (a, b) = self.within(&w, |cx| (cx.eval_v(&st, l, sure), cx.eval_v(&st, r, sure)));
        let mut t = st.clone();
        let mut f = st;
        self.holds(&mut t, op, (l, a), (r, b));
        self.holds(&mut f, negated(op), (l, a), (r, b));
        self.forget(&mut t, &w);
        self.forget(&mut f, &w);
        (t, f)
    }

    /// Narrows `st` to where `l op r` holds, `a` and `b` being what is known of the two sides.
    fn holds(&self, st: &mut State, op: BinOp, (l, a): (&Expr, Val), (r, b): (&Expr, Val)) {
        if !can_hold(op, a.r, b.r) {
            *st = State::dead();
            return;
        }
        if let Some(x) = self.local_of(l) {
            if let Some(range) = side(op, a.r, b.r) {
                st.narrow(&self.t, x, range);
            }
        }
        if let Some(y) = self.local_of(r) {
            if let Some(range) = side(mirrored(op), b.r, a.r) {
                st.narrow(&self.t, y, range);
            }
        }
        match op {
            BinOp::Lt => self.at_most(st, &a, &b, -1),
            BinOp::Le => self.at_most(st, &a, &b, 0),
            BinOp::Gt => self.at_most(st, &b, &a, -1),
            BinOp::Ge => self.at_most(st, &b, &a, 0),
            BinOp::Eq => {
                self.at_most(st, &a, &b, 0);
                self.at_most(st, &b, &a, 0);
            }
            _ => {
                self.not_equal(st, &a, &b);
                self.not_equal(st, &b, &a);
            }
        }
    }

    /// `x - y <= k` for the two values: a fact between their linear forms, or between one's form
    /// and the other's range.
    fn at_most(&self, st: &mut State, x: &Val, y: &Val, k: i128) {
        match (x.lin, y.lin) {
            // (xn + xc) - (yn + yc) <= k
            (Some(xl), Some(yl)) => st.fact(&self.t, xl.node, yl.node, k - xl.c + yl.c),
            // xn + xc <= k + y <= k + y.hi
            (Some(xl), None) => st.fact(&self.t, xl.node, ZERO, k + y.r.hi - xl.c),
            // yn + yc >= x - k >= x.lo - k
            (None, Some(yl)) => st.fact(&self.t, ZERO, yl.node, k - x.r.lo + yl.c),
            (None, None) => {}
        }
    }

    /// `x != y` with `y` one number: `x`'s node can't be at that end of its range.
    fn not_equal(&self, st: &mut State, x: &Val, y: &Val) {
        let (Some(Lin { node, c }), true) = (x.lin, y.r.lo == y.r.hi) else { return };
        if node == ZERO || st.is_dead() {
            return;
        }
        let at = y.r.lo - c;
        let r = st.node_range(&self.t, node);
        if r.lo == at && r.hi > at {
            st.bound(&self.t, node, Interval::new(at + 1, r.hi));
        } else if r.hi == at && r.lo < at {
            st.bound(&self.t, node, Interval::new(r.lo, at - 1));
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
