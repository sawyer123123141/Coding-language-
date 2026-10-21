//! `@assume` (`Stmt::Assume`): inside its block an assumption teaches what `if cond` teaches on
//! its true branch; nothing outside its block sees it (the addendum's "`@assume`" and "Soundness
//! rules an assumption must not break").
//!
//! In a deciding pass the block runs twice from the state before it:
//! - the **real run** starts from the true state of the conditions, in order (`condition`, the
//!   spec's condition rule), with the assumptions inside the block as written. Its decisions are
//!   the function's. Its `break`s and `continue`s go nowhere (the frames around it are moved aside
//!   while it runs) and its end is dropped;
//! - the **plain run** starts from the state before the assumption, as if no assumption in the
//!   block were written: each assumption inside is a plain block whose conditions are evaluated
//!   for their own checks but teach nothing, not even the facts from their checks that passed
//!   (those hold only while the conditions really run, and a `Trust`ed one doesn't). Its
//!   decisions go to a scratch log. **Everything that leaves the block comes from it:** its end,
//!   its `break`s and `continue`s (so the next iteration starts without the assumption), and
//!   nothing for `return`, as in any block.
//!
//! A site the real run proves and the plain run leaves is one the assumption removes; each
//! statement's uses are folded into one `AssumeUse` (by the statement's address, as checks are
//! keyed). A pass that decides nothing (an enclosing loop's fixed-point rounds), code no state
//! reaches, and a plain run only run the plain block: the real run's decisions would go nowhere
//! and its exits are never used.
//!
//! **Rule 7, the plain run's division facts:** the plain run learns from a division's zero check
//! that passed that the divisor isn't 0 (`arith.rs` `nonzero`), even where the real run proved the
//! check and a `--release` build trusts the assumption and drops it. That still holds because
//! codegen divides a `Proven` division plainly with Cranelift `sdiv`, `srem`, `udiv` or `urem`
//! (`codegen/src/func/divide.rs` `plain_div`), which trap on a zero divisor: past the division
//! the divisor wasn't 0. If `divide.rs` ever emits a non-trapping divide for a proven division,
//! a trusted assumption must stop removing division checks. Under `--release` division and
//! bounds checks are the only checks there are, and a bounds check keeps its assumption
//! evaluated (`mark.rs`), so no other fact from a removed check reaches the code after the block.

use std::collections::{HashMap, HashSet};

use super::state::State;
use super::{writes, Cx, Key};
use crate::ir::*;

/// What range analysis found about one `@assume` statement (one analysed copy of it: inlining can
/// put several in one function, each its own statement).
#[derive(Clone, Debug, PartialEq)]
pub struct AssumeUse {
    /// The ids of its conditions (indexes into `Module::assumptions`): a merged statement's use
    /// reaches each of its `@assume`s (N9).
    pub ids: Vec<u32>,
    /// The function the statement is in.
    pub func: FuncIdx,
    /// The statement's address, as a check site's key is (valid while the module is unchanged).
    pub at: usize,
    /// The distinct check sites it removed: proven by its real run, left by its plain run.
    pub removed: u32,
    /// Whether one of them is a bounds check (then `--release` keeps the assumption evaluated).
    pub bounds: bool,
}

/// The decisions of one run of an assumption's block: each site, proven or not (a site decided
/// twice in one run is proven only if both decisions prove it).
pub(super) type RunLog = HashMap<Key, bool>;

/// An assumption statement's uses so far, folded: the sites removed over every visit.
pub(super) struct Folded {
    ids: Vec<u32>,
    at: usize,
    removed: HashSet<Key>,
}

impl Folded {
    pub(super) fn into_use(self, func: FuncIdx) -> AssumeUse {
        let bounds = self.removed.iter().any(|&(_, index)| index);
        AssumeUse { ids: self.ids, func, at: self.at, removed: self.removed.len() as u32, bounds }
    }
}

/// Records one decision in a run's log.
pub(super) fn note(log: &mut RunLog, key: Key, proven: bool) {
    log.entry(key).and_modify(|p| *p &= proven).or_insert(proven);
}

impl Cx<'_> {
    /// The state after the assumption `s` (with `conds` and `body`), from `st`.
    pub(super) fn assume(&mut self, st: State, s: &Stmt, conds: &[(u32, Expr)], body: &[Stmt]) -> State {
        if !self.deciding || self.keep > 0 || self.plain > 0 {
            return self.plain_block(st, conds, body);
        }
        // The real run, from the true state of each condition in turn. The conditions' own checks
        // are decided here, outside both runs' logs: they are the enclosing code's.
        let mut real = st.clone();
        for (_, cond) in conds {
            real = self.assumed(real, cond);
        }
        let saved: Vec<_> = self.frames.iter_mut().map(std::mem::take).collect();
        let mark = self.learned.len();
        self.logs.push(RunLog::new());
        let _ = self.stmts(real, body);
        let real_log = self.logs.pop().expect("pushed above");
        self.learned.truncate(mark);
        for (frame, old) in self.frames.iter_mut().zip(saved) {
            *frame = old;
        }
        // Its decisions are an enclosing real run's too.
        if let Some(outer) = self.logs.last_mut() {
            for (&key, &proven) in &real_log {
                note(outer, key, proven);
            }
        }
        // The plain run, whose exits and end are the statement's.
        self.logs.push(RunLog::new());
        self.plain += 1;
        let end = self.stmts(st, body);
        self.plain -= 1;
        let plain_log = self.logs.pop().expect("pushed above");
        if !self.gave_up {
            self.record(s, conds, &real_log, &plain_log);
        }
        end
    }

    /// The true state of `cond` from `st`, through the condition rule. Under `--release` the
    /// statement may be trusted and its conditions never evaluated, so the facts from their checks
    /// that passed are dropped (there are none today: a condition's sides can't divide or index).
    fn assumed(&mut self, st: State, cond: &Expr) -> State {
        if self.arith.checked() {
            return self.condition(st, cond).0;
        }
        let mut st = st;
        let w = writes::of_expr(cond);
        self.forget(&mut st, &w);
        self.refine(&mut st, cond, true);
        self.forget(&mut st, &w);
        st
    }

    /// The block as if the assumption weren't written: its conditions are evaluated for their own
    /// checks (an enclosing assumption may remove one) and teach nothing; then the body.
    fn plain_block(&mut self, st: State, conds: &[(u32, Expr)], body: &[Stmt]) -> State {
        let mut st = st;
        for (_, cond) in conds {
            let w = writes::of_expr(cond);
            self.forget(&mut st, &w);
            let mark = self.learned.len();
            self.within(&w, |cx| {
                cx.eval(&st, cond, true);
            });
            self.learned.truncate(mark);
            self.forget(&mut st, &w);
        }
        self.stmts(st, body)
    }

    /// Folds one deciding visit of statement `s` into its use: the sites its real run proved and
    /// its plain run left (or never decided).
    fn record(&mut self, s: &Stmt, conds: &[(u32, Expr)], real: &RunLog, plain: &RunLog) {
        let at = s as *const Stmt as usize;
        let i = match self.uses.iter().position(|u| u.at == at) {
            Some(i) => i,
            None => {
                let ids = conds.iter().map(|(id, _)| *id).collect();
                self.uses.push(Folded { ids, at, removed: HashSet::new() });
                self.uses.len() - 1
            }
        };
        let removed = real.iter().filter(|&(k, &proven)| proven && !plain.get(k).copied().unwrap_or(false));
        self.uses[i].removed.extend(removed.map(|(&k, _)| k));
    }
}
