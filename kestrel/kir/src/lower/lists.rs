//! List methods: `remove_at`, `pop`, `insert`, `clear`, `has` and `find` (`add` is `Stmt::ListAdd`).
//! The runtime moves items for `remove_at` and `insert` (`Stmt::List`); `pop` and `clear` are
//! built from `SetLen` and drops; `has` and `find` call a generated search (see `list_helpers`).

use kestrel_check::{ListMethod, Ty};
use kestrel_syntax::ast;

use super::helpers::{owning, with};
use super::{kty, FnLowerer};
use crate::ir::*;

fn int(n: i64, ty: KTy, line: u32) -> Expr {
    Expr { kind: ExprKind::Int(n), ty, line }
}

fn load(l: LocalIdx, ty: KTy, line: u32) -> Expr {
    Expr { kind: ExprKind::Load(Place::local(l)), ty, line }
}

fn binary(op: BinOp, a: Expr, b: Expr, ty: KTy, line: u32) -> Expr {
    Expr { kind: ExprKind::Binary(op, Box::new(a), Box::new(b)), ty, line }
}

impl<'a> FnLowerer<'a> {
    /// A list method call whose result is thrown away.
    pub(crate) fn list_stmt(&mut self, method: ListMethod, e: &ast::Expr, receiver: &ast::Expr, args: &[ast::Arg], out: &mut Vec<Stmt>) {
        let line = self.line(e.span);
        let list_ty = kty(self.t.ty(receiver.id));
        let elem = self.elem_ty(list_ty);
        match method {
            ListMethod::RemoveAt => {
                let list = self.place_or_temp(receiver, out, true);
                let at = self.expr(&args[0].value);
                let at = if owning(elem, self.t) { self.spill(at, out) } else { at };
                // The item goes first, so a bad index panics before anything is dropped.
                let item = with(&list, Proj::Index { at: at.clone(), checked: true });
                out.extend(self.helpers.drop_stmts(item, elem, self.t));
                out.push(Stmt::List(ListOp::RemoveAt, list, vec![at]));
            }
            ListMethod::Insert => {
                let list = self.place_or_temp(receiver, out, true);
                // The index, then the value, are evaluated before the items move: the value may read the list.
                let at = self.expr(&args[0].value);
                let at = self.spill(at, out);
                let value = self.value(&args[1].value);
                let value = self.spill(value, out);
                out.push(Stmt::List(ListOp::Insert, list, vec![at, value]));
            }
            ListMethod::Clear => {
                let list = self.place_or_temp(receiver, out, true);
                out.extend(self.helpers.drop_stmts(list.clone(), list_ty, self.t));
                out.push(Stmt::List(ListOp::Clear, list, Vec::new()));
            }
            // The popped item is a value like any other: an owning one is held and dropped.
            ListMethod::Pop => {
                let ty = kty(self.t.ty(e.id));
                let kind = self.pop(receiver, ty, line);
                self.keep(Expr { kind, ty, line }, out);
            }
            ListMethod::Has | ListMethod::Find | ListMethod::Join => unreachable!("the checker refuses a discarded `{method:?}`"),
        }
    }

    /// A list method call that has a value: `pop`, `has` or `find`.
    pub(crate) fn list_value(&mut self, method: ListMethod, receiver: &ast::Expr, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        match method {
            ListMethod::Pop => self.pop(receiver, ty, line),
            ListMethod::Find => self.find(receiver, &args[0].value, ty, line),
            ListMethod::Join => self.str_join(receiver, &args[0].value),
            ListMethod::Has => {
                // `find`, then whether it found something.
                let opt = self.int_optional();
                let mut pre = Vec::new();
                let found = Expr { kind: self.find(receiver, &args[0].value, opt, line), ty: opt, line };
                let tmp = self.temp(opt);
                pre.push(Stmt::Let(tmp, found));
                let tag = Expr { kind: ExprKind::Tag(Place::local(tmp)), ty: KTy::U8, line };
                let some = binary(BinOp::Ne, tag, int(0, KTy::U8, line), KTy::Bool, line);
                Self::with_pre(pre, some.kind, ty, line)
            }
            other => unreachable!("`{other:?}` has no value"),
        }
    }

    /// The type of a list's or array's items.
    fn elem_ty(&self, list_ty: KTy) -> KTy {
        match list_ty {
            KTy::Array(el) | KTy::List(el) => kty(self.t.elems[el.0 as usize]),
            other => unreachable!("{other:?} has no items"),
        }
    }

    /// `int?`, the type of a search's result.
    fn int_optional(&self) -> KTy {
        let id = self.t.elem_ids[&Ty::Int];
        KTy::Optional(ElemIdx(id.0))
    }

    /// `xs.pop()`: the last item moves out into an optional, or `none` for an empty list. The
    /// slot is left zeroed by the move, and the length goes down after.
    fn pop(&mut self, receiver: &ast::Expr, ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let list = self.place_or_temp(receiver, &mut pre, true);
        let elem = self.elem_ty(kty(self.t.ty(receiver.id)));
        let (result, n) = (self.temp(ty), self.temp(KTy::I64));
        pre.push(Stmt::Let(result, Expr { kind: ExprKind::NoneLit, ty, line }));
        let len = Expr { kind: ExprKind::Len(list.clone()), ty: KTy::I64, line };
        pre.push(Stmt::Let(n, len));
        let last = binary(BinOp::Sub, load(n, KTy::I64, line), int(1, KTy::I64, line), KTy::I64, line);
        let item = with(&list, Proj::Index { at: last.clone(), checked: true });
        // Scalars are read, and anything with memory of its own moves.
        let kind = if elem.is_aggregate() { ExprKind::Move(item) } else { ExprKind::Load(item) };
        let some = Expr { kind: ExprKind::Some(Box::new(Expr { kind, ty: elem, line })), ty, line };
        let not_empty = binary(BinOp::Ne, load(n, KTy::I64, line), int(0, KTy::I64, line), KTy::Bool, line);
        let then_body = vec![Stmt::Assign(Place::local(result), some), Stmt::SetLen(list, last)];
        pre.push(Stmt::If { cond: not_empty, then_body, else_body: Vec::new() });
        // A new value that nothing holds yet (see `builds_new`).
        self.scratch_results.insert(result);
        Self::with_pre(pre, ExprKind::Move(Place::local(result)), ty, line)
    }

    /// `xs.find(v)`: a call of the search generated for the list's type.
    fn find(&mut self, receiver: &ast::Expr, needle: &ast::Expr, ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let list_ty = kty(self.t.ty(receiver.id));
        let list = self.place_or_temp(receiver, &mut pre, true);
        let needle = self.expr(needle);
        let f = self.helpers.find_fn(list_ty, ty, self.t);
        let args = vec![Arg::Value(Expr { kind: ExprKind::Load(list), ty: list_ty, line }), Arg::Value(needle)];
        Self::with_pre(pre, ExprKind::Call(f, args), ty, line)
    }
}
