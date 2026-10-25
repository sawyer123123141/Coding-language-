//! List methods: `remove_at`, `pop`, `insert`, `clear`, `has` and `find` (`add` is `Stmt::ListAdd`).
//! The runtime moves items for `remove_at` and `insert` and empties the list for `clear`
//! (`Stmt::List`, after drops); `pop` is built from `SetLen` and a move, copy or drop of the last
//! item; `has` and `find` call a generated search (see `list_helpers`); `filter`, `map`, `any` and
//! `all` call a generated loop (see `list_fns`); `sort` and `sort_by` are `ListOp::Sort` or, for a
//! `sort_by` with a number or bool key, `ListOp::SortBy` (see `sort`).

use kestrel_check::{ListMethod, Ty};
use kestrel_syntax::ast;

use super::helpers::{has_heap, owning, with};
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
                let item = with(&list, Proj::Index { at: at.clone(), check: IndexCheck::Checked });
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
            // The last item moves out (zeroing its slot) and is dropped at once, not when the
            // block ends: an owner the list is inside of may be replaced before then.
            ListMethod::Pop => {
                let list = self.place_or_temp(receiver, out, true);
                let n = self.temp(KTy::I64);
                out.push(Stmt::Let(n, Expr { kind: ExprKind::Len(list.clone()), ty: KTy::I64, line }));
                let last = binary(BinOp::Sub, load(n, KTy::I64, line), int(1, KTy::I64, line), KTy::I64, line);
                let mut then_body = Vec::new();
                if elem.is_aggregate() {
                    let item = with(&list, Proj::Index { at: last.clone(), check: IndexCheck::Checked });
                    let tmp = self.temp(elem);
                    then_body.push(Stmt::Let(tmp, Expr { kind: ExprKind::Move(item), ty: elem, line }));
                    then_body.extend(self.helpers.drop_stmts(Place::local(tmp), elem, self.t));
                }
                then_body.push(Stmt::SetLen(list, last));
                let not_empty = binary(BinOp::Ne, load(n, KTy::I64, line), int(0, KTy::I64, line), KTy::Bool, line);
                out.push(Stmt::If { cond: not_empty, then_body, else_body: Vec::new() });
            }
            ListMethod::Sort | ListMethod::SortBy => self.sort_stmt(method, receiver, args, out),
            ListMethod::Has | ListMethod::Find | ListMethod::Join | ListMethod::Filter | ListMethod::Map | ListMethod::Any | ListMethod::All => {
                unreachable!("the checker refuses a discarded `{method:?}`")
            }
        }
    }

    /// A list method call that has a value: `pop`, `has` or `find`.
    pub(crate) fn list_value(&mut self, method: ListMethod, receiver: &ast::Expr, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        match method {
            ListMethod::Pop => self.pop(receiver, ty, line),
            ListMethod::Find => self.find(receiver, &args[0].value, ty, line),
            ListMethod::Join => self.str_join(receiver, &args[0].value),
            ListMethod::Filter | ListMethod::Map | ListMethod::Any | ListMethod::All => self.list_fn_call(method, receiver, &args[0].value, ty, line),
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

    /// `xs.pop()`: the last item goes into an optional, or `none` for an empty list, and the
    /// length goes down after. An owning item moves out (the checker makes sure no owner the
    /// list is inside of holds its memory). Any other item is a read: it is copied into the
    /// current arena like an indexed read, since the list's memory may belong to an owner that
    /// is replaced or cleared while the item lives. Either way the slot is left zeroed.
    fn pop(&mut self, receiver: &ast::Expr, ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let list = self.place_or_temp(receiver, &mut pre, true);
        let elem = self.elem_ty(kty(self.t.ty(receiver.id)));
        let (result, n) = (self.temp(ty), self.temp(KTy::I64));
        pre.push(Stmt::Let(result, Expr { kind: ExprKind::NoneLit, ty, line }));
        let len = Expr { kind: ExprKind::Len(list.clone()), ty: KTy::I64, line };
        pre.push(Stmt::Let(n, len));
        let last = binary(BinOp::Sub, load(n, KTy::I64, line), int(1, KTy::I64, line), KTy::I64, line);
        let item = with(&list, Proj::Index { at: last.clone(), check: IndexCheck::Checked });
        let mut then_body = Vec::new();
        let is_owning = owning(elem, self.t);
        let kind = if !elem.is_aggregate() {
            ExprKind::Load(item)
        } else if is_owning || !has_heap(elem, self.t) {
            ExprKind::Move(item)
        } else {
            let copy = self.temp(elem);
            then_body.push(Stmt::Let(copy, Expr { kind: ExprKind::Move(item), ty: elem, line }));
            then_body.extend(self.helpers.fixup(Place::local(copy), elem, self.t));
            ExprKind::Move(Place::local(copy))
        };
        let some = Expr { kind: ExprKind::Some(Box::new(Expr { kind, ty: elem, line })), ty, line };
        then_body.extend([Stmt::Assign(Place::local(result), some), Stmt::SetLen(list, last)]);
        let not_empty = binary(BinOp::Ne, load(n, KTy::I64, line), int(0, KTy::I64, line), KTy::Bool, line);
        pre.push(Stmt::If { cond: not_empty, then_body, else_body: Vec::new() });
        if is_owning {
            // A new owning value that nothing holds yet (see `builds_new`).
            self.scratch_results.insert(result);
        }
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
