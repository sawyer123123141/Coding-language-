//! The code a contract site covers: its copies (a function's own body and every copy the inliner
//! made of it; every `Prove` block of a loop), and what is in each copy (its checks left and
//! events, and, whole, the `par for` bodies it runs and the closures it makes).

use std::collections::HashMap;

use crate::ir::*;
use crate::range::events::{Event, What};
use crate::range::{Analysis, CheckLeft};
use crate::visit::{walk, Node};

/// One compiled copy of a site's code.
#[derive(Clone, Copy, Debug)]
pub(super) struct Copy {
    /// The function that holds it.
    pub func: usize,
    /// Its `Prove` block; `None` for a function's own body, whole.
    pub block: Option<Region>,
}

/// Every `Prove` block of `m`, by site id: the function holding it and its region, in function
/// order and, in a function, in `visit::walk` order (numbered as `prove_copies` numbers them).
pub(super) fn blocks(m: &Module) -> HashMap<u32, Vec<(usize, Region)>> {
    let mut out: HashMap<u32, Vec<(usize, Region)>> = HashMap::new();
    for (f, func) in m.funcs.iter().enumerate() {
        let mut copy = 0;
        walk(&func.body, &mut |n| {
            if let Node::Stmt(Stmt::Prove { id, at, .. }) = n {
                out.entry(*id).or_default().push((f, Region { id: *id, at: *at, copy }));
                copy += 1;
            }
        });
    }
    out
}

/// What went wrong when a site's code can't be found: the compiler lost track of it.
pub(super) enum Lost {
    /// A function site's function isn't a function of the module with a body.
    Func,
    /// No `Prove` block of a loop site is in the function that holds the loop.
    Loop(u32),
}

/// The copies of site `id` (`site`), its own first: for a function its own body, then each block
/// the inliner made of it elsewhere; for a loop, each of its blocks, those in its own function
/// first. A site whose code isn't where it should be is `Lost`, never a site with no copies.
pub(super) fn copies(m: &Module, blocks: &HashMap<u32, Vec<(usize, Region)>>, id: u32, site: &ContractSite) -> Result<Vec<Copy>, Lost> {
    let found = blocks.get(&id).map_or(&[][..], Vec::as_slice);
    let elsewhere = |home: usize| found.iter().filter(move |(g, _)| *g != home).map(|&(func, r)| Copy { func, block: Some(r) });
    match site.target {
        ProveTarget::Func(f) => {
            let home = f.0 as usize;
            if !m.funcs.get(home).is_some_and(|func| func.native.is_none()) {
                return Err(Lost::Func);
            }
            // (A block in the function itself is part of its body, already whole.)
            Ok(std::iter::once(Copy { func: home, block: None }).chain(elsewhere(home)).collect())
        }
        ProveTarget::Loop { func, line } => {
            let home = func.0 as usize;
            let own: Vec<Copy> = found.iter().filter(|(g, _)| *g == home).map(|&(func, r)| Copy { func, block: Some(r) }).collect();
            if own.is_empty() {
                return Err(Lost::Loop(line));
            }
            Ok(own.into_iter().chain(elsewhere(home)).collect())
        }
    }
}

/// What is in one copy: its checks left and its events, and the functions whose code is in it.
pub(super) struct Contents<'a> {
    /// The copy's function first, then each `par for` body it runs and each closure it makes
    /// (whole, and what those run and make in turn), each with the line to give an item that has
    /// none (a `par for` body's items are at the `par for`; 0 keeps the item's own).
    pub funcs: Vec<(usize, u32)>,
    /// The checks left, with the function each is in, function by function.
    pub checks: Vec<(usize, &'a CheckLeft)>,
    /// The events, with the function each is in, function by function.
    pub events: Vec<(usize, &'a Event)>,
}

/// The contents of `copy`, from the analysis `a` and every function's `events`.
pub(super) fn contents<'a>(a: &'a Analysis, events: &'a [Vec<Event>], copy: Copy) -> Contents<'a> {
    let inside = |regions: &[Region]| copy.block.is_none_or(|b| regions.iter().any(|r| r.id == b.id && r.copy == b.copy));
    let mut c = Contents { funcs: vec![(copy.func, 0)], checks: Vec::new(), events: Vec::new() };
    let mut i = 0;
    while i < c.funcs.len() {
        let g = c.funcs[i].0;
        // Every function after the copy's own is in it whole.
        let whole = i > 0;
        c.checks.extend(a.funcs[g].left.iter().filter(|k| whole || inside(&k.regions)).map(|k| (g, k)));
        for e in events[g].iter().filter(|e| whole || inside(&e.regions)) {
            if let What::Body(b) | What::Closure(b) = e.what {
                let b = b.0 as usize;
                if !c.funcs.iter().any(|&(f, _)| f == b) {
                    let at = if matches!(e.what, What::Body(_)) { e.line } else { 0 };
                    c.funcs.push((b, at));
                }
            }
            c.events.push((g, e));
        }
        i += 1;
    }
    c
}
