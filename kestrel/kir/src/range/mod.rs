//! Range analysis: proves which overflow, shift, division and narrowing checks can never fail,
//! marks them (`ExprKind::Proven`), and says which checks are left and why. It runs on KIR after
//! `optimize`, one function at a time, interpreting each function abstractly: a range for every
//! tracked integer local, one statement at a time (see `stmt`), conditions refining the ranges
//! (`refine`), loops repeated to a fixed point (`loops`). See the range-analysis design spec.

mod eval;
pub mod interval;
mod loops;
mod refine;
pub mod show;
pub mod sites;
pub mod state;
mod stmt;
pub mod writes;

use std::collections::HashMap;

use crate::ir::*;
use crate::visit::{walk, Node};
use crate::visit_mut::{walk_expr, walk_place, VisitMut};
use interval::Interval;
pub use sites::{Arith, CheckKind};
use state::{State, Tracked};

/// The most items a list or an array holds (the runtime refuses more), so every `xs.len` of one
/// fits an `int`.
pub const MAX_ITEMS: i128 = 2_147_483_647;

/// How deep statements and expressions may nest before the analysis gives a function up.
const MAX_DEPTH: u32 = 256;

/// What range analysis found in a module: one entry per `Module::funcs` (empty for natives).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Analysis {
    pub funcs: Vec<FuncChecks>,
}

/// The checks left in one function, in the order the code runs them, and how many were removed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FuncChecks {
    pub left: Vec<CheckLeft>,
    pub removed: u32,
    /// The function nests too deeply or took too long to analyse: it was left exactly as it was,
    /// with every check in `left`.
    pub gave_up: bool,
}

/// A check the analysis couldn't prove.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckLeft {
    pub kind: CheckKind,
    /// The packed line of the expression (see `pack_line`).
    pub line: u32,
    /// The packed lines of the loops around it, outermost first: a `ForRange`'s `start.line`, a
    /// `While`'s `cond.line`.
    pub loops: Vec<u32>,
    /// Why it may fail, worded for the report after the kind: "`a * b` may exceed `int` (`a` can be
    /// any `int`; `b` can be any `int`)".
    pub text: String,
}

/// Analyses every function with a body, marks the checks it proves, and returns what is left.
pub fn remove_checks(m: &mut Module, arith: Arith) -> Analysis {
    let mut funcs = Vec::with_capacity(m.funcs.len());
    for i in 0..m.funcs.len() {
        if m.funcs[i].native.is_some() {
            funcs.push(FuncChecks::default());
            continue;
        }
        let (checks, proven) = analyse(m, &m.funcs[i], arith);
        if !proven.is_empty() {
            Marker { proven: &proven }.visit_stmts(&mut m.funcs[i].body);
        }
        funcs.push(checks);
    }
    Analysis { funcs }
}

/// The statement visits a function may take over all passes: 200,000, or 100 per statement when
/// that is more.
fn budget(f: &Func) -> u32 {
    let mut statements: u32 = 0;
    walk(&f.body, &mut |n| {
        if let Node::Stmt(_) = n {
            statements = statements.saturating_add(1);
        }
    });
    statements.saturating_mul(100).max(200_000)
}

/// A check site: an arithmetic node, or an index projection (`bounds`), by address. The two kinds
/// are kept apart because an index expression may share its projection's address.
type Key = (usize, bool);

fn expr_key(e: &Expr) -> Key {
    (e as *const Expr as usize, false)
}

fn index_key(p: &Proj) -> Key {
    (p as *const Proj as usize, true)
}

/// One function's checks, and the sites it proved.
fn analyse(m: &Module, f: &Func, arith: Arith) -> (FuncChecks, HashMap<Key, ()>) {
    let mut cx = Cx::new(m, f, arith);
    let start = cx.t.top();
    let _ = cx.stmts(start, &f.body);
    if cx.gave_up {
        let left = every_site(m, f, arith);
        return (FuncChecks { left, removed: 0, gave_up: true }, HashMap::new());
    }
    let mut proven = HashMap::new();
    let mut left = Vec::new();
    for d in std::mem::take(&mut cx.order) {
        match d.left {
            None => {
                proven.insert(d.key, ());
            }
            Some(c) => left.push(c),
        }
    }
    // Every site is decided in some deciding pass; one that wasn't (a gap in the interpreter's
    // walk) keeps its check and is listed at the end. Its text is only made then.
    for (key, site) in sites_of(f, arith) {
        if !cx.decisions.contains_key(&key) {
            let c = site.left(m, f);
            debug_assert!(false, "range analysis never decided a {:?} check in `{}`: {}", c.kind, f.name, c.text);
            left.push(c);
        }
    }
    let removed = proven.len() as u32;
    (FuncChecks { left, removed, gave_up: false }, proven)
}

/// A check site found by walking a function: an arithmetic node's check, or step `i` of a place.
enum Site<'a> {
    Expr(CheckKind, &'a Expr),
    Bounds(&'a Place, usize),
}

impl Site<'_> {
    /// The site as a check left, with the text of a check nothing is known about.
    fn left(&self, m: &Module, f: &Func) -> CheckLeft {
        match *self {
            Site::Expr(kind, e) => CheckLeft { kind, line: e.line, loops: Vec::new(), text: show::check_text(m, f, kind, e, &[]) },
            Site::Bounds(p, i) => {
                let line = p.path[i].at().map_or(0, |at| at.line);
                CheckLeft { kind: CheckKind::Bounds, line, loops: Vec::new(), text: show::bounds_text(m, f, p, i, None) }
            }
        }
    }
}

/// Every check site in `f`, in walk order. The node inside a `Proven` (a module analysed before)
/// has no checks left, so it is no site.
fn sites_of(f: &Func, arith: Arith) -> Vec<(Key, Site<'_>)> {
    let mut out = Vec::new();
    let mut inside_proven: Option<*const Expr> = None;
    walk(&f.body, &mut |n| match n {
        Node::Expr(e) => {
            if let ExprKind::Proven(inner) = &e.kind {
                inside_proven = Some(&**inner as *const Expr);
            }
            if inside_proven == Some(e as *const Expr) {
                inside_proven = None;
                return;
            }
            for kind in sites::checks_of(e, arith) {
                out.push((expr_key(e), Site::Expr(kind, e)));
            }
        }
        Node::Place(p, _) => {
            for (i, proj) in p.path.iter().enumerate() {
                if sites::is_bounds_site(proj) {
                    out.push((index_key(proj), Site::Bounds(p, i)));
                }
            }
        }
        Node::Stmt(_) => {}
    });
    out
}

/// Every check site in `f`, kept: what a function the analysis gave up on reports.
fn every_site(m: &Module, f: &Func, arith: Arith) -> Vec<CheckLeft> {
    sites_of(f, arith).into_iter().map(|(_, s)| s.left(m, f)).collect()
}

/// Wraps every proven arithmetic node in `Proven`, and marks every proven index.
struct Marker<'a> {
    proven: &'a HashMap<Key, ()>,
}

impl VisitMut for Marker<'_> {
    fn visit_expr(&mut self, e: &mut Expr) {
        let key = expr_key(e);
        walk_expr(self, e);
        if self.proven.contains_key(&key) {
            let (ty, line) = (e.ty, e.line);
            let inner = std::mem::replace(e, Expr { kind: ExprKind::Bool(false), ty, line });
            *e = Expr { kind: ExprKind::Proven(Box::new(inner)), ty, line };
        }
    }

    fn visit_place(&mut self, p: &mut Place) {
        for proj in &mut p.path {
            if self.proven.contains_key(&index_key(proj)) {
                if let Proj::Index { check, .. } = proj {
                    *check = IndexCheck::Proven;
                }
            }
        }
        walk_place(self, p);
    }
}

/// What happened to one check site in the deciding passes.
struct Decision {
    key: Key,
    /// `None` when proven; otherwise what is left.
    left: Option<CheckLeft>,
}

/// The states that leave a loop's body other than by its end.
#[derive(Default)]
struct Frame {
    breaks: Option<State>,
    continues: Option<State>,
}

/// The interpreter of one function.
struct Cx<'a> {
    m: &'a Module,
    f: &'a Func,
    t: Tracked,
    arith: Arith,
    /// Whether every loop around the current point is in its deciding pass (checks are decided
    /// only then).
    deciding: bool,
    /// More than 0 inside code the analysis found can't run: its checks stay, and nothing flows
    /// out of it.
    keep: u32,
    visits: u32,
    budget: u32,
    depth: u32,
    gave_up: bool,
    /// The loops around the current point, innermost last.
    frames: Vec<Frame>,
    /// The packed lines of their headers.
    loop_lines: Vec<u32>,
    /// Facts from checks that passed on every path of the statements being analysed: each local's
    /// value is also in the range. A statement takes the ones its operands added (step 4).
    learned: Vec<(LocalIdx, Interval)>,
    decisions: HashMap<Key, usize>,
    order: Vec<Decision>,
}

impl<'a> Cx<'a> {
    fn new(m: &'a Module, f: &'a Func, arith: Arith) -> Self {
        Cx {
            m,
            f,
            t: Tracked::of(f),
            arith,
            deciding: true,
            keep: 0,
            visits: 0,
            budget: budget(f),
            depth: 0,
            gave_up: false,
            frames: Vec::new(),
            loop_lines: Vec::new(),
            learned: Vec::new(),
            decisions: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Enters a statement (`stmt`, counted against the budget) or an expression. `false` when the
    /// analysis has given the function up, now or before; otherwise the caller must `leave`.
    fn enter(&mut self, stmt: bool) -> bool {
        if self.gave_up {
            return false;
        }
        if stmt {
            self.visits += 1;
        }
        if self.visits > self.budget || self.depth >= MAX_DEPTH {
            self.gave_up = true;
            return false;
        }
        self.depth += 1;
        true
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    /// Records the decision about a check site, in a deciding pass: proven when `safe` (outside
    /// unreachable code), otherwise left with `text()`. A site decided twice is proven only if
    /// both decisions prove it.
    fn decide(&mut self, key: Key, kind: CheckKind, line: u32, safe: bool, text: impl FnOnce(&Self) -> String) {
        if !self.deciding {
            return;
        }
        let proven = safe && self.keep == 0;
        let left = (!proven).then(|| CheckLeft { kind, line, loops: self.loop_lines.clone(), text: text(self) });
        match self.decisions.get(&key) {
            Some(&i) => {
                if self.order[i].left.is_none() {
                    self.order[i].left = left;
                }
            }
            None => {
                self.decisions.insert(key, self.order.len());
                self.order.push(Decision { key, left });
            }
        }
    }

    /// Forgets every tracked local `writes` may change.
    fn forget(&self, st: &mut State, writes: &[writes::Write]) {
        for w in writes {
            st.forget(&self.t, w.root());
        }
    }

    /// The type of the value stored at a place.
    fn place_ty(&self, p: &Place) -> KTy {
        let mut ty = self.f.locals[p.root.0 as usize].ty;
        for proj in &p.path {
            ty = match (proj, ty) {
                (Proj::Field(fi), KTy::Struct(s)) => self.m.layout(s).fields[*fi as usize].ty,
                (Proj::Variant(v, fi), KTy::Enum(e)) => self.m.enum_layout(e).variants[*v as usize].fields[*fi as usize].ty,
                (Proj::Index { .. }, KTy::Array(el) | KTy::List(el)) | (Proj::Payload, KTy::Optional(el)) => self.m.elem(el),
                (Proj::MapKey { .. }, KTy::Map(k, _)) => self.m.elem(k),
                (Proj::MapValue { .. }, KTy::Map(_, v)) => self.m.elem(v),
                _ => return KTy::I64,
            };
        }
        ty
    }
}
