//! What a function does, followed through its calls: for each function the first reason it
//! allocates, can fail, has a check left, makes a call no one can see into, or can call itself (and
//! the calls that lead to it), found to a fixed point over the call graph. The report uses
//! `allocates` and `fails`; contracts use every chain. The reasons come from `events` and the
//! analysis; the sentences a chain makes are written here.

mod cycles;

pub(crate) use cycles::{implementations, implementations_of};

use super::events::{Event, What};
use super::sites::CheckKind;
use super::Analysis;
use crate::ir::*;

/// Why something allocates or can fail, in the words a chain uses and in the words of an item in
/// the function itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Reason {
    /// The sentence for an item in the function that does it ("`add` may grow `ys`"); `None` when
    /// it is the chain's own words: `via`, the line, `tail`.
    pub direct: Option<String>,
    /// How a chain ends: "adds to `ps`", then " (line 12)", then `tail`. Empty for a reason about
    /// the whole function (range analysis stopped early in it): the chain ends with `tail` alone.
    pub via: String,
    pub tail: String,
    /// The packed line of the code that does it.
    pub line: u32,
    /// For a way to fail: which part of what the operation can do it is.
    pub part: Part,
}

/// Which part of what one operation can do a way to fail is. Running out of resources (a list or
/// a map growing past the item limit) isn't a way to fail for `no_panic`; the report lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// All of it (and every reason that isn't a way to fail).
    Whole,
    /// The logic failure of an operation that can also run out of resources (`insert` with an
    /// index outside the list): it counts for `no_panic`. The report prints it together with the
    /// `Resource` part that comes right after it, whose words say both.
    Logic,
    /// Running out of resources only.
    Resource,
}

/// A call on the way to a reason: the callee, and the packed line the previous function calls it
/// at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub callee: FuncIdx,
    pub line: u32,
}

/// The way from a function to the first reason of a kind in it or in what it calls: the calls
/// after the function itself, then the reason.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    pub steps: Vec<Step>,
    pub reason: Reason,
}

/// What a function does, with what it calls (each a chain to the first reason, if there is one).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// It may allocate: an allocation, or an unknown call that may allocate (the report's, and
    /// `no_alloc`'s).
    pub allocates: Option<Chain>,
    /// It may fail, running out of resources included, or make an unknown call that may (the
    /// report's).
    pub fails: Option<Chain>,
    /// It may fail for a logic reason (`no_panic`'s; unknown calls are in `unknown_panic`).
    pub panics: Option<Chain>,
    /// It has a check left (`no_checks`'s and `no_panic`'s).
    pub checks: Option<Chain>,
    /// Range analysis stopped early in it (or left a site undecided, or the walk was cut), or it
    /// makes a call no one can see into that may have a check, allocate, or fail.
    pub unknown_checks: Option<Chain>,
    pub unknown_alloc: Option<Chain>,
    pub unknown_panic: Option<Chain>,
    /// It can call itself, directly or through other functions, so the stack may run out.
    pub recursion: Option<Chain>,
    /// The checks left in it and in everything it calls, each counted once (the report's; 0 when
    /// not asked for).
    pub checks_left: u32,
}

/// The summary of every function in `m`, from each function's events and the analysis `a`. With
/// `own_checks` (each function's check ids, see `report::check_ids`) it also counts the checks
/// left in what each function reaches.
pub fn summarize(m: &Module, a: &Analysis, events: &[Vec<Event>], own_checks: Option<&[Vec<u64>]>) -> Vec<Summary> {
    let first = |pick: &dyn Fn(&Event) -> Option<Reason>| -> Vec<Option<Reason>> { events.iter().map(|es| es.iter().find_map(pick)).collect() };
    let allocates = first(&|e| match &e.what {
        What::Alloc(r) => Some(r.clone()),
        What::Unknown(k, r) if k.allocates() => Some(r.clone()),
        _ => None,
    });
    let fails = first(&|e| match &e.what {
        What::Fail(r) if r.part != Part::Logic => Some(r.clone()),
        What::Unknown(k, r) if k.fails() => Some(r.clone()),
        _ => None,
    });
    let panics = first(&|e| match &e.what {
        What::Fail(r) if r.part != Part::Resource => Some(r.clone()),
        _ => None,
    });
    let stopped: Vec<Option<Reason>> = (0..events.len()).map(|f| stopped(a, events, f)).collect();
    let unknown = |pick: fn(super::events::Unknown) -> bool| -> Vec<Option<Reason>> {
        let found = first(&|e| match &e.what {
            What::Unknown(k, r) if pick(*k) => Some(r.clone()),
            _ => None,
        });
        stopped.iter().zip(found).map(|(s, u)| s.clone().or(u)).collect()
    };
    let checks: Vec<Option<Reason>> = (0..events.len()).map(|f| a.funcs.get(f).and_then(|c| c.left.first()).map(check_reason)).collect();
    let recursion = cycles::own_reasons(m, events);
    let out = Chains::new(events);
    let reach = match own_checks {
        Some(own) => reachable_checks(m, events, own),
        None => vec![0; events.len()],
    };
    let mut chains = [
        out.chains(allocates),
        out.chains(fails),
        out.chains(panics),
        out.chains(checks),
        out.chains(unknown(|k| k.checks())),
        out.chains(unknown(|k| k.allocates())),
        out.chains(unknown(|k| k.fails())),
        out.chains(recursion),
    ]
    .map(|v| v.into_iter());
    reach
        .into_iter()
        .map(|checks_left| {
            let mut next = |i: usize| chains[i].next().flatten();
            Summary {
                allocates: next(0),
                fails: next(1),
                panics: next(2),
                checks: next(3),
                unknown_checks: next(4),
                unknown_alloc: next(5),
                unknown_panic: next(6),
                recursion: next(7),
                checks_left,
            }
        })
        .collect()
}

/// "an overflow", "a bounds": a check kind with its article.
pub fn check_words(kind: CheckKind) -> &'static str {
    match kind {
        CheckKind::Overflow => "an overflow",
        CheckKind::Shift => "a shift",
        CheckKind::Division => "a division",
        CheckKind::Narrowing => "a narrowing",
        CheckKind::Bounds => "a bounds",
    }
}

/// A check left, as a reason: "has a bounds check" (line): "{text}".
fn check_reason(c: &super::CheckLeft) -> Reason {
    let via = format!("has {} check", check_words(c.kind));
    Reason { direct: Some(c.text.clone()), via, tail: format!(": {}", c.text), line: c.line, part: Part::Whole }
}

/// Why nothing can be proven about a function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// Range analysis gave it up because its code nests too deeply, or the events walk was cut
    /// short in it for the same reason.
    Deep,
    /// Range analysis gave it up because it took too many steps.
    Slow,
    /// Range analysis never decided one of its check sites (a gap in the analysis).
    Undecided,
}

impl Stop {
    /// What range analysis did, as words after "range analysis" that a place can follow:
    /// ("stopped early", "(its code nests too deeply)").
    pub fn words(self) -> (&'static str, &'static str) {
        match self {
            Stop::Deep => ("stopped early", " (its code nests too deeply)"),
            Stop::Slow => ("stopped early", " (it took too long to analyse)"),
            Stop::Undecided => ("missed a check", " (a compiler bug)"),
        }
    }
}

/// Why nothing can be proven about function `f`, if that is so.
pub fn stop(a: &Analysis, events: &[Vec<Event>], f: usize) -> Option<Stop> {
    let c = a.funcs.get(f)?;
    if (c.gave_up && c.deep) || events[f].iter().any(|e| e.what == What::Cut) {
        Some(Stop::Deep)
    } else if c.gave_up {
        Some(Stop::Slow)
    } else if c.undecided {
        Some(Stop::Undecided)
    } else {
        None
    }
}

/// `stop` as a reason about the whole function: its chain ends ", where range analysis stopped
/// early (...)".
fn stopped(a: &Analysis, events: &[Vec<Event>], f: usize) -> Option<Reason> {
    let (did, why) = stop(a, events, f)?.words();
    let tail = format!(", where range analysis {did}{why}");
    Some(Reason { direct: None, via: String::new(), tail, line: 0, part: Part::Whole })
}

/// The calls a function makes, in the order it makes them (a `par for` body and a closure made
/// count as called).
pub(super) fn callees(events: &[Event]) -> impl Iterator<Item = (FuncIdx, u32)> + '_ {
    events.iter().filter_map(|e| match e.what {
        What::Call(g) | What::Body(g) | What::Closure(g) => Some((g, e.line)),
        _ => None,
    })
}

/// The call graph, for finding chains.
struct Chains<'a> {
    events: &'a [Vec<Event>],
    /// Who calls each function.
    callers: Vec<Vec<usize>>,
}

impl<'a> Chains<'a> {
    fn new(events: &'a [Vec<Event>]) -> Self {
        let mut callers: Vec<Vec<usize>> = vec![Vec::new(); events.len()];
        for (f, es) in events.iter().enumerate() {
            for (g, _) in callees(es) {
                callers[g.0 as usize].push(f);
            }
        }
        Chains { events, callers }
    }

    /// Each function's chain to the first reason in `own`. A function with a reason of its own
    /// uses it; otherwise it takes the first call to a function that is closer to one (fewer calls
    /// away), so a cycle never leads back to itself.
    fn chains(&self, own: Vec<Option<Reason>>) -> Vec<Option<Chain>> {
        let n = self.events.len();
        let mut dist: Vec<Option<u32>> = own.iter().map(|r| r.as_ref().map(|_| 0)).collect();
        let mut queue: std::collections::VecDeque<usize> = (0..n).filter(|&f| dist[f].is_some()).collect();
        let mut order = Vec::new();
        while let Some(g) = queue.pop_front() {
            order.push(g);
            let d = dist[g].expect("queued with a distance") + 1;
            for &f in &self.callers[g] {
                if dist[f].is_none() {
                    dist[f] = Some(d);
                    queue.push_back(f);
                }
            }
        }
        let mut out: Vec<Option<Chain>> = vec![None; n];
        let mut own = own;
        for f in order {
            let d = dist[f].expect("only functions with a distance");
            out[f] = if d == 0 {
                own[f].take().map(|reason| Chain { steps: Vec::new(), reason })
            } else {
                callees(&self.events[f]).find(|(g, _)| dist[g.0 as usize].is_some_and(|dg| dg < d)).map(|(g, line)| {
                    let next = out[g.0 as usize].as_ref().expect("a nearer function has its chain");
                    let mut steps = vec![Step { callee: g, line }];
                    steps.extend(next.steps.iter().copied());
                    Chain { steps, reason: next.reason.clone() }
                })
            };
        }
        out
    }
}

/// For each function, the checks left in it and in every function it calls. `own` identifies each
/// check of a function (its line, kind and text); a check that an inlined copy repeats in another
/// function is one check.
fn reachable_checks(m: &Module, events: &[Vec<Event>], own: &[Vec<u64>]) -> Vec<u32> {
    let n = m.funcs.len();
    let mut out = Vec::with_capacity(n);
    let mut seen = vec![u32::MAX; n];
    let mut stack = Vec::new();
    for f in 0..n {
        let mut ids: Vec<u64> = Vec::new();
        stack.push(f);
        seen[f] = f as u32;
        while let Some(g) = stack.pop() {
            ids.extend_from_slice(&own[g]);
            for (c, _) in callees(&events[g]) {
                let c = c.0 as usize;
                if seen[c] != f as u32 {
                    seen[c] = f as u32;
                    stack.push(c);
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        out.push(ids.len() as u32);
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
    if chain.steps.is_empty() && is_builtin_body(m, callee) && !chain.reason.via.is_empty() {
        // Code the compiler made has no lines: its reason is shown at the line of the call.
        let mut reason = chain.reason.clone();
        if reason.line == 0 {
            reason.line = line;
        }
        return reason_text(m, &reason, home);
    }
    format!("calls {} ({}){}", super::report::callee_name(m, callee), line_ref(m, line, home), chain_rest(m, chain, line))
}

/// Whether `callee` is a function the compiler made whose reasons read as the built-in's own (not
/// a sort's comparison, which calls the program's key).
pub fn is_builtin_body(m: &Module, callee: FuncIdx) -> bool {
    super::report::is_generated(m, callee) && !super::report::is_sort_comparison(&m.funcs[callee.0 as usize].name)
}

/// What a chain says after its first call, made at `line`: each further call, then the reason
/// (", which calls `b` (line 7), which adds to `ps` (line 12) and may grow the list").
pub fn chain_rest(m: &Module, chain: &Chain, line: u32) -> String {
    let home = unpack_line(line).1;
    // Code the compiler made has no lines: its calls and reasons are shown at the line before.
    let mut at = line;
    let mut s = String::new();
    for step in &chain.steps {
        if step.line != 0 {
            at = step.line;
        }
        // (A thunk only passes the call on.)
        if m.funcs[step.callee.0 as usize].name.starts_with("__kestrel_fn_") {
            continue;
        }
        s.push_str(&format!(", which calls {} ({})", super::report::callee_name(m, step.callee), line_ref(m, at, home)));
    }
    let reason = &chain.reason;
    if reason.via.is_empty() {
        s.push_str(&reason.tail);
        return s;
    }
    let shown = if reason.line == 0 { at } else { reason.line };
    s.push_str(&format!(", which {} ({}){}", reason.via, line_ref(m, shown, home), reason.tail));
    s
}
