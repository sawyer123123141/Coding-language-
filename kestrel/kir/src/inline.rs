//! Inlines calls to small functions, the way `block_at(y, h).weight()`
//! wants: the callee's body is copied into the caller with its locals
//! renamed and its by-pointer parameters replaced by the argument places.

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
    let (init, last) = match f.body.split_last() {
        Some((Stmt::Return(Some(_)), init)) => (init, true),
        _ => (&f.body[..], false),
    };
    if !last && f.ret.is_some() {
        return None;
    }
    let mut nodes = 0;
    let mut ok = true;
    walk(&f.body, &mut |s| {
        nodes += 1;
        match s {
            Node::Stmt(Stmt::Return(_)) => {}
            Node::Expr(Expr { kind: ExprKind::Call(callee, _), .. }) | Node::Stmt(Stmt::Call(callee, _)) => {
                ok &= *callee != idx;
            }
            _ => {}
        }
    });
    let mut early_return = false;
    walk(init, &mut |s| early_return |= matches!(s, Node::Stmt(Stmt::Return(_))));
    (ok && !early_return && nodes <= MAX_NODES).then_some(nodes)
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

    fn stmt(&mut self, s: &mut Stmt, depth: u32) {
        match s {
            Stmt::Let(_, e) | Stmt::Return(Some(e)) => self.expr(e, depth),
            Stmt::Assign(p, e) | Stmt::ListAdd(p, e) => {
                self.place(p, depth);
                self.expr(e, depth);
            }
            Stmt::If { cond, then_body, else_body } => {
                self.expr(cond, depth);
                self.stmts(then_body, depth);
                self.stmts(else_body, depth);
            }
            Stmt::While { cond, body } => {
                self.expr(cond, depth);
                self.stmts(body, depth);
            }
            Stmt::ForRange { start, end, body, .. } => {
                self.expr(start, depth);
                self.expr(end, depth);
                self.stmts(body, depth);
            }
            Stmt::Call(_, args) => self.args(args, depth),
            Stmt::Print(parts) | Stmt::Write(parts) => self.print(parts, depth),
            Stmt::Bind(_, p) | Stmt::Dup(p) | Stmt::FreeArena(p) | Stmt::PushArena(Some(p)) => self.place(p, depth),
            Stmt::MapSet(p, k, v, _) => {
                self.place(p, depth);
                self.expr(k, depth);
                self.expr(v, depth);
            }
            Stmt::MapUnlock(p, saved) => {
                self.place(p, depth);
                self.expr(saved, depth);
            }
            Stmt::ParFor { args, count, .. } => {
                self.args(args, depth);
                self.expr(count, depth);
            }
            Stmt::PushArena(None) | Stmt::PopArena { .. } | Stmt::Break | Stmt::Continue | Stmt::Return(None) => {}
        }
    }

    fn place(&mut self, p: &mut Place, depth: u32) {
        for at in p.path.iter_mut().filter_map(Proj::at_mut) {
            self.expr(at, depth);
        }
    }

    fn args(&mut self, args: &mut [Arg], depth: u32) {
        for a in args {
            match a {
                Arg::Value(e) => self.expr(e, depth),
                Arg::Ref(p) => self.place(p, depth),
            }
        }
    }

    fn print(&mut self, parts: &mut [PrintArg], depth: u32) {
        for p in parts {
            match p {
                PrintArg::Value(e) => self.expr(e, depth),
                PrintArg::Interp(inner) => self.print(inner, depth),
                PrintArg::Str(_) => {}
            }
        }
    }

    fn expr(&mut self, e: &mut Expr, depth: u32) {
        match &mut e.kind {
            ExprKind::Call(fi, args) if self.take(*fi, depth) => {
                let (fi, args) = (*fi, std::mem::take(args));
                let (body, value) = self.expand(fi, args, depth);
                let value = value.expect("a value call's callee ends in `return value`");
                e.kind = if body.is_empty() { value.kind } else { ExprKind::Seq(body, Box::new(value)) };
            }
            ExprKind::Format(parts) => self.print(parts, depth),
            ExprKind::Load(p) | ExprKind::Tag(p) | ExprKind::Len(p) | ExprKind::Move(p) => self.place(p, depth),
            ExprKind::Map(_, p, xs) => {
                self.place(p, depth);
                for x in xs {
                    self.expr(x, depth);
                }
            }
            ExprKind::Unary(_, x) | ExprKind::Cast(x) | ExprKind::FieldOf(x, _) | ExprKind::ArrayNew(x) | ExprKind::Some(x) => self.expr(x, depth),
            ExprKind::Binary(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) => {
                self.expr(a, depth);
                self.expr(b, depth);
            }
            ExprKind::Call(_, args) => self.args(args, depth),
            ExprKind::StructLit(_, xs) | ExprKind::VariantLit(_, _, xs) | ExprKind::ArrayLit(xs) => {
                for x in xs {
                    self.expr(x, depth);
                }
            }
            ExprKind::Seq(stmts, v) => {
                self.stmts(stmts, depth);
                self.expr(v, depth);
            }
            ExprKind::If { cond, then_body, then_val, else_body, else_val } => {
                self.expr(cond, depth);
                self.stmts(then_body, depth);
                self.stmts(else_body, depth);
                for v in [then_val, else_val].into_iter().flatten() {
                    self.expr(v, depth);
                }
            }
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::StrLit(_)
            | ExprKind::ListNew
            | ExprKind::MapNew
            | ExprKind::NewArena
            | ExprKind::NoneLit => {}
        }
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
