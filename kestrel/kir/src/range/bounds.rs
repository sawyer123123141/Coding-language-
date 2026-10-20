//! Values with linear forms, and bounds checks. An integer expression evaluates to a range and,
//! when it has one, a linear form `t + c` (a term node plus a constant). A bounds check is proven
//! when the index is 0 or more and below its list's length term; one that passed teaches both
//! (see the spec's "Facts from a check that passed").

use super::facts::ZERO;
use super::interval::Interval;
use super::sites::{self, CheckKind};
use super::state::State;
use super::terms::Term;
use super::{index_key, terms, writes, Cx, Learn, MAX_ITEMS};
use crate::ir::*;

/// `node + c`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Lin {
    pub node: usize,
    pub c: i128,
}

impl Lin {
    fn plus(self, by: i128) -> Lin {
        Lin { node: self.node, c: self.c + by }
    }
}

/// What the analysis knows about an expression's value.
#[derive(Clone, Copy, Debug)]
pub(super) struct Val {
    pub r: Interval,
    /// The value is exactly this.
    pub lin: Option<Lin>,
    /// The value is between these two, either way round (`rand_int`'s bounds).
    pub between: Option<(Lin, Lin)>,
    /// For a list or array value: its length.
    pub len: Option<Lin>,
}

impl Val {
    pub fn range(r: Interval) -> Val {
        Val { r, lin: None, between: None, len: None }
    }

    pub fn exact(r: Interval, lin: Option<Lin>) -> Val {
        Val { r, lin, between: None, len: None }
    }

    /// Two linear forms the value lies between (its own form twice, when it has one).
    pub fn bounds(&self) -> Option<(Lin, Lin)> {
        self.lin.map(|l| (l, l)).or(self.between)
    }

    /// The same value plus the constant `by` (no linear form when `keep` is false: the
    /// operation may have wrapped).
    pub fn plus(&self, r: Interval, by: i128, keep: bool) -> Val {
        Val {
            r,
            lin: self.lin.filter(|_| keep).map(|l| l.plus(by)),
            between: self.between.filter(|_| keep).map(|(a, b)| (a.plus(by), b.plus(by))),
            len: None,
        }
    }
}

impl Cx<'_> {
    /// The index expressions of a place, and its bounds checks.
    pub(super) fn place(&mut self, st: &State, p: &Place, sure: bool) {
        for (i, proj) in p.path.iter().enumerate() {
            let Some(at) = proj.at() else { continue };
            let v = self.eval_v(st, at, sure);
            if sites::is_bounds_site(proj) {
                let len = terms::list_key(self.m, self.f, p, i).and_then(|k| self.t.len_node(&k));
                let safe = self.in_bounds(st, &v, len);
                self.decide(index_key(proj), CheckKind::Bounds, at.line, safe, |cx| super::show::bounds_text(cx.m, cx.f, p, i, Some(v.r)));
                if sure {
                    self.learn_bounds(at, &v, len);
                }
            }
        }
    }

    /// Whether an index with value `v` is surely inside a list whose length is node `len`.
    fn in_bounds(&self, st: &State, v: &Val, len: Option<usize>) -> bool {
        let Some(len) = len else { return false };
        if st.is_dead() {
            return false;
        }
        let forms: Vec<Lin> = v.bounds().map_or(Vec::new(), |(a, b)| vec![a, b]);
        let all = |ok: &dyn Fn(Lin) -> bool| !forms.is_empty() && forms.iter().all(|l| ok(*l));
        // 0 <= node + c, that is 0 - node <= c.
        let low = v.r.lo >= 0 || all(&|l| st.implies(&self.t, ZERO, l.node, l.c));
        // node + c < len, that is node - len <= -1 - c.
        // A volatile length (the expression may change it) is used by its range only.
        let by_facts = !self.volatile(len) && all(&|l| st.implies(&self.t, l.node, len, -1 - l.c));
        let high = v.r.hi < st.node_range(&self.t, len).lo || by_facts;
        low && high
    }

    /// After the check on `at` passed: `0 <= at < len`.
    fn learn_bounds(&mut self, at: &Expr, v: &Val, len: Option<usize>) {
        // The length the check read may not be the one a volatile node stands for (see `within`).
        let len = len.filter(|&n| !self.volatile(n));
        match v.lin {
            Some(l) => {
                self.learned.push(Learn::Diff(ZERO, l.node, l.c));
                if let Some(len) = len {
                    self.learned.push(Learn::Diff(l.node, len, -1 - l.c));
                }
            }
            // The length is above the index's lowest value.
            None => {
                if let Some(len) = len {
                    self.learned.push(Learn::Diff(ZERO, len, -1 - v.r.lo.max(0)));
                }
            }
        }
        if let Some(l) = self.local_of(at).filter(|&l| !self.t.node(l).is_some_and(|n| self.volatile(n))) {
            self.learned.push(Learn::Range(l, Interval::new(0, MAX_ITEMS - 1)));
        }
    }

    /// `v`, the value of a `Seq` whose statements are `stmts`, as seen from before the `Seq`: a
    /// linear form over a term the statements wrote is moved onto an equal term they didn't
    /// write (`after` says which are equal), or dropped.
    pub(super) fn rebase(&self, after: &State, stmts: &[Stmt], v: Val) -> Val {
        let forms = [v.lin, v.between.map(|b| b.0), v.between.map(|b| b.1), v.len];
        if forms.iter().flatten().all(|l| l.node == ZERO) {
            return v;
        }
        let touched = self.touched_nodes(&writes::of_stmts(stmts));
        let moved = |l: Option<Lin>| -> Option<Lin> {
            let l = l?;
            if !touched[l.node] {
                return Some(l);
            }
            // An untouched node `u` with `node == u + k`.
            (0..self.t.nodes()).find_map(|u| {
                // Not a loop's end: a `ForRange` in the statements may set it again.
                if u == l.node || touched[u] || matches!(self.t.term(u), Some(Term::Hidden(_))) {
                    return None;
                }
                let k = after.equal_offset(&self.t, l.node, u)?;
                Some(Lin { node: u, c: l.c + k })
            })
        };
        let between = match v.between {
            Some((a, b)) => moved(Some(a)).zip(moved(Some(b))),
            None => None,
        };
        Val { r: v.r, lin: moved(v.lin), between, len: moved(v.len) }
    }
}

