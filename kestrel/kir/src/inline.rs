//! Inlines calls to small functions, the way `block_at(y, h).weight()`
//! wants: the callee's body is copied into the caller with its locals
//! renamed and its by-pointer parameters replaced by the argument places.

mod descend;

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
    let sizes: Vec<Option<usize>> = originals.iter().enumerate().map(|(i, f)| inlinable(FuncIdx(i as u32), f)).collect();
    for f in &mut m.funcs {
        let locals = std::mem::take(&mut f.locals);
        let mut cx = Inliner { originals: &originals, sizes: &sizes, locals, budget: GROWTH_BUDGET };
        cx.stmts(&mut f.body, 0);
        f.locals = cx.locals;
    }
}

/// The size in KIR nodes of a function that can be inlined: small, not
/// directly recursive, and with no `return` except a final one (so the body
/// can be spliced in place of the call).
fn inlinable(idx: FuncIdx, f: &Func) -> Option<usize> {
    // A native's body is empty, but the call does something: it stays a call.
    if f.native.is_some() {
        return None;
    }
    let last = matches!(f.body.last(), Some(Stmt::Return(Some(_))));
    if !last && f.ret.is_some() {
        return None;
    }
    let mut nodes = 0;
    let mut ok = true;
    let mut returns = 0;
    walk(&f.body, &mut |s| {
        nodes += 1;
        match s {
            Node::Stmt(Stmt::Return(_)) => returns += 1,
            Node::Expr(Expr { kind: ExprKind::Call(callee, _), .. }) | Node::Stmt(Stmt::Call(callee, _)) => {
                ok &= *callee != idx;
            }
            _ => {}
        }
    });
    // Every `return` anywhere in the body counts, including one inside the
    // final return's own value (a tail `if` or `match` with a `return` in a
    // branch): only the final statement itself may be one.
    (ok && returns == usize::from(last) && nodes <= MAX_NODES).then_some(nodes)
}

struct Inliner<'a> {
    originals: &'a [Func],
    /// Each function's size when it can be inlined.
    sizes: &'a [Option<usize>],
    /// The caller's locals; inlined callees add theirs.
    locals: Vec<Local>,
    /// Nodes the caller may still grow by.
    budget: usize,
}

impl Inliner<'_> {
    /// True when a call to `fi` at `depth` should be inlined; takes its size
    /// from the budget.
    fn take(&mut self, fi: FuncIdx, depth: u32) -> bool {
        match self.sizes[fi.0 as usize] {
            Some(size) if depth < MAX_DEPTH && size <= self.budget => {
                self.budget -= size;
                true
            }
            _ => false,
        }
    }

    fn temp(&mut self, name: String, ty: KTy, kind: LocalKind) -> LocalIdx {
        self.locals.push(Local { name, ty, kind });
        LocalIdx(self.locals.len() as u32 - 1)
    }

    fn stmts(&mut self, stmts: &mut Vec<Stmt>, depth: u32) {
        let mut out = Vec::with_capacity(stmts.len());
        for mut s in std::mem::take(stmts) {
            if let Stmt::Call(fi, args) = &mut s {
                if self.take(*fi, depth) {
                    let (fi, args) = (*fi, std::mem::take(args));
                    let (mut body, value) = self.expand(fi, args, depth);
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
    /// Arguments are evaluated first, in order.
    fn expand(&mut self, fi: FuncIdx, args: Vec<Arg>, depth: u32) -> (Vec<Stmt>, Option<Expr>) {
        let callee = &self.originals[fi.0 as usize];
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
        pre.extend(body);
        (pre, value)
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

fn local_kind(ty: KTy) -> LocalKind {
    if ty.is_aggregate() {
        LocalKind::Addressed
    } else {
        LocalKind::Value
    }
}
