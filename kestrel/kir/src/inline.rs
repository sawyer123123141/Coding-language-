//! Inlines calls to small functions, the way `block_at(y, h).weight()`
//! wants: the callee's body is copied into the caller with its locals
//! renamed and its by-pointer parameters replaced by the argument places.
//! A callee with a contract (see `ContractSite`) has its copy marked: its body and its value's
//! temporary go in a `Stmt::Prove` for each of its sites; the arguments, the caller's code, stay
//! outside.
//!
//! It works bottom-up (see `order`): each function is inlined into once, callees first, and a
//! caller copies its callees as they are after their own inlining, without looking into the copy
//! again. So hot code split into helpers is inlined as if it were written in one piece, and each
//! function's cost to its callers is its size after inlining, worked out once.
//! When a function's budget can't fit all its calls, the calls in loops get it first (see `decide`).
//! Copies nest at most `MAX_NEST` deep in any function, so a long chain of calls doesn't make every
//! function in it as big as the limits allow.

mod decide;
mod descend;
mod order;

use std::collections::HashMap;

use crate::ir::*;
use crate::rename::{rename_expr, rename_stmts, Subst};
use crate::visit::{walk, Node};

/// Callees larger than this many KIR nodes as written stay calls. Big enough for a
/// noise octave (8 lattice lookups), whose constant arguments then fold.
const MAX_NODES: usize = 256;
/// Callees larger than this many KIR nodes after their own inlining stay calls: a function that
/// inlining made big is called, with what it inlined still inlined inside it.
const POST_LIMIT: usize = 4096;
/// How many KIR nodes the copies inlined directly into one function may add, each charged its
/// callee's size after inlining; calls past that stay calls. Twice `POST_LIMIT`, so two copies of
/// the biggest callee fit. With it, a function after inlining is about its own size plus this at
/// most, so code size grows only linearly in the number of functions.
const BUDGET: usize = 8192;
/// How deep copies may nest in a function after its inlining: a copy of a function holding copies
/// that nest `n` deep nests `n + 1` deep. A call whose copy would nest deeper stays a call. Without
/// it, each function of a long chain of small calls (`h0` calls `h1`, which calls `h2`, ...) grows to
/// about `POST_LIMIT`, and a chain 2,000 long took 14 seconds to compile (2026-10-26).
const MAX_NEST: usize = 16;

/// What the callers of a function already inlined into see of it.
struct Done {
    /// Its size in KIR nodes after its own inlining.
    size: usize,
    /// How many locals it had as written: the ones after came from the bodies inlined into it.
    own_locals: usize,
    /// The calls left in it, in the order decided. A copy of it leaves them in its caller too.
    left: Vec<NotInlined>,
    /// How deep the copies in it nest: 0 with none, else one more than the deepest copy's own.
    nest: usize,
}

/// Inlines small functions everywhere in `m`.
pub fn inline_small(m: &mut Module) {
    let sizes: Vec<Result<usize, Why>> = m.funcs.iter().enumerate().map(|(i, f)| inlinable(FuncIdx(i as u32), f)).collect();
    // Each contracted function's sites, in the order listed.
    let mut sites: HashMap<u32, Vec<u32>> = HashMap::new();
    for (id, site) in m.contracts.iter().enumerate() {
        if let ProveTarget::Func(f) = site.target {
            sites.entry(f.0).or_default().push(id as u32);
        }
    }
    let comps = order::callee_first(&m.funcs);
    let mut comp = vec![0; m.funcs.len()];
    for (c, funcs) in comps.iter().enumerate() {
        for f in funcs {
            comp[f.0 as usize] = c;
        }
    }
    let mut done: Vec<Option<Done>> = m.funcs.iter().map(|_| None).collect();
    for funcs in &comps {
        // Functions that call each other inline each other one level, from the bodies as written.
        let mates: Vec<(FuncIdx, Func)> = if funcs.len() > 1 { funcs.iter().map(|&f| (f, m.funcs[f.0 as usize].clone())).collect() } else { Vec::new() };
        for &caller in funcs {
            let i = caller.0 as usize;
            let mut body = std::mem::take(&mut m.funcs[i].body);
            let locals = std::mem::take(&mut m.funcs[i].locals);
            let own_locals = locals.len();
            let line = m.funcs[i].lines.0;
            let mode = decide::Mode::Plan(Vec::new());
            let mut cx = Inliner { m, done: &done, comp: &comp, mates: &mates, sizes: &sizes, sites: &sites, locals, budget: BUDGET, caller, line, log: Vec::new(), in_mate: false, depth: 0, nest: 0, mode };
            cx.run(&mut body);
            let Inliner { locals, log, nest, .. } = cx;
            let mut size = 0;
            walk(&body, &mut |_| size += 1);
            m.funcs[i].body = body;
            m.funcs[i].locals = locals;
            done[i] = Some(Done { size, own_locals, left: log, nest });
        }
    }
    // Function by function, each in the order decided.
    m.not_inlined = done.into_iter().flat_map(|d| d.expect("every function is inlined into").left).collect();
}

/// Why a function can't be inlined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    /// A native: its body is Rust. The call stays and is not worth a note.
    Native,
    Recursive,
    EarlyReturn,
    /// This many nodes.
    Big(usize),
}

impl Why {
    fn text(self) -> String {
        match self {
            Why::Native => "it is implemented in Rust".to_string(),
            Why::Recursive => "it calls itself".to_string(),
            Why::EarlyReturn => "it has an early `return`".to_string(),
            Why::Big(n) => format!("it is {n} nodes; the limit is {MAX_NODES}"),
        }
    }
}

/// The size in KIR nodes of a function that can be inlined: small, not
/// directly recursive, and with no `return` except a final one (so the body
/// can be spliced in place of the call). Otherwise why not.
fn inlinable(idx: FuncIdx, f: &Func) -> Result<usize, Why> {
    // A native's body is empty, but the call does something: it stays a call.
    if f.native.is_some() {
        return Err(Why::Native);
    }
    let last = matches!(f.body.last(), Some(Stmt::Return(Some(_))));
    if !last && f.ret.is_some() {
        return Err(Why::EarlyReturn);
    }
    let mut nodes = 0;
    let mut ok = true;
    let mut returns = 0;
    walk(&f.body, &mut |s| {
        nodes += 1;
        match s {
            Node::Stmt(Stmt::Return(_)) => returns += 1,
            Node::Expr(Expr { kind: ExprKind::Call(callee, _), .. }) | Node::Stmt(Stmt::Call(callee, _, _)) => {
                ok &= *callee != idx;
            }
            _ => {}
        }
    });
    if !ok {
        return Err(Why::Recursive);
    }
    // Every `return` anywhere in the body counts, including one inside the
    // final return's own value (a tail `if` or `match` with a `return` in a
    // branch): only the final statement itself may be one.
    if returns != usize::from(last) {
        return Err(Why::EarlyReturn);
    }
    if nodes > MAX_NODES {
        return Err(Why::Big(nodes));
    }
    Ok(nodes)
}

struct Inliner<'a> {
    /// The module: every function outside the caller's component is already inlined into (the
    /// caller's own body and locals are taken out while it is).
    m: &'a Module,
    /// What callers see of each function already inlined into.
    done: &'a [Option<Done>],
    /// Each function's component (see `order::callee_first`).
    comp: &'a [usize],
    /// The caller's component as written, when it has more than one function (mutual recursion).
    mates: &'a [(FuncIdx, Func)],
    /// Each function's size as written when it can be inlined, or why not.
    sizes: &'a [Result<usize, Why>],
    /// The contract sites of each function that has one (see `ProveTarget::Func`).
    sites: &'a HashMap<u32, Vec<u32>>,
    /// The caller's locals; inlined callees add theirs.
    locals: Vec<Local>,
    /// Nodes the caller may still grow by.
    budget: usize,
    /// The function being inlined into.
    caller: FuncIdx,
    /// The packed line of the latest expression seen: the line of a call statement with no
    /// argument that has one (see `NotInlined::line`).
    line: u32,
    /// The calls left in place, in the order they were decided (see `NotInlined`).
    log: Vec<NotInlined>,
    /// Inside the copy of a function from the caller's own component: calls back into the
    /// component stay calls, and the calls don't compete for the budget (see `decide`).
    in_mate: bool,
    /// How many loops the code being walked is in.
    depth: usize,
    /// How deep the copies made so far nest (see `Done::nest`).
    nest: usize,
    /// Whether the walk lists the calls that compete for the budget or inlines them.
    mode: decide::Mode,
}

impl Inliner<'_> {
    fn temp(&mut self, name: String, ty: KTy, kind: LocalKind) -> LocalIdx {
        self.locals.push(Local { name, ty, kind });
        LocalIdx(self.locals.len() as u32 - 1)
    }

    fn stmts(&mut self, stmts: &mut Vec<Stmt>) {
        let mut out = Vec::with_capacity(stmts.len());
        for mut s in std::mem::take(stmts) {
            if let Stmt::Call(fi, args, call) = &mut s {
                let line = if *call != 0 { *call } else { crate::visit::args_line(args) };
                if self.take(*fi, line) {
                    let (fi, args) = (*fi, std::mem::take(args));
                    let (mut body, value) = self.expand(fi, args, line);
                    if let Some(v) = value {
                        // A discarded result still has to be computed (for its effects).
                        let tmp = self.temp("discard".into(), v.ty, local_kind(v.ty));
                        body.push(Stmt::Let(tmp, v));
                    }
                    out.extend(body);
                    continue;
                }
            }
            self.stmt(&mut s);
            out.push(s);
        }
        *stmts = out;
    }

    /// The callee's body, renamed into the caller, and its returned value: as they are after its
    /// own inlining, or for a function of the caller's own component, as written with their calls
    /// inlined here.
    /// Arguments are evaluated first, in order, except a by-value one the callee never mentions
    /// whose evaluation has no effect (see `no_effect`): a closure body that reads no capture
    /// doesn't load its environment word. `line` is the call's packed line.
    fn expand(&mut self, fi: FuncIdx, args: Vec<Arg>, line: u32) -> (Vec<Stmt>, Option<Expr>) {
        // As `take` notes a call with no line of its own.
        let at = if line == 0 { self.line } else { line };
        let (funcs, mates) = (&self.m.funcs, self.mates);
        let done = if self.comp[fi.0 as usize] == self.comp[self.caller.0 as usize] { None } else { self.done[fi.0 as usize].as_ref() };
        // A copy inside the copy of a function of the caller's own component is one deeper.
        self.nest = self.nest.max(usize::from(self.in_mate) + 1 + done.map_or(0, |d| d.nest));
        let callee = match done {
            Some(_) => &funcs[fi.0 as usize],
            None => &mates.iter().find(|(g, _)| *g == fi).expect("a function of the caller's component").1,
        };
        // A local the callee got from its own inlining keeps its name, which already names the
        // function it was written in; the callee's own locals get the callee's name in front.
        let own = done.map_or(callee.locals.len(), |d| d.own_locals);
        let name = |i: usize, local: &Local| if i < own { format!("{}.{}", callee.name, local.name) } else { local.name.clone() };
        let mut mentioned = vec![false; callee.locals.len()];
        walk(&callee.body, &mut |n| {
            if let Node::Place(place, _) = n {
                mentioned[place.root.0 as usize] = true;
            }
        });
        let mut pre = Vec::new();
        let mut subst: Vec<Option<Subst>> = vec![None; callee.locals.len()];
        for (&p, mut arg) in callee.params.iter().zip(args) {
            // The arguments belong to the caller: inline their calls too.
            match &mut arg {
                Arg::Value(v) => self.expr(v),
                Arg::Ref(place) => self.place(place),
            }
            let local = &callee.locals[p.0 as usize];
            let name = name(p.0 as usize, local);
            subst[p.0 as usize] = Some(match (local.kind, arg) {
                (LocalKind::RefParam, Arg::Ref(place)) => Subst::Place(self.stable(place, &mut pre)),
                (LocalKind::RefParam, Arg::Value(Expr { kind: ExprKind::Load(place), .. })) => {
                    Subst::Place(self.stable(place, &mut pre))
                }
                (LocalKind::RefParam, Arg::Value(v)) => {
                    let tmp = self.temp(name, v.ty, LocalKind::Addressed);
                    pre.push(Stmt::Let(tmp, v));
                    Subst::Place(Place::local(tmp))
                }
                // Never read, and nothing to run: the parameter stays unset.
                (_, Arg::Value(v)) if !mentioned[p.0 as usize] && no_effect(&v) => Subst::Local(self.temp(name, local.ty, local.kind)),
                (_, Arg::Value(v)) => {
                    let tmp = self.temp(name, local.ty, local.kind);
                    pre.push(Stmt::Let(tmp, v));
                    Subst::Local(tmp)
                }
                (_, Arg::Ref(_)) => unreachable!("a `var` argument goes to a by-pointer parameter"),
            });
        }
        for (i, local) in callee.locals.iter().enumerate() {
            if subst[i].is_none() {
                subst[i] = Some(Subst::Local(self.temp(name(i, local), local.ty, local.kind)));
            }
        }
        let subst: Vec<Subst> = subst.into_iter().map(|s| s.expect("every local mapped")).collect();
        let mut body = callee.body.clone();
        let mut value = match body.last() {
            Some(Stmt::Return(Some(_))) => match body.pop() {
                Some(Stmt::Return(Some(v))) => Some(v),
                _ => unreachable!("just checked"),
            },
            _ => None,
        };
        rename_stmts(&mut body, &subst);
        if let Some(v) = &mut value {
            rename_expr(v, &subst);
        }
        match done {
            // Already inlined into, and not looked into again: the calls left in it are now the
            // caller's too.
            Some(d) => {
                let caller = self.caller;
                self.log.extend(d.left.iter().map(|n| NotInlined { caller, ..n.clone() }));
            }
            // As written: inline the calls it makes, except back into the component.
            None => {
                let was = std::mem::replace(&mut self.in_mate, true);
                self.stmts(&mut body);
                if let Some(v) = &mut value {
                    self.expr(v);
                }
                self.in_mate = was;
            }
        }
        // (A program without contracts has no sites: nothing to look up.)
        let sites = if self.sites.is_empty() { None } else { self.sites.get(&fi.0) };
        if let Some(ids) = sites {
            let (marked, v) = self.marked(ids, at, body, value);
            (body, value) = (marked, v);
        }
        pre.extend(body);
        (pre, value)
    }

    /// A contracted callee's expanded `body` and `value` as its sites' `Prove` blocks, the first
    /// site outermost: the value is computed into a temporary inside them, and read after. A value
    /// that is a read of a whole local is read after them as it is (it has no check and makes no
    /// event, and nothing between changes the local): a temporary would hide the length of a
    /// returned list from range analysis, which tracks it only for locals it indexes or measures.
    fn marked(&mut self, ids: &[u32], at: u32, mut body: Vec<Stmt>, value: Option<Expr>) -> (Vec<Stmt>, Option<Expr>) {
        let value = value.map(|v| {
            if v.is_local_read() {
                return v;
            }
            let (ty, line) = (v.ty, v.line);
            let tmp = self.temp("result".into(), ty, local_kind(ty));
            body.push(Stmt::Let(tmp, v));
            Expr { kind: ExprKind::Load(Place::local(tmp)), ty, line }
        });
        for &id in ids.iter().rev() {
            body = vec![Stmt::Prove { id, at, body }];
        }
        (body, value)
    }

    /// `place` with its index expressions evaluated once, now, so the
    /// inlined body can use it many times.
    fn stable(&mut self, mut place: Place, pre: &mut Vec<Stmt>) -> Place {
        for at in place.path.iter_mut().filter_map(Proj::at_mut) {
            if !matches!(at.kind, ExprKind::Int(_)) {
                let tmp = self.temp("index".into(), at.ty, LocalKind::Value);
                let (ty, line) = (at.ty, at.line);
                let v = std::mem::replace(at, Expr { kind: ExprKind::Load(Place::local(tmp)), ty, line });
                pre.push(Stmt::Let(tmp, v));
            }
        }
        place
    }
}

/// Whether evaluating `e` has no effect and can't fail: a constant, the environment word of a
/// function value in a local, or a read of a whole local.
fn no_effect(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) => true,
        ExprKind::FnEnv(p) | ExprKind::Load(p) => p.path.is_empty(),
        _ => false,
    }
}

fn local_kind(ty: KTy) -> LocalKind {
    if ty.is_aggregate() {
        LocalKind::Addressed
    } else {
        LocalKind::Value
    }
}
