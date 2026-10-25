//! A graph over placeholders for the expansion checks (G10): the type check (`expand.rs`) and
//! the function check (Task 5). An edge says a placeholder feeds another; it is growing when the
//! argument isn't exactly the placeholder (`list[T]` for `T`). A strongly connected component
//! with a growing edge inside would need endless copies.

use std::collections::{HashMap, HashSet};

use crate::generic::ParamId;

/// An edge of the placeholder graph: `from` feeds `to`; `site` is the caller's note of where.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Edge {
    pub from: ParamId,
    pub to: ParamId,
    pub growing: bool,
    pub site: usize,
}

/// A graph over placeholders.
#[derive(Default)]
pub(crate) struct Graph {
    pub edges: Vec<Edge>,
}

impl Graph {
    pub fn add(&mut self, from: ParamId, to: ParamId, growing: bool, site: usize) {
        self.edges.push(Edge { from, to, growing, site });
    }

    /// The strongly connected components with a growing edge inside, each as its placeholders
    /// (sorted), in the order of their smallest placeholder.
    pub fn growing_cycles(&self) -> Vec<Vec<ParamId>> {
        let mut nodes: Vec<ParamId> = self.edges.iter().flat_map(|e| [e.from, e.to]).collect();
        nodes.sort();
        nodes.dedup();
        let mut out: HashMap<ParamId, Vec<ParamId>> = HashMap::new();
        for e in &self.edges {
            out.entry(e.from).or_default().push(e.to);
        }
        let comp = components(&nodes, &out);
        let mut cycles: Vec<Vec<ParamId>> = Vec::new();
        let mut done: HashSet<usize> = HashSet::new();
        for e in self.edges.iter().filter(|e| e.growing && comp[&e.from] == comp[&e.to]) {
            if done.insert(comp[&e.from]) {
                let mut members: Vec<ParamId> = nodes.iter().copied().filter(|n| comp[n] == comp[&e.from]).collect();
                members.sort();
                cycles.push(members);
            }
        }
        cycles.sort();
        cycles
    }
}

/// Tarjan's strongly connected components: each node's component number.
fn components(nodes: &[ParamId], out: &HashMap<ParamId, Vec<ParamId>>) -> HashMap<ParamId, usize> {
    struct State<'a> {
        out: &'a HashMap<ParamId, Vec<ParamId>>,
        index: HashMap<ParamId, usize>,
        low: HashMap<ParamId, usize>,
        stack: Vec<ParamId>,
        on_stack: HashSet<ParamId>,
        comp: HashMap<ParamId, usize>,
        next: usize,
        comps: usize,
    }
    fn visit(s: &mut State, v: ParamId) {
        s.index.insert(v, s.next);
        s.low.insert(v, s.next);
        s.next += 1;
        s.stack.push(v);
        s.on_stack.insert(v);
        for &w in s.out.get(&v).map(Vec::as_slice).unwrap_or(&[]) {
            if !s.index.contains_key(&w) {
                visit(s, w);
                let low = s.low[&v].min(s.low[&w]);
                s.low.insert(v, low);
            } else if s.on_stack.contains(&w) {
                let low = s.low[&v].min(s.index[&w]);
                s.low.insert(v, low);
            }
        }
        if s.low[&v] == s.index[&v] {
            while let Some(w) = s.stack.pop() {
                s.on_stack.remove(&w);
                s.comp.insert(w, s.comps);
                if w == v {
                    break;
                }
            }
            s.comps += 1;
        }
    }
    let mut s = State {
        out,
        index: HashMap::new(),
        low: HashMap::new(),
        stack: Vec::new(),
        on_stack: HashSet::new(),
        comp: HashMap::new(),
        next: 0,
        comps: 0,
    };
    for &n in nodes {
        if !s.index.contains_key(&n) {
            visit(&mut s, n);
        }
    }
    s.comp
}
