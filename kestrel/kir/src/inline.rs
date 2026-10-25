//! Inlines calls to small functions, the way `block_at(y, h).weight()`
//! wants: the callee's body is copied into the caller with its locals
//! renamed and its by-pointer parameters replaced by the argument places.
//! A callee with a contract (see `ContractSite`) has its copy marked: its body and its value's
//! temporary go in a `Stmt::Prove` for each of its sites; the arguments, the caller's code, stay
//! outside.

mod descend;

use std::collections::HashMap;

use crate::ir::*;
use crate::rename::{rename_expr, rename_stmts, Subst};
use crate::visit::{walk, Node};

/// Callees larger than this many KIR nodes stay calls. Big enough for a
/// noise octave (8 lattice lookups), whose constant arguments then fold.
const MAX_NODES: usize = 256;
/// How many KIR nodes inlining may add to one function; calls past that
/// stay calls, so nested inlining can't blow up code size.
const GROWTH_BUDGET: usize = 4096;
/// How deep inlined bodies are themselves inlined (the budget also bounds
/// mutual recursion; this keeps a chain like solids -> density -> octave ->
/// lattice -> hash whole even when it is itself inlined).
const MAX_DEPTH: u32 = 8;

/// Inlines small functions everywhere in `m`.
pub fn inline_small(m: &mut Module) {
    let originals = m.funcs.clone();
    let sizes: Vec<Result<usize, Why>> = originals.iter().enumerate().map(|(i, f)| inlinable(FuncIdx(i as u32), f)).collect();
    let mut log = Vec::new();
    // Each contracted function's sites, in the order listed.
    let mut sites: HashMap<u32, Vec<u32>> = HashMap::new();
    for (id, site) in m.contracts.iter().enumerate() {
        if let ProveTarget::Func(f) = site.target {
            sites.entry(f.0).or_default().push(id as u32);
        }
    }
    for (i, f) in m.funcs.iter_mut().enumerate() {
        let locals = std::mem::take(&mut f.locals);
        let caller = FuncIdx(i as u32);
        let mut cx = Inliner { originals: &originals, sizes: &sizes, sites: &sites, locals, budget: GROWTH_BUDGET, caller, line: f.lines.0, log: &mut log };
        cx.stmts(&mut f.body, 0);
        f.locals = cx.locals;
    }
    m.not_inlined = log;
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
    originals: &'a [Func],
    /// Each function's size when it can be inlined, or why not.
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
    log: &'a mut Vec<NotInlined>,
}

impl Inliner<'_> {
    /// True when a call to `fi` at `depth` (written on `line`) should be inlined; takes its size
    /// from the budget. Otherwise notes the call and why in the log.
    fn take(&mut self, fi: FuncIdx, depth: u32, line: u32) -> bool {
        let reason = match self.sizes[fi.0 as usize] {
            Ok(_) if depth >= MAX_DEPTH => format!("inlined calls nest more than {MAX_DEPTH} deep"),
            Ok(size) if size <= self.budget => {
                self.budget -= size;
                return true;
            }
            Ok(_) => format!("the inlining budget of `{}` ran out", self.originals[self.caller.0 as usize].name),
            Err(Why::Native) => return false,
            Err(why) => why.text(),
        };
        let line = if line == 0 { self.line } else { line };
        self.log.push(NotInlined { caller: self.caller, line, callee: fi, reason });
        false
    }

    fn temp(&mut self, name: String, ty: KTy, kind: LocalKind) -> LocalIdx {
        self.locals.push(Local { name, ty, kind });
        LocalIdx(self.locals.len() as u32 - 1)
    }

    fn stmts(&mut self, stmts: &mut Vec<Stmt>, depth: u32) {
        let mut out = Vec::with_capacity(stmts.len());
        for mut s in std::mem::take(stmts) {
            if let Stmt::Call(fi, args, call) = &mut s {
                let line = if *call != 0 { *call } else { crate::visit::args_line(args) };
                if self.take(*fi, depth, line) {
                    let (fi, args) = (*fi, std::mem::take(args));
                    let (mut body, value) = self.expand(fi, args, depth, line);
                    if let Some(v) = value {
                        // A discarded result still has to be computed (for its effects).
                        let tmp = self.temp("discard".into(), v.ty, local_kind(v.ty));
                        body.push(Stmt::Let(tmp, v));
                    }
                    out.extend(body);
                    continue;
                }
            }
            self.stmt(&mut s, depth);
            out.push(s);
        }
        *stmts = out;
    }

    /// The callee's body, renamed into the caller, and its returned value.
    /// Arguments are evaluated first, in order, except a by-value one the callee never mentions
    /// whose evaluation has no effect (see `no_effect`): a closure body that reads no capture
    /// doesn't load its environment word. `line` is the call's packed line.
    fn expand(&mut self, fi: FuncIdx, args: Vec<Arg>, depth: u32, line: u32) -> (Vec<Stmt>, Option<Expr>) {
        // As `take` notes a call with no line of its own.
        let at = if line == 0 { self.line } else { line };
        let callee = &self.originals[fi.0 as usize];
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
                Arg::Value(v) => self.expr(v, depth),
                Arg::Ref(place) => self.place(place, depth),
            }
            let local = &callee.locals[p.0 as usize];
            let name = format!("{}.{}", callee.name, local.name);
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
                let name = format!("{}.{}", callee.name, local.name);
                subst[i] = Some(Subst::Local(self.temp(name, local.ty, local.kind)));
            }
        }
        let subst: Vec<Subst> = subst.into_iter().map(|s| s.expect("every local mapped")).collect();
        let mut body = callee.body.clone();
        let value = match body.last() {
            Some(Stmt::Return(Some(_))) => match body.pop() {
                Some(Stmt::Return(Some(v))) => Some(v),
                _ => unreachable!("just checked"),
            },
            _ => None,
        };
        let mut value = value;
        rename_stmts(&mut body, &subst);
        if let Some(v) = &mut value {
            rename_expr(v, &subst);
        }
        // Inline calls the callee itself makes.
        self.stmts(&mut body, depth + 1);
        if let Some(v) = &mut value {
            self.expr(v, depth + 1);
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
