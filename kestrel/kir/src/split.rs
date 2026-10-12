//! The rewrite `scalarize` and `promote` share. Every place rooted at a chosen local (a **split
//! root**) becomes the local holding that leaf, and every store of a plain struct value is done
//! one leaf at a time, with all of the value's leaves computed before any is stored.

use std::collections::HashMap;

use crate::ir::*;
use crate::layout::StructLayout;
use crate::leaves::{field_path, is_plain_struct, leaf_place, leaves, load, rebuild, under, zero, Leaf};
use crate::visit_mut::{walk_expr, walk_place, walk_stmt, VisitMut};

/// A split root's leaves and the local that holds each.
pub(crate) struct Split {
    pub leaves: Vec<Leaf>,
    pub locals: Vec<LocalIdx>,
}

pub(crate) struct Rewriter<'a> {
    pub structs: &'a [StructLayout],
    pub locals: &'a mut Vec<Local>,
    pub roots: HashMap<LocalIdx, Split>,
}

impl Rewriter<'_> {
    pub(crate) fn temp(&mut self, name: &str, ty: KTy, kind: LocalKind) -> LocalIdx {
        self.locals.push(Local { name: name.to_string(), ty, kind });
        LocalIdx(self.locals.len() as u32 - 1)
    }

    /// The local holding the leaf at `p`, when `p` is rooted at a split root.
    fn leaf_local(&self, p: &Place) -> Option<LocalIdx> {
        let split = self.roots.get(&p.root)?;
        let path = field_path(&p.path).expect("a split root's places are field paths");
        let k = split.leaves.iter().position(|l| l.path == path).expect("a scalar place is a leaf");
        Some(split.locals[k])
    }

    /// The value at `p` (rooted at a split root): its leaf local, or a `StructLit` of them.
    fn read(&self, p: &Place, ty: KTy, line: u32) -> Expr {
        let split = &self.roots[&p.root];
        let path = field_path(&p.path).expect("a split root's places are field paths");
        let range = under(&split.leaves, &path);
        let mut vals = range.map(|k| load(split.locals[k], split.leaves[k].ty, line));
        rebuild(self.structs, ty, &mut vals, line)
    }

    /// Whether `e` is a plain struct value that is copied as a whole between two places in
    /// memory, the one struct store that isn't split.
    fn memory_copy(&self, e: &Expr) -> bool {
        matches!(&e.kind, ExprKind::Load(q) if !self.roots.contains_key(&q.root))
    }

    /// `e` in a fresh local, unless it is a constant: the result can be read later, after other
    /// code has run, and still give `e`'s value.
    fn hold(&mut self, mut e: Expr, out: &mut Vec<Stmt>) -> Expr {
        self.visit_expr(&mut e);
        if matches!(e.kind, ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_)) {
            return e;
        }
        let (ty, line) = (e.ty, e.line);
        let t = self.temp("leaf", ty, LocalKind::Value);
        out.push(Stmt::Let(t, e));
        load(t, ty, line)
    }

    /// `p` with every index expression that isn't a constant or a local evaluated now, into a
    /// local, so the place can be used several times and still mean one element.
    fn stable(&mut self, mut p: Place, out: &mut Vec<Stmt>) -> Place {
        for at in p.path.iter_mut().filter_map(Proj::at_mut) {
            let simple = match &at.kind {
                ExprKind::Int(_) => true,
                ExprKind::Load(q) => q.path.is_empty() && self.locals[q.root.0 as usize].kind == LocalKind::Value,
                _ => false,
            };
            if !simple {
                let (ty, line) = (at.ty, at.line);
                let t = self.temp("index", ty, LocalKind::Value);
                let v = std::mem::replace(at, load(t, ty, line));
                out.push(Stmt::Let(t, v));
            }
        }
        p
    }

    /// The leaves of `e`, a plain struct value, in leaf order. The code computing them goes to
    /// `out` in the order codegen evaluates `e`; each leaf is a constant or a load of a local that
    /// holds it.
    pub(crate) fn split(&mut self, e: Expr, out: &mut Vec<Stmt>) -> Vec<Expr> {
        let Expr { kind, ty, line } = e;
        match kind {
            ExprKind::StructLit(_, fields) => {
                let mut vals = Vec::new();
                for f in fields {
                    if f.ty.is_aggregate() {
                        vals.extend(self.split(f, out));
                    } else {
                        vals.push(self.hold(f, out));
                    }
                }
                vals
            }
            ExprKind::Load(mut p) => {
                self.visit_place(&mut p);
                let all = leaves(self.structs, ty).expect("a plain struct");
                if self.roots.contains_key(&p.root) {
                    let split = &self.roots[&p.root];
                    let range = under(&split.leaves, &field_path(&p.path).expect("a field path"));
                    let reads: Vec<Expr> = range.map(|k| load(split.locals[k], split.leaves[k].ty, line)).collect();
                    return reads.into_iter().map(|r| self.hold(r, out)).collect();
                }
                let p = self.stable(p, out);
                all.iter().map(|l| self.hold(Expr { kind: ExprKind::Load(leaf_place(&p, l)), ty: l.ty, line }, out)).collect()
            }
            ExprKind::FieldOf(base, i) if is_plain_struct(self.structs, base.ty) => {
                let base_ty = base.ty;
                let vals = self.split(*base, out);
                let all = leaves(self.structs, base_ty).expect("a plain struct");
                vals[under(&all, &[i])].to_vec()
            }
            ExprKind::Seq(stmts, value) => {
                let mut stmts = stmts;
                self.visit_stmts(&mut stmts);
                out.extend(stmts);
                self.split(*value, out)
            }
            ExprKind::If { mut cond, then_body, then_val, else_body, else_val } => {
                self.visit_expr(&mut cond);
                let all = leaves(self.structs, ty).expect("a plain struct");
                let temps: Vec<LocalIdx> = all
                    .iter()
                    .map(|l| {
                        let t = self.temp("leaf", l.ty, LocalKind::Value);
                        out.push(Stmt::Let(t, zero(l.ty, line)));
                        t
                    })
                    .collect();
                let then_body = self.branch(then_body, then_val, &temps);
                let else_body = self.branch(else_body, else_val, &temps);
                out.push(Stmt::If { cond: *cond, then_body, else_body });
                temps.iter().zip(&all).map(|(&t, l)| load(t, l.ty, line)).collect()
            }
            kind => {
                // A call's result, or any other value built in memory: keep it in a temporary.
                let mut e = Expr { kind, ty, line };
                self.visit_expr(&mut e);
                let t = self.temp("value", ty, LocalKind::Addressed);
                out.push(Stmt::Let(t, e));
                let at = Place::local(t);
                let all = leaves(self.structs, ty).expect("a plain struct");
                all.iter().map(|l| Expr { kind: ExprKind::Load(leaf_place(&at, l)), ty: l.ty, line }).collect()
            }
        }
    }

    /// One branch of a struct-valued `if`: its statements, then its value's leaves stored in `temps`.
    /// A branch without a value never finishes, so it stores nothing.
    fn branch(&mut self, body: Vec<Stmt>, val: Option<Box<Expr>>, temps: &[LocalIdx]) -> Vec<Stmt> {
        let mut body = body;
        self.visit_stmts(&mut body);
        if let Some(v) = val {
            let vals = self.split(*v, &mut body);
            for (&t, v) in temps.iter().zip(vals) {
                body.push(Stmt::Assign(Place::local(t), v));
            }
        }
        body
    }

    /// `dest = e`, rewritten: a leaf of a split root is stored in its local, and a plain struct
    /// value is stored one leaf at a time after all of its leaves are computed. `dest`'s index
    /// expressions run after `e`, as codegen orders a store.
    fn store(&mut self, dest: Place, e: Expr, out: &mut Vec<Stmt>) {
        let mut dest = dest;
        if !e.ty.is_aggregate() {
            let mut e = e;
            self.visit_expr(&mut e);
            self.visit_place(&mut dest);
            let dest = match self.leaf_local(&dest) {
                Some(l) => Place::local(l),
                None => dest,
            };
            out.push(Stmt::Assign(dest, e));
            return;
        }
        let ty = e.ty;
        let vals = self.split(e, out);
        self.visit_place(&mut dest);
        let dest = self.stable(dest, out);
        for (leaf, v) in leaves(self.structs, ty).expect("a plain struct").iter().zip(vals) {
            let p = leaf_place(&dest, leaf);
            let p = match self.leaf_local(&p) {
                Some(l) => Place::local(l),
                None => p,
            };
            out.push(Stmt::Assign(p, v));
        }
    }

    fn stmt(&mut self, s: Stmt, out: &mut Vec<Stmt>) {
        match s {
            Stmt::Let(l, e) if self.roots.contains_key(&l) => self.store(Place::local(l), e, out),
            Stmt::Assign(p, e) if self.roots.contains_key(&p.root) => self.store(p, e, out),
            Stmt::Assign(p, e) if is_plain_struct(self.structs, e.ty) && !self.memory_copy(&e) => self.store(p, e, out),
            mut s => {
                walk_stmt(self, &mut s);
                out.push(s);
            }
        }
    }
}

impl VisitMut for Rewriter<'_> {
    fn visit_stmts(&mut self, list: &mut Vec<Stmt>) {
        let mut out = Vec::with_capacity(list.len());
        for s in std::mem::take(list) {
            self.stmt(s, &mut out);
        }
        *list = out;
    }

    fn visit_expr(&mut self, e: &mut Expr) {
        match &mut e.kind {
            ExprKind::Load(p) if self.roots.contains_key(&p.root) => {
                walk_place(self, p);
                *e = self.read(p, e.ty, e.line);
            }
            ExprKind::FieldOf(base, i) if is_plain_struct(self.structs, base.ty) => {
                let (i, line, ty) = (*i, e.line, e.ty);
                let base = std::mem::replace(&mut **base, zero(KTy::Bool, line));
                let base_ty = base.ty;
                let mut pre = Vec::new();
                let vals = self.split(base, &mut pre);
                let all = leaves(self.structs, base_ty).expect("a plain struct");
                let value = rebuild(self.structs, ty, &mut vals[under(&all, &[i])].iter().cloned(), line);
                *e = if pre.is_empty() { value } else { Expr { kind: ExprKind::Seq(pre, Box::new(value)), ty, line } };
            }
            _ => walk_expr(self, e),
        }
    }
}
