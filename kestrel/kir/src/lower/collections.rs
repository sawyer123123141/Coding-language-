//! Arrays and lists: indexing, `.len`, `for ... in`, and the copies value
//! semantics needs when a value is stored somewhere new.

use kestrel_syntax::ast;

use super::helpers::{has_heap, owning};
use crate::rename::{rename_stmts, Subst};
use super::FnLowerer;
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// `e` as a place: itself when it names one, else a temporary holding
    /// its value. With `stable`, index expressions in the place are
    /// evaluated once, here, so the place can be used more than once.
    pub(crate) fn place_or_temp(&mut self, e: &ast::Expr, pre: &mut Vec<Stmt>, stable: bool) -> Place {
        let Some(mut p) = self.place(e) else {
            let v = self.expr(e);
            let tmp = self.temp(v.ty);
            pre.push(Stmt::Let(tmp, v));
            return Place::local(tmp);
        };
        if stable {
            self.stabilize(&mut p, pre);
        }
        p
    }

    /// Evaluates `p`'s index expressions now, into temporaries, so using
    /// `p` again doesn't run them again.
    pub(crate) fn stabilize(&mut self, p: &mut Place, pre: &mut Vec<Stmt>) {
        for proj in &mut p.path {
            if let Some(at) = proj.at_mut() {
                if !matches!(at.kind, ExprKind::Int(_)) {
                    *at = self.spill(at.clone(), pre);
                }
            }
        }
    }

    /// `base[index]` read as a value.
    pub(crate) fn index_value(&mut self, e: &ast::Expr, base: &ast::Expr, index: &ast::Expr, ty: KTy, line: u32) -> ExprKind {
        if let Some(p) = self.place(e) {
            return ExprKind::Load(p);
        }
        let mut pre = Vec::new();
        let mut p = self.place_or_temp(base, &mut pre, false);
        p.path.push(Proj::Index { at: self.expr(index), checked: true });
        Self::with_pre(pre, ExprKind::Load(p), ty, line)
    }

    /// `xs.len`: stored as an i64, read as an `int` (checked when it doesn't fit).
    pub(crate) fn len_value(&mut self, base: &ast::Expr, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let p = self.place_or_temp(base, &mut pre, false);
        let len = Expr { kind: ExprKind::Len(p), ty: KTy::I64, line };
        Self::with_pre(pre, ExprKind::Cast(Box::new(len)), KTy::I32, line)
    }

    /// `for i, v in xs`: a counted loop where `v` stands for `xs[i]`. The
    /// length is read once, before the first iteration. When the body can't
    /// move the elements (it never changes `xs` itself, only elements), `v`
    /// is a pointer found once per iteration; otherwise it re-indexes on
    /// every use.
    pub(crate) fn for_in(
        &mut self,
        index: Option<&ast::Binding>,
        var: &ast::Binding,
        iter: &ast::Expr,
        body: &ast::Block,
        out: &mut Vec<Stmt>,
    ) {
        if let kestrel_check::Ty::Map(..) = self.t.ty(iter.id) {
            return self.map_for_in(index, var, iter, body, out);
        }
        let line = self.line(iter.span);
        let list = self.place_or_temp(iter, out, true);
        let i = match index {
            Some(b) => self.bind_local(b),
            None => self.temp(KTy::I32),
        };
        let at = Expr { kind: ExprKind::Load(Place::local(i)), ty: KTy::I32, line };
        let mut elem = list.clone();
        elem.path.push(Proj::Index { at: at.clone(), checked: true });
        let var = self.t.bind_local[&var.id];
        let ptr = LocalIdx(self.locals.len() as u32);
        let ty = super::kty(self.t.local(var).ty);
        self.locals.push(Local { name: format!("{}_ref", self.t.local(var).name), ty, kind: LocalKind::RefParam });
        self.origins.insert(ptr, elem.clone());
        self.elems.insert(var, Place::local(ptr));
        // `i < xs.len` holds throughout, since the body can't change `xs` itself.
        let mut known = list.clone();
        known.path.push(Proj::Index { at, checked: false });
        let mut stmts = vec![Stmt::Bind(ptr, known)];
        stmts.extend(self.loop_block(body));
        if self.may_move(&stmts, &list) {
            // Re-index on every use instead: point every use of the pointer at `xs[i]`.
            stmts.remove(0);
            let mut subst: Vec<Subst> = (0..self.locals.len() as u32).map(|l| Subst::Local(LocalIdx(l))).collect();
            subst[ptr.0 as usize] = Subst::Place(elem);
            rename_stmts(&mut stmts, &subst);
        }
        let len = Expr { kind: ExprKind::Len(list), ty: KTy::I64, line };
        let end = Expr { kind: ExprKind::Cast(Box::new(len)), ty: KTy::I32, line };
        let start = Expr { kind: ExprKind::Int(0), ty: KTy::I32, line };
        out.push(Stmt::ForRange { var: i, start, end, body: stmts });
    }

    /// Whether `stmts` may change the collection at `list` itself (not just
    /// its elements), moving its elements: a write to `list` or to a place
    /// containing it. Element pointers are compared by what they point at,
    /// and any index may equal any other.
    pub(crate) fn may_move(&self, stmts: &[Stmt], list: &Place) -> bool {
        let list = self.origin(list);
        let mut moved = false;
        crate::visit::places(stmts, &mut |p, written| {
            if written && !moved {
                let p = self.origin(p);
                moved = p.root == list.root
                    && p.path.len() <= list.path.len()
                    && p.path.iter().zip(&list.path).all(|(a, b)| match (a, b) {
                        (Proj::Index { .. }, Proj::Index { .. }) => true,
                        _ => a == b,
                    });
            }
        });
        moved
    }

    /// `p` with any element pointer at its root replaced by the place it points at.
    fn origin(&self, p: &Place) -> Place {
        match self.origins.get(&p.root) {
            Some(o) => {
                let mut o = self.origin(o);
                o.path.extend(p.path.iter().cloned());
                o
            }
            None => p.clone(),
        }
    }

    /// `let x = opt else { ... }`: the optional goes into a temporary; when
    /// it's `none` the `else` block runs (and leaves), otherwise its value
    /// moves out into `x` (the temporary is fresh, so nothing is shared).
    pub(crate) fn let_else(&mut self, bind: &ast::Binding, value: &ast::Expr, otherwise: &ast::Block, out: &mut Vec<Stmt>) {
        let line = self.line(value.span);
        let v = self.value(value);
        let opt = self.temp(v.ty);
        out.push(Stmt::Let(opt, v));
        let tag = Expr { kind: ExprKind::Tag(Place::local(opt)), ty: KTy::U8, line };
        let zero = Expr { kind: ExprKind::Int(0), ty: KTy::U8, line };
        let cond = Expr { kind: ExprKind::Binary(BinOp::Eq, Box::new(tag), Box::new(zero)), ty: KTy::Bool, line };
        let then_body = self.block_stmts(otherwise);
        out.push(Stmt::If { cond, then_body, else_body: Vec::new() });
        let idx = self.bind_local(bind);
        let ty = self.locals[idx.0 as usize].ty;
        let payload = Place { root: opt, path: vec![Proj::Payload] };
        out.push(Stmt::Let(idx, Expr { kind: ExprKind::Load(payload), ty, line }));
        self.declared(idx);
    }

    /// `e` lowered as a value about to be stored somewhere new.
    pub(crate) fn value(&mut self, e: &ast::Expr) -> Expr {
        let v = self.expr(e);
        self.owned(v, false)
    }

    /// Makes `v` safe to store. Every variable owns its data, so reading a
    /// place whose value holds arrays or lists copies them. With `moving`
    /// (a returned value), reads of locals aren't copied: they're about to
    /// die. Parameters belong to the caller, so they're copied regardless.
    pub(crate) fn owned(&mut self, v: Expr, moving: bool) -> Expr {
        if !has_heap(v.ty, self.t) {
            return v;
        }
        let Expr { kind, ty, line } = v;
        let kind = match kind {
            // A local that owns an arena may hold memory from it, and is dropped on the way out.
            ExprKind::Load(p)
                if moving
                    && self.locals[p.root.0 as usize].kind != LocalKind::RefParam
                    && !owning(self.locals[p.root.0 as usize].ty, self.t) =>
            {
                ExprKind::Load(p)
            }
            // A place, or a field of an `if` / `match` that yields places.
            kind @ (ExprKind::Load(_) | ExprKind::FieldOf(..)) if !Self::fresh(&kind) => {
                let tmp = self.temp(ty);
                let mut stmts = vec![Stmt::Let(tmp, Expr { kind, ty, line })];
                stmts.extend(self.helpers.fixup(Place::local(tmp), ty, self.t));
                ExprKind::Seq(stmts, Box::new(Expr { kind: ExprKind::Load(Place::local(tmp)), ty, line }))
            }
            ExprKind::Seq(stmts, inner) => ExprKind::Seq(stmts, Box::new(self.owned(*inner, moving))),
            ExprKind::If { cond, then_body, then_val, else_body, else_val } => ExprKind::If {
                cond,
                then_body,
                then_val: then_val.map(|v| Box::new(self.owned(*v, moving))),
                else_body,
                else_val: else_val.map(|v| Box::new(self.owned(*v, moving))),
            },
            // Everything else builds a fresh value.
            other => other,
        };
        Expr { kind, ty, line }
    }

    /// Whether `kind` builds a new value rather than reading an existing one.
    fn fresh(kind: &ExprKind) -> bool {
        match kind {
            ExprKind::Load(_) => false,
            ExprKind::FieldOf(base, _) => matches!(base.kind, ExprKind::Call(..) | ExprKind::StructLit(..) | ExprKind::VariantLit(..)),
            _ => true,
        }
    }
}
