//! The terms of one function, and what a `par for` body knows when it starts: the runtime hands
//! it a chunk `start..end` of `0..count` (`kestrel_rt_par_for`), with `count` the length of the
//! list the loop goes over, which the body can't change and no other thread does. So the body
//! starts with `0 <= start <= end <= xs.len`, `xs` being its parameter that points at that list
//! (or the place inside it the loop goes over).

use std::collections::HashMap;

use super::facts::ZERO;
use super::state::State;
use super::terms::{self, Term, MAX_TERMS};
use super::Cx;
use crate::ir::*;
use crate::visit::{walk, Node};

/// The list each `par for` body goes over, as a place in the body, by the body's index in
/// `Module::funcs`. A body whose `ParFor`s don't all agree has none.
pub(super) fn lists(m: &Module) -> HashMap<usize, Place> {
    let mut found: HashMap<usize, Option<Place>> = HashMap::new();
    for f in &m.funcs {
        walk(&f.body, &mut |n| {
            if let Node::Stmt(Stmt::ParFor { func, args, count }) = n {
                let i = func.0 as usize;
                let list = m.funcs.get(i).and_then(|body| body_list(body, args, count));
                match found.get(&i) {
                    None => {
                        found.insert(i, list);
                    }
                    Some(old) if *old == list => {}
                    Some(_) => {
                        found.insert(i, None);
                    }
                }
            }
        });
    }
    found.into_iter().filter_map(|(i, p)| p.map(|p| (i, p))).collect()
}

/// The body's list: the `ParFor`'s `count` is `Len(P)`, and `P` with its root renamed to the
/// parameter its argument points at is the same list in the body (`P` may be a temporary the loop
/// made). The rest of `P`'s path must be stable without the caller's locals.
fn body_list(body: &Func, args: &[Arg], count: &Expr) -> Option<Place> {
    let ExprKind::Len(p) = &count.kind else { return None };
    if body.params.len() != args.len() + 2 {
        return None;
    }
    let k = args.iter().position(|a| match a {
        Arg::Ref(q) => q.root == p.root && q.path.len() <= p.path.len() && q.path[..] == p.path[..q.path.len()],
        Arg::Value(_) => false,
    })?;
    let Arg::Ref(q) = &args[k] else { return None };
    let rest = &p.path[q.path.len()..];
    let stable = rest.iter().all(|s| match s {
        Proj::Field(_) | Proj::Variant(..) | Proj::Payload => true,
        Proj::Index { at, .. } => matches!(at.kind, ExprKind::Int(_)),
        Proj::MapKey { .. } | Proj::MapValue { .. } => false,
    });
    let param = body.params[k];
    (stable && body.locals[param.0 as usize].kind == LocalKind::RefParam).then(|| Place { root: param, path: rest.to_vec() })
}

/// The terms of `f` (the pre-scan's, after a `par for` body's `start`, `end` and list length),
/// each loop end's type by the loop's number, and each `ForRange`'s number by its address.
pub(super) fn terms(m: &Module, f: &Func, par_list: Option<&Place>) -> (Vec<Term>, Vec<KTy>, HashMap<usize, u32>) {
    let mut picked = Vec::new();
    if let Some(list) = par_list {
        let n = f.params.len();
        picked.extend([Term::Local(f.params[n - 2]), Term::Local(f.params[n - 1])]);
        picked.extend(terms::list_key(m, f, list, list.path.len()).map(Term::Len));
    }
    for t in terms::pick(m, f) {
        if !picked.contains(&t) {
            picked.push(t);
        }
    }
    picked.truncate(MAX_TERMS);
    let (mut tys, mut numbers) = (Vec::new(), HashMap::new());
    walk(&f.body, &mut |n| {
        if let Node::Stmt(s @ Stmt::ForRange { end, .. }) = n {
            numbers.insert(s as *const Stmt as usize, tys.len() as u32);
            tys.push(end.ty);
        }
    });
    (picked, tys, numbers)
}

impl Cx<'_> {
    /// A `par for` body's facts at its start: `0 <= start <= end <= xs.len`.
    pub(super) fn par_entry(&self, st: &mut State, list: &Place) {
        let n = self.f.params.len();
        let (start, end) = (self.t.node(self.f.params[n - 2]), self.t.node(self.f.params[n - 1]));
        let len = terms::list_key(self.m, self.f, list, list.path.len()).and_then(|k| self.t.len_node(&k));
        if let (Some(s), Some(e), Some(l)) = (start, end, len) {
            st.fact(&self.t, ZERO, s, 0);
            st.fact(&self.t, s, e, 0);
            st.fact(&self.t, e, l, 0);
        }
    }
}
