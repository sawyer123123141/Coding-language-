//! The descent that finds calls to inline inside statements, expressions and places.

use super::Inliner;
use crate::ir::*;

impl Inliner<'_> {
    pub(super) fn stmt(&mut self, s: &mut Stmt, depth: u32) {
        match s {
            Stmt::Let(_, e) | Stmt::Return(Some(e)) | Stmt::RandSeed(e) => self.expr(e, depth),
            Stmt::Assign(p, e) | Stmt::ListAdd(p, e) | Stmt::SetLen(p, e) => {
                self.place(p, depth);
                self.expr(e, depth);
            }
            Stmt::List(_, p, xs) => {
                self.place(p, depth);
                for x in xs {
                    self.expr(x, depth);
                }
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
            Stmt::Bind(_, p) | Stmt::Dup(p) | Stmt::FreeArena(p) | Stmt::DropGpu(p) | Stmt::PushArena(Some(p)) => self.place(p, depth),
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
            Stmt::BindGlobal(..) | Stmt::PushArena(None) | Stmt::PopArena { .. } | Stmt::SetLine(_) | Stmt::Break | Stmt::Continue | Stmt::Return(None) => {}
        }
    }

    pub(super) fn place(&mut self, p: &mut Place, depth: u32) {
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

    pub(super) fn expr(&mut self, e: &mut Expr, depth: u32) {
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
            ExprKind::Binary(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) | ExprKind::RandInt(a, b) => {
                self.expr(a, depth);
                self.expr(b, depth);
            }
            ExprKind::Call(_, args) => self.args(args, depth),
            ExprKind::StructLit(_, xs) | ExprKind::VariantLit(_, _, xs) | ExprKind::ArrayLit(xs) | ExprKind::Math(_, xs) | ExprKind::Str(_, xs) => {
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
            | ExprKind::StaticArena
            | ExprKind::NoneLit
            | ExprKind::RandFloat => {}
        }
    }
}
