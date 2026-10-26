//! Which calls get inlined: the per-call decision, and the choice of which calls of a function
//! get the budget when it can't fit them all.
//!
//! A function is walked twice. The first walk (`Mode::Plan`) inlines nothing: it lists the calls
//! that could be inlined but for the budget, each with its size and how many loops it is in. The
//! budget goes to those calls deepest loop first, then in program order (`grant`). The second
//! walk (`Mode::Run`) inlines, and reads each call's answer from that list: both walks meet the
//! same calls in the same order, since neither looks into a copy of a callee that is already
//! inlined into, and the calls inside a copy of a function of the caller's own component are not
//! in the list (see `Inliner::in_mate`): they take what the list left, in program order.

use std::cmp::Reverse;

use super::{Inliner, Why, POST_LIMIT};
use crate::ir::*;
use crate::range::report::callee_name;
use crate::range::show::commas;

/// A call that could be inlined but for the budget.
pub(super) struct Site {
    /// The callee's size in nodes: what a copy of it costs.
    size: usize,
    /// How many loops the call is in.
    depth: usize,
}

/// What the walk of a function is doing.
pub(super) enum Mode {
    /// Listing the calls that compete for the budget.
    Plan(Vec<Site>),
    /// Inlining: whether each of those calls was granted, in the order listed, and how many have
    /// been met.
    Run { granted: Vec<bool>, next: usize },
}

/// Which of `sites` get the budget, and what is left of `budget` after them: deepest loop first,
/// then in the order listed. A call that doesn't fit what is left is passed over, and a later,
/// smaller one may still fit.
fn grant(sites: &[Site], mut budget: usize) -> (Vec<bool>, usize) {
    let mut order: Vec<usize> = (0..sites.len()).collect();
    order.sort_by_key(|&i| (Reverse(sites[i].depth), i));
    let mut granted = vec![false; sites.len()];
    for i in order {
        if sites[i].size <= budget {
            budget -= sites[i].size;
            granted[i] = true;
        }
    }
    (granted, budget)
}

/// How a call is decided, apart from the budget.
enum Verdict {
    /// A native: it stays a call and is not worth a note.
    Native,
    /// It stays a call, for this reason.
    Stays(String),
    /// It is inlined if the budget allows; this many nodes.
    Fits(usize),
}

impl Inliner<'_> {
    /// Inlines into `body`, the function being inlined into.
    pub(super) fn run(&mut self, body: &mut Vec<Stmt>) {
        let line = self.line;
        self.mode = Mode::Plan(Vec::new());
        self.stmts(body);
        let Mode::Plan(sites) = std::mem::replace(&mut self.mode, Mode::Plan(Vec::new())) else { unreachable!("just set") };
        let (granted, left) = grant(&sites, self.budget);
        self.mode = Mode::Run { granted, next: 0 };
        self.budget = left;
        // What the first walk noted, the second notes again.
        self.log.clear();
        self.line = line;
        self.stmts(body);
    }

    fn verdict(&self, fi: FuncIdx) -> Verdict {
        let i = fi.0 as usize;
        let mate = self.comp[i] == self.comp[self.caller.0 as usize];
        match self.sizes[i] {
            Err(Why::Native) => Verdict::Native,
            Err(why) => Verdict::Stays(why.text()),
            Ok(_) if mate && self.in_mate => Verdict::Stays(format!("it calls back into {}", callee_name(self.m, fi))),
            // A function of the caller's own component is copied one level, as written. Its size
            // after inlining isn't known, so `POST_LIMIT` doesn't apply on purpose: it must be
            // inlinable (above) and fit the budget, which bounds the copy and what it inlines.
            Ok(written) if mate => Verdict::Fits(written),
            Ok(_) => {
                let size = self.done[i].as_ref().expect("callees are inlined into first").size;
                if size > POST_LIMIT {
                    Verdict::Stays(format!("it is {} nodes after its own inlining; the limit is {}", commas(size), commas(POST_LIMIT)))
                } else {
                    Verdict::Fits(size)
                }
            }
        }
    }

    /// True when a call to `fi` (written on `line`) should be inlined. Otherwise notes the call
    /// and why in the log. (In the first walk a call is only listed, and stays.)
    pub(super) fn take(&mut self, fi: FuncIdx, line: u32) -> bool {
        let reason = match self.verdict(fi) {
            Verdict::Native => return false,
            Verdict::Stays(reason) => reason,
            Verdict::Fits(size) => {
                let fits = if self.in_mate {
                    // Not listed: takes what the listed calls left.
                    let fits = size <= self.budget;
                    if fits {
                        self.budget -= size;
                    }
                    fits
                } else {
                    match &mut self.mode {
                        Mode::Plan(sites) => {
                            sites.push(Site { size, depth: self.depth });
                            return false;
                        }
                        Mode::Run { granted, next } => {
                            *next += 1;
                            granted[*next - 1]
                        }
                    }
                };
                if fits {
                    return true;
                }
                format!("the inlining budget of {} ran out", callee_name(self.m, self.caller))
            }
        };
        let line = if line == 0 { self.line } else { line };
        self.log.push(NotInlined { caller: self.caller, line, callee: fi, reason });
        false
    }
}
