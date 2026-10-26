//! The order the inliner visits functions in: the call graph's strongly connected components
//! (Tarjan's algorithm), callees first, so every callee outside a function's own component is
//! already inlined into when the function is.

use crate::ir::*;
use crate::visit::{walk, Node};

/// The components of the direct-call graph of `funcs`, each after every component it calls; the
/// functions of a component (a cycle of calls, or one function) in index order. A closure's call
/// is `Call(code, [FnEnv(f), ..])`, a direct call of its code, so it counts; calls through a
/// function value don't (the inliner can't inline them).
pub(super) fn callee_first(funcs: &[Func]) -> Vec<Vec<FuncIdx>> {
    let edges: Vec<Vec<usize>> = funcs.iter().map(callees).collect();
    let n = funcs.len();
    let mut t = Tarjan { index: vec![UNSEEN; n], low: vec![0; n], on_stack: vec![false; n], stack: Vec::new(), next: 0, out: Vec::new() };
    for root in 0..n {
        if t.index[root] == UNSEEN {
            t.visit(&edges, root);
        }
    }
    t.out
}

/// The functions `f` calls directly, each once.
fn callees(f: &Func) -> Vec<usize> {
    let mut out = Vec::new();
    walk(&f.body, &mut |n| {
        if let Node::Expr(Expr { kind: ExprKind::Call(c, _), .. }) | Node::Stmt(Stmt::Call(c, _, _)) = n {
            out.push(c.0 as usize);
        }
    });
    out.sort_unstable();
    out.dedup();
    out
}

const UNSEEN: u32 = u32::MAX;

struct Tarjan {
    /// The order each function was first reached in (`UNSEEN` before).
    index: Vec<u32>,
    /// The lowest index reachable from each function through the functions still on the stack.
    low: Vec<u32>,
    on_stack: Vec<bool>,
    stack: Vec<usize>,
    next: u32,
    /// The components found, callees first.
    out: Vec<Vec<FuncIdx>>,
}

impl Tarjan {
    /// Tarjan's depth-first search from `root`, with its own stack of (function, next edge) so a
    /// long chain of calls can't overflow the compiler's.
    fn visit(&mut self, edges: &[Vec<usize>], root: usize) {
        let mut path: Vec<(usize, usize)> = vec![(root, 0)];
        self.enter(root);
        while let Some(top) = path.last_mut() {
            let v = top.0;
            if let Some(&w) = edges[v].get(top.1) {
                top.1 += 1;
                if self.index[w] == UNSEEN {
                    self.enter(w);
                    path.push((w, 0));
                } else if self.on_stack[w] {
                    self.low[v] = self.low[v].min(self.index[w]);
                }
                continue;
            }
            path.pop();
            if let Some(&(parent, _)) = path.last() {
                self.low[parent] = self.low[parent].min(self.low[v]);
            }
            if self.low[v] == self.index[v] {
                let mut comp = Vec::new();
                loop {
                    let w = self.stack.pop().expect("`v` is on the stack");
                    self.on_stack[w] = false;
                    comp.push(FuncIdx(w as u32));
                    if w == v {
                        break;
                    }
                }
                comp.sort_unstable_by_key(|f| f.0);
                self.out.push(comp);
            }
        }
    }

    fn enter(&mut self, v: usize) {
        self.index[v] = self.next;
        self.low[v] = self.next;
        self.next += 1;
        self.stack.push(v);
        self.on_stack[v] = true;
    }
}
