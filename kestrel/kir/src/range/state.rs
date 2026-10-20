//! What the analysis knows at one point of a function: a range for every tracked integer local.
//! The ranges live in a `Vec` (on the heap), so recursing deep never puts a big state on the stack.

use super::interval::Interval;
use crate::ir::*;

/// The locals the analysis tracks: the integer locals that hold their own value (`Value` or
/// `Addressed`), each with a slot in every `State`. A `RefParam` reads memory, so a load through
/// one is "any other integer load".
pub struct Tracked {
    slot: Vec<Option<u32>>,
    tys: Vec<KTy>,
}

impl Tracked {
    pub fn of(f: &Func) -> Tracked {
        let mut slot = Vec::with_capacity(f.locals.len());
        let mut tys = Vec::new();
        for l in &f.locals {
            let tracked = l.ty.is_int() && matches!(l.kind, LocalKind::Value | LocalKind::Addressed);
            slot.push(tracked.then(|| {
                tys.push(l.ty);
                tys.len() as u32 - 1
            }));
        }
        Tracked { slot, tys }
    }

    pub fn slot(&self, l: LocalIdx) -> Option<usize> {
        self.slot.get(l.0 as usize).copied().flatten().map(|s| s as usize)
    }

    pub fn is_tracked(&self, l: LocalIdx) -> bool {
        self.slot(l).is_some()
    }

    /// Every tracked local at its type's range: the state at a function's start.
    pub fn top(&self) -> State {
        State { dead: false, ranges: self.tys.iter().map(|&t| Interval::of_type(t)).collect() }
    }
}

/// The ranges at one point, or no state at all (`dead`: the point can't be reached).
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    dead: bool,
    ranges: Vec<Interval>,
}

impl State {
    /// The state of a point nothing reaches.
    pub fn dead() -> State {
        State { dead: true, ranges: Vec::new() }
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

    /// Replaces what is known about `l` (a no-op for an untracked local), clipped to its type.
    pub fn set(&mut self, t: &Tracked, l: LocalIdx, v: Interval) {
        if let (false, Some(s)) = (self.dead, t.slot(l)) {
            self.ranges[s] = v.clip(t.tys[s]);
        }
    }

    /// `l` can be anything its type holds: it was written.
    pub fn forget(&mut self, t: &Tracked, l: LocalIdx) {
        if let (false, Some(s)) = (self.dead, t.slot(l)) {
            self.ranges[s] = Interval::of_type(t.tys[s]);
        }
    }

    /// Adds what a condition or a passed check says about `l`: its value is also in `v`. When
    /// the two don't overlap, the point can't be reached.
    pub fn narrow(&mut self, t: &Tracked, l: LocalIdx, v: Interval) {
        if let (false, Some(s)) = (self.dead, t.slot(l)) {
            match self.ranges[s].meet(v) {
                Some(r) => self.ranges[s] = r,
                None => *self = State::dead(),
            }
        }
    }

    /// What holds on either path.
    pub fn join(&self, other: &State) -> State {
        match (self.dead, other.dead) {
            (true, _) => other.clone(),
            (_, true) => self.clone(),
            _ => State { dead: false, ranges: self.ranges.iter().zip(&other.ranges).map(|(a, b)| a.join(*b)).collect() },
        }
    }

    /// Joins `other` into `self`.
    pub fn absorb(&mut self, other: &State) {
        if !other.dead {
            *self = self.join(other);
        }
    }

    /// `self` (a loop's top so far) widened by `new`: see `Interval::widen`.
    pub fn widen(&self, t: &Tracked, new: &State) -> State {
        match (self.dead, new.dead) {
            (true, _) => new.clone(),
            (_, true) => self.clone(),
            _ => State {
                dead: false,
                ranges: self.ranges.iter().zip(&new.ranges).zip(&t.tys).map(|((a, b), &ty)| a.widen(*b, ty)).collect(),
            },
        }
    }
}
