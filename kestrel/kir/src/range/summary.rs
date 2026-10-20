//! What a function allocates and how it can fail, followed through its calls: for each function
//! the first reason (and the calls that lead to it), found to a fixed point over the call graph.
//! The reasons come from `events`; the sentences a chain makes are written here.

use super::events::{Event, What};
use crate::ir::*;

/// Why something allocates or can fail, in the words a chain uses and in the words of an item in
/// the function itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Reason {
    /// The sentence for an item in the function that does it ("`add` may grow `ys`"); `None` when
    /// it is the chain's own words: `via`, the line, `tail`.
    pub direct: Option<String>,
    /// How a chain ends: "adds to `ps`", then " (line 12)", then `tail`.
    pub via: String,
    pub tail: String,
    /// The packed line of the code that does it.
    pub line: u32,
}

/// A call on the way to a reason: the callee, and the packed line the previous function calls it
/// at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub callee: FuncIdx,
    pub line: u32,
}

/// The way from a function to the first thing that allocates (or can fail) in it or in what it
/// calls: the calls after the function itself, then the reason.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    pub steps: Vec<Step>,
    pub reason: Reason,
}

/// What a function does, with what it calls: the first reason it allocates, the first reason it
/// can fail, and the checks left in it and in everything it calls (each function counted once).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub allocates: Option<Chain>,
    pub fails: Option<Chain>,
    pub checks_left: u32,
}

/// Which reasons a chain follows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Want {
    Alloc,
    Fail,
}

/// The summary of every function in `m`, from each function's events and the number of checks
/// left in each.
pub fn summarize(m: &Module, events: &[Vec<Event>], own_checks: &[u32]) -> Vec<Summary> {
    let allocs = chains(events, Want::Alloc);
    let fails = chains(events, Want::Fail);
    let reach = reachable_checks(m, events, own_checks);
    allocs
        .into_iter()
        .zip(fails)
        .zip(reach)
        .map(|((allocates, fails), checks_left)| Summary { allocates, fails, checks_left })
        .collect()
}

/// The calls a function makes, in the order it makes them (a `par for` body counts as called).
fn callees(events: &[Event]) -> impl Iterator<Item = (FuncIdx, u32)> + '_ {
    events.iter().filter_map(|e| match e.what {
        What::Call(g) | What::Body(g) => Some((g, e.line)),
        _ => None,
    })
}

/// Each function's chain to the first reason of the kind `want`. A function with a reason of its
/// own uses the first; otherwise it takes the first call to a function that is closer to one, so
/// a cycle never leads back to itself.
fn chains(events: &[Vec<Event>], want: Want) -> Vec<Option<Chain>> {
    let own = |e: &Event| match (&e.what, want) {
        (What::Alloc(r), Want::Alloc) | (What::Fail(r), Want::Fail) => Some(r.clone()),
        _ => None,
    };
    let n = events.len();
    // How many calls away from a reason each function is.
    let mut dist: Vec<Option<u32>> = events.iter().map(|es| es.iter().any(|e| own(e).is_some()).then_some(0)).collect();
    let mut round = 1;
    loop {
        let mut changed = false;
        for f in 0..n {
            if dist[f].is_none() && callees(&events[f]).any(|(g, _)| dist[g.0 as usize] == Some(round - 1)) {
                dist[f] = Some(round);
                changed = true;
            }
        }
        if !changed {
            break;
        }
        round += 1;
    }
    let mut out: Vec<Option<Chain>> = vec![None; n];
    let mut order: Vec<usize> = (0..n).filter(|&f| dist[f].is_some()).collect();
    order.sort_by_key(|&f| dist[f]);
    for f in order {
        let d = dist[f].expect("only functions with a distance");
        let chain = if d == 0 {
            events[f].iter().find_map(own).map(|reason| Chain { steps: Vec::new(), reason })
        } else {
            callees(&events[f]).find(|(g, _)| dist[g.0 as usize].is_some_and(|dg| dg < d)).map(|(g, line)| {
                let next = out[g.0 as usize].as_ref().expect("a nearer function has its chain");
                let mut steps = vec![Step { callee: g, line }];
                steps.extend(next.steps.iter().copied());
                Chain { steps, reason: next.reason.clone() }
            })
        };
        out[f] = chain;
    }
    out
}

/// For each function, the checks left in it and in every function it calls, each counted once.
fn reachable_checks(m: &Module, events: &[Vec<Event>], own: &[u32]) -> Vec<u32> {
    let n = m.funcs.len();
    let mut out = Vec::with_capacity(n);
    let mut seen = vec![u32::MAX; n];
    let mut stack = Vec::new();
    for f in 0..n {
        let mut total = 0u32;
        stack.push(f);
        seen[f] = f as u32;
        while let Some(g) = stack.pop() {
            total = total.saturating_add(own[g]);
            for (c, _) in callees(&events[g]) {
                let c = c.0 as usize;
                if seen[c] != f as u32 {
                    seen[c] = f as u32;
                    stack.push(c);
                }
            }
        }
        out.push(total);
    }
    out
}

/// A line as the text of a chain writes it: "line 12", or "geom.kes:12" when it is in another
/// file than `home` (a file index).
pub fn line_ref(m: &Module, line: u32, home: u32) -> String {
    let (n, file) = unpack_line(line);
    if file == home || file == NO_FILE {
        return format!("line {n}");
    }
    match m.files.get(file as usize) {
        Some(name) => format!("{name}:{n}"),
        None => format!("line {n}"),
    }
}

/// The words of a reason on its own, in the function that has it.
pub fn reason_text(m: &Module, r: &Reason, home: u32) -> String {
    match &r.direct {
        Some(d) => d.clone(),
        None => format!("{} ({}){}", r.via, line_ref(m, r.line, home), r.tail),
    }
}

/// The words of an item for a call at `line` to `callee`, whose chain is `chain`: "calls `spawn`
/// (line 40), which adds to `ps` (line 12) and may grow the list". A built-in's own generated
/// function that has its reason itself is just that reason ("`filter` builds a new list").
pub fn chain_text(m: &Module, callee: FuncIdx, chain: &Chain, line: u32) -> String {
    let home = unpack_line(line).1;
    if chain.steps.is_empty() && super::report::is_generated(m, callee) {
        return reason_text(m, &chain.reason, home);
    }
    let mut s = format!("calls {} ({})", super::report::callee_name(m, callee), line_ref(m, line, home));
    for step in &chain.steps {
        s.push_str(&format!(", which calls {} ({})", super::report::callee_name(m, step.callee), line_ref(m, step.line, home)));
    }
    let r = &chain.reason;
    s.push_str(&format!(", which {} ({}){}", r.via, line_ref(m, r.line, home), r.tail));
    s
}
