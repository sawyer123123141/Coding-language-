use kestrel_check::{ConstId, FnId, Res};
use kestrel_syntax::ast;

use super::{binop, kty, FnLowerer};
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// Lowers `e` and applies any implicit conversion the checker recorded.
    pub(crate) fn expr(&mut self, e: &ast::Expr) -> Expr {
        let Some(&to) = self.t.coerce.get(&e.id) else { return self.expr_raw(e) };
        let to = kty(to);
        // A literal converted to a float is emitted at the target type, read from the
        // source, so an f64 never inherits the rounding of an f32 literal.
        if to.is_float() {
            if let Some(v) = literal_as(e, to) {
                return Expr { kind: ExprKind::Float(v), ty: to, line: self.line(e.span) };
            }
        }
        let raw = self.expr_raw(e);
        Expr { ty: to, line: raw.line, kind: ExprKind::Cast(Box::new(raw)) }
    }

    fn expr_raw(&mut self, e: &ast::Expr) -> Expr {
        let ty = kty(self.t.ty(e.id));
        let line = self.line(e.span);
        let kind = match &e.kind {
            ast::ExprKind::Int(n) => {
                if ty.is_float() {
                    ExprKind::Float(*n as f64)
                } else {
                    ExprKind::Int(*n as i64)
                }
            }
            ast::ExprKind::Float(f) => ExprKind::Float(*f),
            ast::ExprKind::Bool(b) => ExprKind::Bool(*b),
            ast::ExprKind::Str(_) => unreachable!("strings only appear as print arguments"),
            ast::ExprKind::Ident(_) => match self.t.res[&e.id] {
                Res::Local(l) => ExprKind::Local(self.local_map[&l]),
                Res::Const(c) => return self.const_value(c),
                other => unreachable!("name resolved to {other:?}"),
            },
            ast::ExprKind::Unary { op, expr } => {
                let op = match op {
                    ast::UnaryOp::Neg => UnOp::Neg,
                    ast::UnaryOp::Not => UnOp::Not,
                };
                ExprKind::Unary(op, Box::new(self.expr(expr)))
            }
            ast::ExprKind::Binary { op, lhs, rhs } => {
                let l = Box::new(self.expr(lhs));
                let r = Box::new(self.expr(rhs));
                match op {
                    ast::BinaryOp::And => ExprKind::And(l, r),
                    ast::BinaryOp::Or => ExprKind::Or(l, r),
                    other => ExprKind::Binary(binop(*other), l, r),
                }
            }
            ast::ExprKind::Call { args, .. } => match self.t.res[&e.id] {
                Res::Convert(_) => ExprKind::Cast(Box::new(self.expr(&args[0].value))),
                Res::Fn(f) => {
                    let (fi, a) = self.call_parts(e, f, args);
                    ExprKind::Call(fi, a)
                }
                other => unreachable!("{other:?} used as a value"),
            },
            ast::ExprKind::Field { .. } | ast::ExprKind::MethodCall { .. } => {
                unreachable!("checker rejects struct expressions")
            }
            ast::ExprKind::If { cond, then_block, else_block } => {
                let else_block = else_block.as_ref().expect("checker requires `else` for a value `if`");
                let cond = Box::new(self.expr(cond));
                let (then_body, then_val) = self.block_value(then_block);
                let (else_body, else_val) = self.block_value(else_block);
                ExprKind::If {
                    cond,
                    then_body,
                    then_val: then_val.map(Box::new),
                    else_body,
                    else_val: else_val.map(Box::new),
                }
            }
        };
        Expr { kind, ty, line }
    }

    /// A constant's initializer, re-lowered at each use.
    fn const_value(&mut self, c: ConstId) -> Expr {
        let (t, program) = (self.t, self.program);
        let item = t.consts[c.0 as usize].item;
        let ast::Item::Const(decl) = &program.items[item] else { unreachable!("ConstInfo points at a const") };
        self.expr(&decl.value)
    }

    /// Arguments in parameter order; `var` parameters get a reference.
    pub(crate) fn call_parts(&mut self, e: &ast::Expr, f: FnId, args: &[ast::Arg]) -> (FuncIdx, Vec<Arg>) {
        let t = self.t;
        let order = &t.call_args[&e.id];
        let params = &t.func(f).params;
        let lowered = order
            .iter()
            .zip(params)
            .map(|(&ai, p)| {
                let v = &args[ai].value;
                if p.is_var {
                    let Some(Res::Local(l)) = t.res.get(&v.id).copied() else {
                        unreachable!("checker requires a variable for a `var` argument")
                    };
                    Arg::Ref(self.local_map[&l])
                } else {
                    Arg::Value(self.expr(v))
                }
            })
            .collect();
        (FuncIdx(f.0), lowered)
    }
}

/// The value of a numeric literal (possibly negated) as float type `to`,
/// or `None` if `e` is anything else.
fn literal_as(e: &ast::Expr, to: KTy) -> Option<f64> {
    match &e.kind {
        ast::ExprKind::Int(n) if to == KTy::F32 => Some(*n as f32 as f64),
        ast::ExprKind::Int(n) => Some(*n as f64),
        ast::ExprKind::Float(f) => Some(*f),
        ast::ExprKind::Unary { op: ast::UnaryOp::Neg, expr } => literal_as(expr, to).map(|v| -v),
        _ => None,
    }
}
