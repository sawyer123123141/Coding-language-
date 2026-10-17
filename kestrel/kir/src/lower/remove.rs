//! Loops over a list that remove their item (`remove b`). The list is compacted in the same
//! pass: `kept` counts the items kept so far, each kept item moves down to `xs[kept]` when its
//! pass ends, and `next` is the first item not reached yet. Leaving the loop, by its end, `break`
//! or `return`, moves the items from `next` on down to `kept` and sets the length.
//!
//! The item re-indexes `xs[i]` on every use (it has no `Bind`, so `promote` never keeps it in
//! locals across a pass's end), and every slot an item leaves is zeroed: code that reaches the
//! list some other way meanwhile (a `var` parameter) reads zeros or moved items, never freed
//! memory.

use kestrel_check::LocalId;
use kestrel_syntax::{ast, Span};

use super::helpers::owning;
use super::FnLowerer;
use crate::ir::*;

/// A loop that removes its item, being lowered.
pub(crate) struct RemoveLoop {
    item: LocalId,
    /// The item's slot, `xs[i]`, bounds-checked.
    slot: Place,
    /// Set by `remove`, cleared at the start of each pass.
    removed: LocalIdx,
}

fn load(l: LocalIdx, ty: KTy, line: u32) -> Expr {
    Expr { kind: ExprKind::Load(Place::local(l)), ty, line }
}

fn int(n: i64, ty: KTy, line: u32) -> Expr {
    Expr { kind: ExprKind::Int(n), ty, line }
}

/// The zero of a scalar type.
fn zero(ty: KTy, line: u32) -> Expr {
    let kind = match ty {
        KTy::Bool => ExprKind::Bool(false),
        KTy::F32 | KTy::F64 => ExprKind::Float(0.0),
        _ => ExprKind::Int(0),
    };
    Expr { kind, ty, line }
}

impl<'a> FnLowerer<'a> {
    /// `for i, v in xs` whose body removes `v` (see `Tables::removing_loops`).
    pub(crate) fn removing_for_in(
        &mut self,
        index: Option<&ast::Binding>,
        var: &ast::Binding,
        iter: &ast::Expr,
        body: &ast::Block,
        out: &mut Vec<Stmt>,
    ) {
        let line = self.line(iter.span);
        let list = self.place_or_temp(iter, out, true);
        let (kept, next, removed) = (self.temp(KTy::I64), self.temp(KTy::I64), self.temp(KTy::Bool));
        out.push(Stmt::Let(kept, int(0, KTy::I64, line)));
        out.push(Stmt::Let(next, int(0, KTy::I64, line)));
        out.push(Stmt::Let(removed, Expr { kind: ExprKind::Bool(false), ty: KTy::Bool, line }));
        let i = match index {
            Some(b) => self.bind_local(b),
            None => self.temp(KTy::I32),
        };
        let mut slot = list.clone();
        slot.path.push(Proj::Index { at: load(i, KTy::I32, line), checked: true });
        let item = self.t.bind_local[&var.id];
        self.elems.insert(item, slot.clone());
        let ty = super::kty(self.t.local(item).ty);
        let compact = Stmt::List(ListOp::Compact, list.clone(), vec![load(kept, KTy::I64, line), load(next, KTy::I64, line)]);
        let keep = keep_step(&list, &slot, ty, [i, kept, next, removed], line);
        self.removing.push(RemoveLoop { item, slot, removed });
        // A `return` inside compacts the list on its way out; the loop's end and `break` arrive below.
        let stmts = self.guarded(vec![compact.clone()], |s| {
            let mut stmts = vec![Stmt::Assign(Place::local(removed), Expr { kind: ExprKind::Bool(false), ty: KTy::Bool, line })];
            stmts.extend(s.loop_block_finishing(body, keep));
            stmts
        });
        self.removing.pop();
        let len = Expr { kind: ExprKind::Len(list), ty: KTy::I64, line };
        let end = Expr { kind: ExprKind::Cast(Box::new(len)), ty: KTy::I32, line };
        out.push(Stmt::ForRange { var: i, start: int(0, KTy::I32, line), end, body: stmts });
        out.push(compact);
    }

    /// `remove v`: the item moves out of its slot, which zeroes it, and is dropped.
    pub(crate) fn remove_stmt(&mut self, span: Span, out: &mut Vec<Stmt>) {
        let item = self.t.removes[&span];
        let lp = self.removing.iter().rev().find(|l| l.item == item).expect("`remove` is inside its item's loop");
        let (slot, removed) = (lp.slot.clone(), lp.removed);
        let ty = super::kty(self.t.local(item).ty);
        let line = self.line(span);
        if ty.is_aggregate() {
            let tmp = self.temp(ty);
            out.push(Stmt::Let(tmp, Expr { kind: ExprKind::Move(slot), ty, line }));
            if owning(ty, self.t) {
                out.extend(self.helpers.drop_stmts(Place::local(tmp), ty, self.t));
            }
        } else {
            out.push(Stmt::Assign(slot, zero(ty, line)));
        }
        out.push(Stmt::Assign(Place::local(removed), Expr { kind: ExprKind::Bool(true), ty: KTy::Bool, line }));
    }
}

/// The end of a pass: an item that wasn't removed moves down to `xs[kept]` (unless it is there
/// already), zeroing its old slot, and is counted; then `next` moves past it.
fn keep_step(list: &Place, slot: &Place, ty: KTy, [i, kept, next, removed]: [LocalIdx; 4], line: u32) -> Vec<Stmt> {
    let at = Expr { kind: ExprKind::Cast(Box::new(load(i, KTy::I32, line))), ty: KTy::I64, line };
    let mut dest = list.clone();
    dest.path.push(Proj::Index { at: load(kept, KTy::I64, line), checked: true });
    let moved = if ty.is_aggregate() {
        vec![Stmt::Assign(dest, Expr { kind: ExprKind::Move(slot.clone()), ty, line })]
    } else {
        vec![Stmt::Assign(dest, Expr { kind: ExprKind::Load(slot.clone()), ty, line }), Stmt::Assign(slot.clone(), zero(ty, line))]
    };
    let elsewhere = Expr { kind: ExprKind::Binary(BinOp::Ne, Box::new(load(kept, KTy::I64, line)), Box::new(at.clone())), ty: KTy::Bool, line };
    let count = Expr { kind: ExprKind::Binary(BinOp::Add, Box::new(load(kept, KTy::I64, line)), Box::new(int(1, KTy::I64, line))), ty: KTy::I64, line };
    let keep = vec![Stmt::If { cond: elsewhere, then_body: moved, else_body: Vec::new() }, Stmt::Assign(Place::local(kept), count)];
    let kept_it = Expr { kind: ExprKind::Unary(UnOp::Not, Box::new(load(removed, KTy::Bool, line))), ty: KTy::Bool, line };
    let past = Expr { kind: ExprKind::Binary(BinOp::Add, Box::new(at), Box::new(int(1, KTy::I64, line))), ty: KTy::I64, line };
    vec![Stmt::If { cond: kept_it, then_body: keep, else_body: Vec::new() }, Stmt::Assign(Place::local(next), past)]
}
