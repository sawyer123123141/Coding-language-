mod call;
mod collections;
mod expr;
mod helpers;
mod layout;
mod matching;
mod scopes;
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
    let main = t.main.map(|m| FuncIdx(m.0));
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
        Ty::Arena => KTy::Arena,
        Ty::Optional(e) => KTy::Optional(ElemIdx(e.0)),
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
    /// Element pointers made by `for ... in`: the place each points at.
    origins: HashMap<LocalIdx, Place>,
    /// Enclosing blocks, innermost last, with what leaving each must do.
    scopes: Vec<scopes::Scope>,
    helpers: &'a mut Helpers,
}

impl<'a> FnLowerer<'a> {
    fn new(program: &'a Program, t: &'a Tables, lines: &'a LineIndex, fn_id: FnId, helpers: &'a mut Helpers) -> Self {
        let (local_map, locals, bindings, elems) = (HashMap::new(), Vec::new(), HashMap::new(), HashMap::new());
        let (origins, scopes) = (HashMap::new(), Vec::new());
        FnLowerer { program, t, lines, fn_id, local_map, locals, bindings, elems, origins, scopes, helpers }
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
            // Items of other modules are named `module.name`.
            exported: decl.is_pub && info.method.is_none() && !info.name.contains('.'),
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

    /// Whether `e` as a block's last expression can never yield a value: it
    /// is typed `Never`, or an `if` whose branches all end that way.
    pub(crate) fn leaves_early(&self, e: &ast::Expr) -> bool {
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

    pub(crate) fn stmt(&mut self, s: &ast::Stmt, out: &mut Vec<Stmt>) {
        let lowered = match s {
            ast::Stmt::Let { bind, value, else_block: Some(otherwise), .. } => {
                return self.let_else(bind, value, otherwise, out);
            }
            ast::Stmt::Let { bind, value, .. } => {
                let idx = self.bind_local(bind);
                let value = self.value(value);
                out.push(Stmt::Let(idx, value));
                return self.declared(idx);
            }
            ast::Stmt::Assign { target, value, .. } if helpers::owning(kty(self.t.ty(target.id)), self.t) => {
                return self.assign_owning(target, value, out);
            }
            ast::Stmt::Assign { target, op, value, .. } => {
                let mut place = self.place(target).expect("checker requires an assignable place");
                let ty = kty(self.t.ty(target.id));
                let rhs = self.expr(value);
                let value = match op.binary() {
                    None => self.owned(rhs, false),
                    Some(bop) => {
                        // The value first, then the place (read and written), once.
                        let indexed = place.path.iter().any(|p| matches!(p, Proj::Index { at, .. } if !matches!(at.kind, ExprKind::Int(_))));
                        let rhs = if indexed {
                            let rhs = self.spill(rhs, out);
                            self.stabilize(&mut place, out);
                            rhs
                        } else {
                            rhs
                        };
                        let line = rhs.line;
                        let rhs = shift_amount(binop(bop), rhs, ty);
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
            ast::Stmt::While { cond, body, .. } => Stmt::While { cond: self.expr(cond), body: self.loop_block(body) },
            ast::Stmt::ForRange { var, start, end, body, .. } => Stmt::ForRange {
                var: self.bind_local(var),
                start: self.expr(start),
                end: self.expr(end),
                body: self.loop_block(body),
            },
            ast::Stmt::ForIn { index, var, iter, body, .. } => return self.for_in(index.as_ref(), var, iter, body, out),
            ast::Stmt::Arena { body, .. } => return self.arena_block(None, body, out),
            ast::Stmt::In { arena, body, .. } => return self.arena_block(Some(arena), body, out),
            ast::Stmt::Break(_) => return self.jump(Stmt::Break, out),
            ast::Stmt::Continue(_) => return self.jump(Stmt::Continue, out),
            ast::Stmt::Return { value, .. } => return self.return_stmt(value.as_ref(), out),
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

/// A shift's amount, converted to the shifted value's type if it differs.
pub(crate) fn shift_amount(op: BinOp, amount: Expr, ty: KTy) -> Expr {
    if !matches!(op, BinOp::Shl | BinOp::Shr) || amount.ty == ty {
        return amount;
    }
    let line = amount.line;
    Expr { kind: ExprKind::Cast(Box::new(amount)), ty, line }
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
        ast::BinaryOp::BitAnd => BinOp::BitAnd,
        ast::BinaryOp::BitOr => BinOp::BitOr,
        ast::BinaryOp::BitXor => BinOp::BitXor,
        ast::BinaryOp::Shl => BinOp::Shl,
        ast::BinaryOp::Shr => BinOp::Shr,
        ast::BinaryOp::And | ast::BinaryOp::Or => unreachable!("lowered to And/Or nodes"),
    }
}
