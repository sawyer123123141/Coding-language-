//! Built-ins on vectors: `min max clamp abs floor ceil round sqrt` lane by lane (each lane through
//! the scalar function of its type, with its checks), `v.sum()`, and `==` / `!=`.

use kestrel_check::Builtin;
use kestrel_syntax::ast;

use super::builtins::{binary, node};
use super::vector::Operand;
use super::FnLowerer;
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// `min(v, 1)`, `clamp(v, lo, hi)`, `floor(v)` ... of vectors of type `ty` (a number fills
    /// every lane). Each argument is evaluated once, left to right; then each lane is the scalar
    /// function of that lane of every argument.
    pub(crate) fn vec_lane_call(&mut self, b: Builtin, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        let KTy::Struct(s) = ty else { unreachable!("vectors are structs") };
        let (dims, lane) = (self.dims(s), self.lane(s));
        let mut pre = Vec::new();
        let vals: Vec<Expr> = args.iter().map(|a| self.expr(&a.value)).collect();
        let mut operands = Vec::with_capacity(vals.len());
        for v in vals {
            let v = self.once(v, &mut pre);
            operands.push(self.operand(v, &mut pre));
        }
        let mut fields = Vec::with_capacity(dims as usize);
        for i in 0..dims {
            let lanes: Vec<Expr> = operands.iter().map(|o| Self::component(o, i, lane, line)).collect();
            let kind = match b {
                Builtin::Floor => ExprKind::Math(MathOp::Floor, lanes),
                Builtin::Ceil => ExprKind::Math(MathOp::Ceil, lanes),
                Builtin::Round => ExprKind::Math(MathOp::Round, lanes),
                Builtin::Sqrt => ExprKind::Math(MathOp::Sqrt, lanes),
                _ if lane.is_float() => self.float_call(b, lanes, lane, line, &mut pre),
                _ => self.int_call(b, lanes, lane, line, &mut pre),
            };
            fields.push(node(kind, lane, line));
        }
        Self::with_pre(pre, ExprKind::StructLit(s, fields), ty, line)
    }

    /// The components of `o` added left to right: `((x + y) + z)`, each `+` of the lane type.
    pub(super) fn sum(o: &Operand, dims: u32, lane: KTy, line: u32) -> Expr {
        (0..dims)
            .map(|i| Self::component(o, i, lane, line))
            .reduce(|sum, c| binary(BinOp::Add, &sum, &c, lane, line))
            .expect("vectors have components")
    }

    /// `l == r` (every component equal) or, with `ne`, `l != r` (its opposite) where one side is
    /// a vector; a number is every component. Each side is evaluated once, left to right.
    pub(crate) fn vec_equal(&mut self, ne: bool, l: Expr, r: Expr, line: u32) -> ExprKind {
        let vty = if l.ty.is_aggregate() { l.ty } else { r.ty };
        let KTy::Struct(s) = vty else { unreachable!("vectors are structs") };
        let (dims, lane) = (self.dims(s), self.lane(s));
        let mut pre = Vec::new();
        let l = self.once(l, &mut pre);
        let r = self.once(r, &mut pre);
        let (lo, ro) = (self.operand(l, &mut pre), self.operand(r, &mut pre));
        let all = (0..dims)
            .map(|i| binary(BinOp::Eq, &Self::component(&lo, i, lane, line), &Self::component(&ro, i, lane, line), KTy::Bool, line))
            .reduce(|all, eq| node(ExprKind::And(Box::new(all), Box::new(eq)), KTy::Bool, line))
            .expect("vectors have components");
        let kind = if ne { ExprKind::Unary(UnOp::Not, Box::new(all)) } else { all.kind };
        Self::with_pre(pre, kind, KTy::Bool, line)
    }
}
