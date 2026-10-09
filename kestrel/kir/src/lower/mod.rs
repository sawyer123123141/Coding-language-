mod expr;

use std::collections::HashMap;

use kestrel_check::{FnId, LocalId, Res, Tables, Ty};
use kestrel_syntax::ast::{self, Program};
use kestrel_syntax::{LineIndex, Span};

use crate::ir::*;

/// Lowers a checked program. `t` must come from a `check` with no errors.
pub fn lower(program: &Program, t: &Tables, lines: &LineIndex) -> Module {
    let funcs = (0..t.fns.len())
        .map(|i| FnLowerer { program, t, lines, fn_id: FnId(i as u32), local_map: HashMap::new() }.lower())
        .collect();
    Module { funcs, main: FuncIdx(t.main.expect("checked program has main").0) }
}

pub(crate) fn kty(ty: Ty) -> KTy {
    match ty {
        Ty::Bool => KTy::Bool,
        Ty::U8 => KTy::U8,
        Ty::Int => KTy::I32,
        Ty::I64 => KTy::I64,
        Ty::Float => KTy::F32,
        Ty::F64 => KTy::F64,
        other => panic!("type {other:?} has no runtime representation"),
    }
}

pub(crate) struct FnLowerer<'a> {
    program: &'a Program,
    t: &'a Tables,
    lines: &'a LineIndex,
    fn_id: FnId,
    local_map: HashMap<LocalId, LocalIdx>,
}

impl<'a> FnLowerer<'a> {
    fn lower(mut self) -> Func {
        // Copy the shared references out so borrows of them don't hold `self`.
        let (t, program) = (self.t, self.program);
        let info = t.func(self.fn_id);
        let ast::Item::Fn(decl) = &program.items[info.item] else { unreachable!("FnInfo points at a fn") };
        let mut locals = Vec::new();
        for (i, &lid) in info.locals.iter().enumerate() {
            let l = t.local(lid);
            let kind = if l.is_var_param {
                LocalKind::RefParam
            } else if l.addressed {
                LocalKind::Addressed
            } else {
                LocalKind::Value
            };
            self.local_map.insert(lid, LocalIdx(i as u32));
            locals.push(Local { name: l.name.clone(), ty: kty(l.ty), kind });
        }
        let params = (0..info.params.len()).map(|i| LocalIdx(i as u32)).collect();
        let ret = if info.ret == Ty::Unit { None } else { Some(kty(info.ret)) };
        let body = if ret.is_some() {
            let (mut stmts, tail) = self.block_value(&decl.body);
            if let Some(v) = tail {
                stmts.push(Stmt::Return(Some(v)));
            }
            stmts
        } else {
            self.block_stmts(&decl.body)
        };
        Func {
            name: info.name.clone(),
            params,
            ret,
            locals,
            body,
            is_pure: info.is_pure,
            io: info.io,
            mutates_params: info.mutates_params,
        }
    }

    pub(crate) fn line(&self, span: Span) -> u32 {
        self.lines.line(span.start)
    }

    fn bind_local(&self, bind: &ast::Binding) -> LocalIdx {
        self.local_map[&self.t.bind_local[&bind.id]]
    }

    pub(crate) fn block_stmts(&mut self, block: &ast::Block) -> Vec<Stmt> {
        block.stmts.iter().map(|s| self.stmt(s)).collect()
    }

    /// A block used as a value: its statements and its tail value, which is
    /// `None` when the block can't finish normally.
    pub(crate) fn block_value(&mut self, block: &ast::Block) -> (Vec<Stmt>, Option<Expr>) {
        let Some((last, init)) = block.stmts.split_last() else { return (Vec::new(), None) };
        let mut stmts: Vec<Stmt> = init.iter().map(|s| self.stmt(s)).collect();
        match last {
            ast::Stmt::Expr(e) if self.t.ty(e.id) != Ty::Never => {
                let v = self.expr(e);
                (stmts, Some(v))
            }
            other => {
                stmts.push(self.stmt(other));
                (stmts, None)
            }
        }
    }

    fn stmt(&mut self, s: &ast::Stmt) -> Stmt {
        match s {
            ast::Stmt::Let { bind, value, .. } => {
                let idx = self.bind_local(bind);
                Stmt::Let(idx, self.expr(value))
            }
            ast::Stmt::Assign { target, op, value, .. } => {
                let Some(Res::Local(lid)) = self.t.res.get(&target.id).copied() else {
                    unreachable!("checker resolved the assignment target")
                };
                let idx = self.local_map[&lid];
                let rhs = self.expr(value);
                let value = match op.binary() {
                    None => rhs,
                    Some(bop) => {
                        let (ty, line) = (rhs.ty, rhs.line);
                        let cur = Expr { kind: ExprKind::Local(idx), ty, line };
                        Expr { kind: ExprKind::Binary(binop(bop), Box::new(cur), Box::new(rhs)), ty, line }
                    }
                };
                Stmt::Assign(idx, value)
            }
            ast::Stmt::While { cond, body, .. } => Stmt::While { cond: self.expr(cond), body: self.block_stmts(body) },
            ast::Stmt::ForRange { var, start, end, body, .. } => Stmt::ForRange {
                var: self.bind_local(var),
                start: self.expr(start),
                end: self.expr(end),
                body: self.block_stmts(body),
            },
            ast::Stmt::Break(_) => Stmt::Break,
            ast::Stmt::Continue(_) => Stmt::Continue,
            ast::Stmt::Return { value, .. } => Stmt::Return(value.as_ref().map(|v| self.expr(v))),
            ast::Stmt::Expr(e) => self.expr_stmt(e),
        }
    }

    /// An expression whose value is discarded.
    fn expr_stmt(&mut self, e: &ast::Expr) -> Stmt {
        match &e.kind {
            ast::ExprKind::If { cond, then_block, else_block } => {
                let cond = self.expr(cond);
                let then_body = self.block_stmts(then_block);
                let else_body = match else_block {
                    Some(b) => self.block_stmts(b),
                    None => Vec::new(),
                };
                Stmt::If { cond, then_body, else_body }
            }
            ast::ExprKind::Call { args, .. } => match self.t.res[&e.id] {
                Res::Print => Stmt::Print(args.iter().map(|a| self.print_arg(&a.value)).collect()),
                Res::Fn(f) => {
                    let (fi, args) = self.call_parts(e, f, args);
                    Stmt::Call(fi, args)
                }
                other => unreachable!("checker rejects a discarded {other:?}"),
            },
            _ => unreachable!("checker rejects statements with no effect"),
        }
    }

    fn print_arg(&mut self, e: &ast::Expr) -> PrintArg {
        match &e.kind {
            ast::ExprKind::Str(s) => PrintArg::Str(s.clone()),
            _ => PrintArg::Value(self.expr(e)),
        }
    }
}

pub(crate) fn binop(op: ast::BinaryOp) -> BinOp {
    match op {
        ast::BinaryOp::Add => BinOp::Add,
        ast::BinaryOp::Sub => BinOp::Sub,
        ast::BinaryOp::Mul => BinOp::Mul,
        ast::BinaryOp::Div => BinOp::Div,
        ast::BinaryOp::Rem => BinOp::Rem,
        ast::BinaryOp::Eq => BinOp::Eq,
        ast::BinaryOp::Ne => BinOp::Ne,
        ast::BinaryOp::Lt => BinOp::Lt,
        ast::BinaryOp::Gt => BinOp::Gt,
        ast::BinaryOp::Le => BinOp::Le,
        ast::BinaryOp::Ge => BinOp::Ge,
        ast::BinaryOp::And | ast::BinaryOp::Or => unreachable!("lowered to And/Or nodes"),
    }
}
