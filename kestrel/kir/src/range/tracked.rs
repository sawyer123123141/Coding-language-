//! The locals the analysis tracks and the terms its facts are over, each a node of the facts
//! (node 0 is the number zero), with the range each node has when nothing else is known.

use std::cell::Cell;
use std::collections::HashMap;

use super::interval::Interval;
use super::state::State;
use super::terms::{PlaceKey, Term};
use super::MAX_ITEMS;
use crate::ir::*;

/// What a fact node stands for, for the range it falls back to when it is forgotten.
#[derive(Clone, Copy, Debug)]
pub(super) enum NodeKind {
    Zero,
    /// A tracked local, with its slot in the ranges.
    Local(usize),
    /// A list's or array's length: 0 to `MAX_ITEMS`.
    Len,
    /// A loop's end, of this type.
    Hidden(KTy),
}

/// The locals the analysis tracks, and the terms its facts are over. A tracked local is an
/// integer or `ivec4` local that holds its own value (`Value` or `Addressed`), with a slot in every
/// `State`; only an integer one can be a term (have a node). A `RefParam` reads memory, so a load
/// through one is "any other integer load".
pub struct Tracked {
    slot: Vec<Option<u32>>,
    pub(super) tys: Vec<KTy>,
    /// The fact node of each local that is a term.
    node: Vec<Option<usize>>,
    pub(super) kinds: Vec<NodeKind>,
    /// The length terms: each one's place and node.
    pub lens: Vec<(PlaceKey, usize)>,
    len_node: HashMap<PlaceKey, usize>,
    hidden: HashMap<u32, usize>,
    /// The term of each node after zero.
    terms: Vec<Term>,
    /// The work spent on facts so far (entries looked at), and how much the function may spend:
    /// past it, no new facts between terms are made (ranges still are), so a function whose
    /// facts are many and keep changing stays quick.
    work: Cell<u64>,
    work_limit: u64,
}

impl Tracked {
    /// The tracked locals of `f`, with facts over `terms` (each a node, after zero). `hidden_tys`
    /// gives each loop end's type, by the loop's number.
    pub fn of(f: &Func, terms: &[Term], hidden_tys: &[KTy], work_limit: u64) -> Tracked {
        let mut slot = Vec::with_capacity(f.locals.len());
        let mut tys = Vec::new();
        for l in &f.locals {
            // An `ivec4` has one range for its four lanes (`vector.rs`), but is never a term.
            let tracked = (l.ty.is_int() || l.ty == KTy::I32X4) && matches!(l.kind, LocalKind::Value | LocalKind::Addressed);
            slot.push(tracked.then(|| {
                tys.push(l.ty);
                tys.len() as u32 - 1
            }));
        }
        let mut t = Tracked { slot, tys, node: vec![None; f.locals.len()], kinds: vec![NodeKind::Zero], lens: Vec::new(), len_node: HashMap::new(), hidden: HashMap::new(), terms: Vec::new(), work: Cell::new(0), work_limit };
        for term in terms {
            let n = t.kinds.len();
            let kind = match term {
                Term::Local(l) => match t.slot(*l) {
                    Some(s) if t.tys[s].is_int() => {
                        t.node[l.0 as usize] = Some(n);
                        NodeKind::Local(s)
                    }
                    _ => continue,
                },
                Term::Len(k) => {
                    t.lens.push((k.clone(), n));
                    t.len_node.insert(k.clone(), n);
                    NodeKind::Len
                }
                Term::Hidden(i) => {
                    t.hidden.insert(*i, n);
                    NodeKind::Hidden(hidden_tys.get(*i as usize).copied().unwrap_or(KTy::I64))
                }
            };
            t.kinds.push(kind);
            t.terms.push(term.clone());
        }
        t
    }

    pub fn slot(&self, l: LocalIdx) -> Option<usize> {
        self.slot.get(l.0 as usize).copied().flatten().map(|s| s as usize)
    }

    pub fn is_tracked(&self, l: LocalIdx) -> bool {
        self.slot(l).is_some()
    }

    /// The fact node of a local that is a term.
    pub fn node(&self, l: LocalIdx) -> Option<usize> {
        self.node.get(l.0 as usize).copied().flatten()
    }

    /// The node of the length of the list at `k`, when it is a term.
    pub fn len_node(&self, k: &PlaceKey) -> Option<usize> {
        self.len_node.get(k).copied()
    }

    /// The node of loop number `i`'s end, when it is a term.
    pub fn hidden(&self, i: u32) -> Option<usize> {
        self.hidden.get(&i).copied()
    }

    /// The term node `n` stands for (`None` for zero).
    pub fn term(&self, n: usize) -> Option<&Term> {
        n.checked_sub(1).and_then(|i| self.terms.get(i))
    }

    /// Whether the function has spent its work on facts: no new ones are made.
    pub fn saturated(&self) -> bool {
        self.work.get() > self.work_limit
    }

    /// Counts `n` more units of work on facts.
    pub fn spend(&self, n: u64) {
        self.work.set(self.work.get().saturating_add(n));
    }

    /// How many fact nodes there are (zero included).
    pub fn nodes(&self) -> usize {
        self.kinds.len()
    }

    /// The range a node has when nothing else is known about it.
    pub(super) fn node_type(&self, n: usize) -> Interval {
        match self.kinds[n] {
            NodeKind::Zero => Interval::exact(0),
            NodeKind::Local(s) => Interval::of_type(self.tys[s]),
            NodeKind::Len => Interval::new(0, MAX_ITEMS),
            NodeKind::Hidden(ty) => Interval::of_type(ty),
        }
    }


    /// Every tracked local at its type's range, and every term at its own: the state at a
    /// function's start.
    pub fn top(&self) -> State {
        let ranges = self.tys.iter().map(|&t| Interval::of_type(t)).collect();
        State::new(ranges, (0..self.kinds.len()).map(|n| self.node_type(n)).collect())
    }
}
