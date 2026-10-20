//! What the analysis knows at one point of a function: a range for every tracked integer local
//! and every other term (a length, a loop's end), and facts between terms (`facts`). Every term's
//! range is kept tight against the facts: `a - b <= c` and `b <= hi` make `a` at most `c + hi`.
//! With the facts closed, one step of that is all it takes, so the ranges and the facts together
//! say everything the two imply. Both live on the heap, so recursing deep never puts a big state
//! on the stack.

use super::facts::{Facts, ZERO};
use super::interval::Interval;
use super::tracked::{NodeKind, Tracked};
use crate::ir::*;

/// The ranges and facts at one point, or no state at all (`dead`: the point can't be reached).
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    dead: bool,
    ranges: Vec<Interval>,
    /// The range of each node that isn't a local (a length, a loop's end).
    nodes: Vec<Interval>,
    facts: Facts,
}

/// Far beyond any value: an open end of a range a fact gives.
const FAR: i128 = 1 << 100;

impl State {
    /// A live state with these ranges (of the tracked locals, and of every node) and no facts.
    pub fn new(ranges: Vec<Interval>, nodes: Vec<Interval>) -> State {
        let n = nodes.len();
        State { dead: false, ranges, nodes, facts: Facts::new(n) }
    }

    /// The state of a point nothing reaches.
    pub fn dead() -> State {
        State { dead: true, ranges: Vec::new(), nodes: Vec::new(), facts: Facts::new(0) }
    }

    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// The range of a tracked local (`None` for any other local, or a dead state).
    pub fn get(&self, t: &Tracked, l: LocalIdx) -> Option<Interval> {
        if self.dead {
            return None;
        }
        t.slot(l).map(|s| self.ranges[s])
    }

    /// The range of a node.
    pub fn node_range(&self, t: &Tracked, n: usize) -> Interval {
        match t.kinds[n] {
            _ if self.dead => t.node_type(n),
            NodeKind::Zero => Interval::exact(0),
            NodeKind::Local(s) => self.ranges[s],
            _ => self.nodes[n],
        }
    }

    fn put_range(&mut self, t: &Tracked, n: usize, r: Interval) {
        match t.kinds[n] {
            NodeKind::Zero => {}
            NodeKind::Local(s) => self.ranges[s] = r,
            _ => self.nodes[n] = r,
        }
    }

    /// The least `c` the facts and ranges give with `a - b <= c`.
    fn bound_of(&self, t: &Tracked, a: usize, b: usize) -> i128 {
        let by_ranges = self.node_range(t, a).hi - self.node_range(t, b).lo;
        if a == ZERO || b == ZERO {
            return by_ranges;
        }
        self.facts.get(a, b).map_or(by_ranges, |c| c.min(by_ranges))
    }

    /// Whether `a - b <= c` holds here (never said of a dead state).
    pub fn implies(&self, t: &Tracked, a: usize, b: usize, c: i128) -> bool {
        if self.dead {
            return false;
        }
        if a == b {
            return c >= 0;
        }
        self.bound_of(t, a, b) <= c
    }

    /// The `k` with `a == b + k`, when the facts and ranges say so.
    pub fn equal_offset(&self, t: &Tracked, a: usize, b: usize) -> Option<i128> {
        if self.dead || a == b {
            return None;
        }
        let k = self.bound_of(t, a, b);
        (self.bound_of(t, b, a) == -k).then_some(k)
    }

    /// Replaces what is known about `l` (a no-op for an untracked local), clipped to its type.
    pub fn set(&mut self, t: &Tracked, l: LocalIdx, v: Interval) {
        if let (false, Some(s)) = (self.dead, t.slot(l)) {
            self.ranges[s] = v.clip(t.tys[s]);
            if let Some(n) = t.node(l) {
                self.facts.forget(n);
            }
        }
    }

    /// `l` can be anything its type holds: it was written.
    pub fn forget(&mut self, t: &Tracked, l: LocalIdx) {
        if let Some(s) = t.slot(l) {
            self.set(t, l, Interval::of_type(t.tys[s]));
        }
    }

    /// Nothing is known about node `n` beyond what it can be at all.
    pub fn forget_node(&mut self, t: &Tracked, n: usize) {
        if self.dead || n == ZERO {
            return;
        }
        self.facts.forget(n);
        self.put_range(t, n, t.node_type(n));
    }

    /// Adds what a condition or a passed check says about `l`: its value is also in `v`. When
    /// the two don't overlap, the point can't be reached.
    pub fn narrow(&mut self, t: &Tracked, l: LocalIdx, v: Interval) {
        if let (false, Some(s)) = (self.dead, t.slot(l)) {
            match t.node(l) {
                Some(n) => self.bound(t, n, v),
                None => match self.ranges[s].meet(v) {
                    Some(r) => self.ranges[s] = r,
                    None => *self = State::dead(),
                },
            }
        }
    }

    /// Node `n` is in `v` too; the ranges of the terms its facts reach tighten with it.
    pub fn bound(&mut self, t: &Tracked, n: usize, v: Interval) {
        if self.dead {
            return;
        }
        if n == ZERO {
            if !v.contains(0) {
                *self = State::dead();
            }
            return;
        }
        let old = self.node_range(t, n);
        let Some(r) = old.meet(v) else {
            *self = State::dead();
            return;
        };
        if r == old {
            return;
        }
        self.put_range(t, n, r);
        // `i - n <= c` makes `i` at most `c + hi`; `n - j <= c` makes `j` at least `lo - c`.
        let ins: Vec<_> = if r.hi < old.hi { self.facts.towards(n).collect() } else { Vec::new() };
        let outs: Vec<_> = if r.lo > old.lo { self.facts.away(n).collect() } else { Vec::new() };
        for (i, c) in ins {
            self.tighten(t, i, -FAR, c + r.hi);
        }
        for (j, c) in outs {
            self.tighten(t, j, r.lo - c, FAR);
        }
    }

    /// Node `n`'s range meets `lo..=hi`, without following its facts further (the caller has).
    fn tighten(&mut self, t: &Tracked, n: usize, lo: i128, hi: i128) {
        if self.dead || n == ZERO {
            return;
        }
        let r = self.node_range(t, n);
        let (lo, hi) = (r.lo.max(lo), r.hi.min(hi));
        if lo > hi {
            *self = State::dead();
        } else {
            self.put_range(t, n, Interval::new(lo, hi));
        }
    }

    /// Adds `a - b <= c`. When it contradicts what is known, the point can't be reached.
    pub fn fact(&mut self, t: &Tracked, a: usize, b: usize, c: i128) {
        if self.dead {
            return;
        }
        if a == b {
            if c < 0 {
                *self = State::dead();
            }
            return;
        }
        if b == ZERO {
            return self.bound(t, a, Interval::new(-FAR, c));
        }
        if a == ZERO {
            return self.bound(t, b, Interval::new(-c, FAR));
        }
        if self.bound_of(t, a, b) <= c || t.saturated() {
            return;
        }
        match self.facts.add(a, b, c) {
            Some(work) => t.spend(work),
            None => {
                *self = State::dead();
                return;
            }
        }
        // Each `i - a <= x` (and `a` itself) now reaches `b`'s upper end, and `a`'s lower end
        // reaches each `b - j <= y` (and `b` itself).
        let (hi_b, lo_a) = (self.node_range(t, b).hi, self.node_range(t, a).lo);
        let ins: Vec<_> = std::iter::once((a, 0)).chain(self.facts.towards(a)).collect();
        let outs: Vec<_> = std::iter::once((b, 0)).chain(self.facts.away(b)).collect();
        for (i, x) in ins {
            self.tighten(t, i, -FAR, x + c + hi_b);
        }
        for (j, y) in outs {
            self.tighten(t, j, lo_a - c - y, FAR);
        }
    }

    /// `a == b + c`.
    pub fn equal(&mut self, t: &Tracked, a: usize, b: usize, c: i128) {
        self.fact(t, a, b, c);
        self.fact(t, b, a, -c);
    }

    /// Node `n` became `n + by`, with the range `range`: its facts move with it.
    pub fn shift(&mut self, t: &Tracked, n: usize, by: i128, range: Interval) {
        if self.dead || n == ZERO {
            return;
        }
        self.facts.shift(n, by);
        let r = self.node_range(t, n);
        self.put_range(t, n, Interval::new(r.lo + by, r.hi + by));
        // Still inside what it can be at all (a list that grew past the item limit stopped the
        // program), and inside `range`.
        self.bound(t, n, t.node_type(n));
        self.bound(t, n, range);
    }

    /// Every range tightened by every fact once (enough, with the facts closed).
    fn retighten(&mut self, t: &Tracked) {
        let facts: Vec<_> = self.facts.all().collect();
        t.spend(facts.len() as u64);
        for (a, b, c) in facts {
            let (ra, rb) = (self.node_range(t, a), self.node_range(t, b));
            self.tighten(t, a, -FAR, c + rb.hi);
            self.tighten(t, b, ra.lo - c, FAR);
        }
    }

    /// What holds on either path. A fact one side has is kept at the weaker of the two sides'
    /// bounds (the other side's may come from its ranges).
    pub fn join(&self, t: &Tracked, other: &State) -> State {
        match (self.dead, other.dead) {
            (true, _) => other.clone(),
            (_, true) => self.clone(),
            _ => {
                let mut facts = Facts::new(self.facts.nodes());
                if !t.saturated() {
                    let pairs: Vec<(usize, usize)> = self.facts.all().chain(other.facts.all()).map(|(a, b, _)| (a, b)).collect();
                    t.spend(4 * pairs.len() as u64);
                    for (a, b) in pairs {
                        facts.put(a, b, Some(self.bound_of(t, a, b).max(other.bound_of(t, a, b))));
                    }
                    t.spend(facts.close());
                }
                let mut out = State {
                    dead: false,
                    ranges: self.ranges.iter().zip(&other.ranges).map(|(a, b)| a.join(*b)).collect(),
                    nodes: self.nodes.iter().zip(&other.nodes).map(|(a, b)| a.join(*b)).collect(),
                    facts,
                };
                out.retighten(t);
                out
            }
        }
    }

    /// `self` (a loop's top so far) widened by `new`: a range's end that still grows goes to its
    /// limit (`Interval::widen`), and a fact that got weaker is dropped, so a loop's repeats end.
    /// `settle` drops every fact, for a loop whose facts keep changing.
    pub fn widen(&self, t: &Tracked, new: &State, settle: bool) -> State {
        match (self.dead, new.dead) {
            (true, _) => new.clone(),
            (_, true) => self.clone(),
            _ => {
                let mut facts = Facts::new(self.facts.nodes());
                if !settle && !t.saturated() {
                    // A fact either side has, at the top's bound (which may come from its ranges),
                    // while the new side keeps to it.
                    let pairs: Vec<(usize, usize)> = self.facts.all().chain(new.facts.all()).map(|(a, b, _)| (a, b)).collect();
                    t.spend(4 * pairs.len() as u64);
                    for (a, b) in pairs {
                        let c = self.bound_of(t, a, b);
                        facts.put(a, b, (new.bound_of(t, a, b) <= c).then_some(c));
                    }
                    t.spend(facts.close());
                }
                let grow = |n: usize, old: Interval, nw: Interval| {
                    let ty = t.node_type(n);
                    let lo = if nw.lo < old.lo { ty.lo.min(nw.lo) } else { old.lo };
                    let hi = if nw.hi > old.hi { ty.hi.max(nw.hi) } else { old.hi };
                    Interval::new(lo, hi)
                };
                let mut out = State {
                    dead: false,
                    ranges: self.ranges.iter().zip(&new.ranges).zip(&t.tys).map(|((a, b), &ty)| a.widen(*b, ty)).collect(),
                    nodes: (0..self.nodes.len()).map(|n| grow(n, self.nodes[n], new.nodes[n])).collect(),
                    facts,
                };
                out.retighten(t);
                out
            }
        }
    }
}
