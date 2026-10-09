mod call;
mod collections;
mod expr;
mod helpers;
mod layout;
mod matching;
mod vector;

use std::collections::HashMap;

use kestrel_check::{FnId, LocalId, Res, Tables, Ty};
use kestrel_syntax::ast::{self, Program};
use kestrel_syntax::{LineIndex, Span};

use crate::ir::*;
use helpers::Helpers;

/// Lowers a checked program. `t` must come from a `check` with no errors.
pub fn lower(program: &Program, t: &Tables, lines: &LineIndex) -> Module {
    let (structs, enums) = layout::layouts(t);
    let mut helpers = Helpers::new(t.fns.len() as u32);
    let mut funcs: Vec<Func> =
        (0..t.fns.len()).map(|i| FnLowerer::new(program, t, lines, FnId(i as u32), &mut helpers).lower()).collect();
    funcs.append(&mut helpers.funcs);
    let elems = t.elems.iter().map(|&e| kty(e)).collect();
    let main = FuncIdx(t.main.expect("checked program has main").0);
    Module { funcs, structs, enums, elems, print_fns: helpers.print, main }
}

pub(crate) fn kty(ty: Ty) -> KTy {
    match ty {
        Ty::Bool => KTy::Bool,
        Ty::U8 => KTy::U8,
        Ty::Int => KTy::I32,
        Ty::I64 => KTy::I64,
        Ty::Float => KTy::F32,
        Ty::F64 => KTy::F64,
        Ty::Str => KTy::Str,
        Ty::Struct(s) => KTy::Struct(StructIdx(s.0)),
        Ty::Enum(e) => KTy::Enum(EnumIdx(e.0)),
        Ty::Array(e) => KTy::Array(ElemIdx(e.0)),
        Ty::List(e) => KTy::List(ElemIdx(e.0)),
        other => panic!("type {other:?} has no runtime representation"),
    }
}

pub(crate) struct FnLowerer<'a> {
    program: &'a Program,
    t: &'a Tables,
    lines: &'a LineIndex,
    fn_id: FnId,
    local_map: HashMap<LocalId, LocalIdx>,
    /// Grows during lowering as temporaries are added.
    locals: Vec<Local>,
    /// `match` bindings: the matched place and the variant whose fields they name.
    bindings: HashMap<LocalId, (Place, u32)>,
    /// `for ... in` bindings: the element place they stand for.
    elems: HashMap<LocalId, Place>,
    helpers: &'a mut Helpers,
}

impl<'a> FnLowerer<'a> {
    fn new(program: &'a Program, t: &'a Tables, lines: &'a LineIndex, fn_id: FnId, helpers: &'a mut Helpers) -> Self {
        let (local_map, locals, bindings, elems) = (HashMap::new(), Vec::new(), HashMap::new(), HashMap::new());
        FnLowerer { program, t, lines, fn_id, local_map, locals, bindings, elems, helpers }
    }

    fn lower(mut self) -> Func {
        // Copy the shared references out so borrows of them don't hold `self`.
        let (t, program) = (self.t, self.program);
        let info = t.func(self.fn_id);
        let decl = info.decl(program);
        for &lid in &info.locals {
            let l = t.local(lid);
            // `match` bindings aren't storage; they name fields of the matched place.
            if let Ty::Variant(..) = l.ty {
                continue;
            }
            let ty = kty(l.ty);
            let kind = if l.is_param && (l.is_var_param || ty.is_aggregate()) {
                LocalKind::RefParam
            } else if l.addressed || ty.is_aggregate() {
                LocalKind::Addressed
            } else {
                LocalKind::Value
            };
            self.local_map.insert(lid, LocalIdx(self.locals.len() as u32));
            self.locals.push(Local { name: l.name.clone(), ty, kind });
        }
        let params = (0..info.params.len()).map(|i| LocalIdx(i as u32)).collect();
        let ret = if info.ret == Ty::Unit { None } else { Some(kty(info.ret)) };
        let body = if ret.is_some() {
            let (mut stmts, tail) = self.block_value(&decl.body);
            if let Some(v) = tail {
                let v = self.owned(v, true);
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
            locals: self.locals,
            body,
            is_pure: info.is_pure,
            io: info.io,
            mutates_params: info.mutates_params,
        }
    }

    /// A fresh local holding an intermediate value.
    pub(crate) fn temp(&mut self, ty: KTy) -> LocalIdx {
        let idx = LocalIdx(self.locals.len() as u32);
        let kind = if ty.is_aggregate() { LocalKind::Addressed } else { LocalKind::Value };
        self.locals.push(Local { name: format!("tmp{}", idx.0), ty, kind });
        idx
    }

    pub(crate) fn line(&self, span: Span) -> u32 {
        self.lines.line(span.start)
    }

    fn bind_local(&self, bind: &ast::Binding) -> LocalIdx {
        self.local_map[&self.t.bind_local[&bind.id]]
    }

    pub(crate) fn block_stmts(&mut self, block: &ast::Block) -> Vec<Stmt> {
        let mut out = Vec::new();
        for s in &block.stmts {
            self.stmt(s, &mut out);
        }
        out
    }

    /// A block used as a value: its statements and its tail value, which is
    /// `None` when the block can't finish normally.
    pub(crate) fn block_value(&mut self, block: &ast::Block) -> (Vec<Stmt>, Option<Expr>) {
        let Some((last, init)) = block.stmts.split_last() else { return (Vec::new(), None) };
        let mut stmts = Vec::new();
        for s in init {
            self.stmt(s, &mut stmts);
        }
        match last {
            ast::Stmt::Expr(e) if !self.leaves_early(e) => {
                let v = self.expr(e);
                (stmts, Some(v))
            }
            other => {
                self.stmt(other, &mut stmts);
                (stmts, None)
            }
        }
    }

    /// Whether `e` as a block's last expression can never yield a value: it
    /// is typed `Never`, or an `if` whose branches all end that way.
    fn leaves_early(&self, e: &ast::Expr) -> bool {
        if self.t.ty(e.id) == Ty::Never {
            return true;
        }
        match &e.kind {
            ast::ExprKind::If { then_block, else_block: Some(else_block), .. } => {
                [then_block, else_block].into_iter().all(|b| match b.stmts.last() {
                    Some(ast::Stmt::Expr(tail)) => self.leaves_early(tail),
                    _ => true,
                })
            }
            _ => false,
        }
    }

    fn stmt(&mut self, s: &ast::Stmt, out: &mut Vec<Stmt>) {
        let lowered = match s {
            ast::Stmt::Let { bind, value, .. } => {
                let idx = self.bind_local(bind);
                Stmt::Let(idx, self.value(value))
            }
            ast::Stmt::Assign { target, op, value, .. } => {
                let place = self.place(target).expect("checker requires an assignable place");
                let ty = kty(self.t.ty(target.id));
                let rhs = self.expr(value);
                let value = match op.binary() {
                    None => self.owned(rhs, false),
                    Some(bop) => {
                        let line = rhs.line;
                        let cur = Expr { kind: ExprKind::Load(place.clone()), ty, line };
                        let kind = if ty.is_aggregate() {
                            self.vec_combine(binop(bop), cur, rhs, ty, line)
                        } else {
                            ExprKind::Binary(binop(bop), Box::new(cur), Box::new(rhs))
                        };
                        Expr { kind, ty, line }
                    }
                };
                Stmt::Assign(place, value)
            }
            ast::Stmt::While { cond, body, .. } => Stmt::While { cond: self.expr(cond), body: self.block_stmts(body) },
            ast::Stmt::ForRange { var, start, end, body, .. } => Stmt::ForRange {
                var: self.bind_local(var),
                start: self.expr(start),
                end: self.expr(end),
                body: self.block_stmts(body),
            },
            ast::Stmt::ForIn { index, var, iter, body, .. } => return self.for_in(index.as_ref(), var, iter, body, out),
            ast::Stmt::Break(_) => Stmt::Break,
            ast::Stmt::Continue(_) => Stmt::Continue,
            ast::Stmt::Return { value, .. } => Stmt::Return(value.as_ref().map(|v| {
                let v = self.expr(v);
                self.owned(v, true)
            })),
            ast::Stmt::Expr(e) => return self.expr_stmt(e, out),
        };
        out.push(lowered);
    }

    /// An expression whose value is discarded.
    fn expr_stmt(&mut self, e: &ast::Expr, out: &mut Vec<Stmt>) {
        let (receiver, args) = match &e.kind {
            ast::ExprKind::If { cond, then_block, else_block } => {
                let cond = self.expr(cond);
                let then_body = self.block_stmts(then_block);
                let else_body = match else_block {
                    Some(b) => self.block_stmts(b),
                    None => Vec::new(),
                };
                out.push(Stmt::If { cond, then_body, else_body });
                return;
            }
            ast::ExprKind::Match { .. } => return self.match_stmt(e, out),
            ast::ExprKind::Call { args, .. } => (None, args),
            ast::ExprKind::MethodCall { receiver, args, .. } => (Some(&**receiver), args),
            _ => unreachable!("checker rejects statements with no effect"),
        };
        match self.t.res[&e.id] {
            Res::Print => out.push(Stmt::Print(args.iter().map(|a| self.print_arg(&a.value)).collect())),
            Res::ListAdd => {
                let v = self.value(&args[0].value);
                let list = self.place(receiver.expect("`add` is a method")).expect("checker requires a place for `add`");
                out.push(Stmt::ListAdd(list, v));
            }
            Res::Fn(f) => {
                let (pre, fi, args) = self.call_parts(e, f, args, receiver);
                out.extend(pre);
                out.push(Stmt::Call(fi, args));
            }
            other => unreachable!("checker rejects a discarded {other:?}"),
        }
    }

    fn print_arg(&mut self, e: &ast::Expr) -> PrintArg {
        match &e.kind {
            ast::ExprKind::Str(s) => PrintArg::Str(s.clone()),
            ast::ExprKind::Interp(parts) => PrintArg::Interp(
                parts
                    .iter()
                    .map(|p| match p {
                        ast::InterpPart::Lit(s) => PrintArg::Str(s.clone()),
                        ast::InterpPart::Expr(x) => self.print_arg(x),
                    })
                    .collect(),
            ),
            _ => {
                let v = self.expr(e);
                self.helpers.ensure_print(v.ty, self.t);
                PrintArg::Value(v)
            }
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
