//! Built-ins on SIMD vectors (`vec4`, `ivec4`): each is one vector operation or a short sequence
//! of them, never a call and never a loop over lanes. `min max floor ceil round sqrt` and a float
//! vector's `abs` are one `Math` of the vector type; `clamp` is `min(max(x, lo), hi)`; an int
//! vector's `abs` is `max(v, -v)` with `v` evaluated once, so its only check is the `Neg`'s; `lerp`
//! is `a + (b - a) * t`; `sum` adds the lanes left to right (checked on an `ivec4`, as `+`);
//! `dot` is the `sum` of a product; `length` is `sqrt` of `dot(v, v)`; `normalized` divides by the
//! length when it is positive (else zeros); `distance_to` is the `length` of the difference.

use kestrel_check::Builtin;
use kestrel_syntax::ast;

use super::builtins::{binary, node, number, select};
use super::vector::{may_write, rereadable};
use super::FnLowerer;
use crate::ir::*;
use crate::leaves::zero;

impl<'a> FnLowerer<'a> {
    /// `min(v, 1)`, `clamp(v, lo, hi)`, `floor(v)` ... of SIMD vectors of type `ty` (a number fills
    /// every lane). Each argument is evaluated once, left to right.
    pub(crate) fn simd_lane_call(&mut self, b: Builtin, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let vals: Vec<Expr> = args.iter().map(|a| self.expr(&a.value)).collect();
        let mut vals: Vec<Expr> = vals
            .into_iter()
            .map(|v| {
                let v = self.once(v, &mut pre);
                if v.ty.is_vector() {
                    v
                } else {
                    node(ExprKind::Splat(Box::new(v)), ty, line)
                }
            })
            .collect();
        let math = |op, xs: Vec<Expr>| node(ExprKind::Math(op, xs), ty, line);
        let kind = match b {
            Builtin::Floor => ExprKind::Math(MathOp::Floor, vals),
            Builtin::Ceil => ExprKind::Math(MathOp::Ceil, vals),
            Builtin::Round => ExprKind::Math(MathOp::Round, vals),
            Builtin::Sqrt => ExprKind::Math(MathOp::Sqrt, vals),
            Builtin::Min => ExprKind::Math(MathOp::Min, vals),
            Builtin::Max => ExprKind::Math(MathOp::Max, vals),
            Builtin::Abs if ty == KTy::F32X4 => ExprKind::Math(MathOp::Abs, vals),
            Builtin::Abs => {
                // `max(v, -v)`: the `Neg` is checked, so the lowest `int` in a lane stops the program.
                let v = vals.remove(0);
                let neg = node(ExprKind::Unary(UnOp::Neg, Box::new(v.clone())), ty, line);
                ExprKind::Math(MathOp::Max, vec![v, neg])
            }
            Builtin::Clamp => {
                let hi = vals.pop().expect("clamp has three values");
                let lo = vals.pop().expect("clamp has three values");
                let x = vals.pop().expect("clamp has three values");
                ExprKind::Math(MathOp::Min, vec![math(MathOp::Max, vec![x, lo]), hi])
            }
            other => unreachable!("{other:?} isn't a lane-by-lane built-in"),
        };
        Self::with_pre(pre, kind, ty, line)
    }

    /// A vector method (`sum`, `length`, `normalized`, `dot`, `distance_to`) of a SIMD vector: the
    /// receiver, then the argument, are evaluated once, in that order. A receiver that is a local
    /// (a place that can be read again) is read where it is used, unless the argument may write
    /// something: then it is copied first, so the argument can't change what it reads.
    pub(crate) fn simd_method(&mut self, b: Builtin, v: Expr, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let vty = v.ty;
        let lane = vty.lane();
        let arg = args.first().map(|a| self.expr(&a.value));
        let plain = matches!(&v.kind, ExprKind::Load(p) if rereadable(p)) && !arg.as_ref().is_some_and(may_write);
        let v = if plain { v } else { self.once(v, &mut pre) };
        let mut arg = arg.into_iter();
        let kind = match b {
            Builtin::Sum => Self::lane_sum(&v, lane, line).kind,
            Builtin::Length => self.simd_length(&v, &v, &mut pre, line),
            Builtin::Dot => {
                let w = arg.next().expect("`dot` takes a vector");
                let w = self.once(w, &mut pre);
                self.simd_dot(&v, &w, &mut pre, line).kind
            }
            Builtin::DistanceTo => {
                let w = arg.next().expect("`distance_to` takes a vector");
                let diff = self.spill(node(ExprKind::Binary(BinOp::Sub, Box::new(v), Box::new(w)), vty, line), &mut pre);
                self.simd_length(&diff, &diff, &mut pre, line)
            }
            Builtin::Normalized => {
                let len = node(self.simd_length(&v, &v, &mut pre, line), KTy::F32, line);
                let len = self.spill(len, &mut pre);
                // A zero vector stays zero.
                let positive = binary(BinOp::Gt, &len, &number(0, KTy::F32, line), KTy::Bool, line);
                let unit = node(ExprKind::Splat(Box::new(len)), vty, line);
                let unit = binary(BinOp::Div, &v, &unit, vty, line);
                select(positive, unit, zero(vty, line))
            }
            other => unreachable!("{other:?} isn't a method of a SIMD vector"),
        };
        Self::with_pre(pre, kind, ty, line)
    }

    /// The lanes of `v` (which can be read more than once) added left to right, `((x + y) + z) + w`,
    /// each `+` of the lane type.
    fn lane_sum(v: &Expr, lane: KTy, line: u32) -> Expr {
        let of = |i: u8| node(ExprKind::LaneOf(Box::new(v.clone()), i), lane, line);
        (1..4).fold(of(0), |sum, i| binary(BinOp::Add, &sum, &of(i), lane, line))
    }

    /// `a.dot(b)` for two float vectors that can be read more than once: the lanes of the product
    /// (made once) added left to right.
    fn simd_dot(&mut self, a: &Expr, b: &Expr, pre: &mut Vec<Stmt>, line: u32) -> Expr {
        let product = self.spill(binary(BinOp::Mul, a, b, a.ty, line), pre);
        Self::lane_sum(&product, KTy::F32, line)
    }

    /// `sqrt(a.dot(b))`.
    fn simd_length(&mut self, a: &Expr, b: &Expr, pre: &mut Vec<Stmt>, line: u32) -> ExprKind {
        ExprKind::Math(MathOp::Sqrt, vec![self.simd_dot(a, b, pre, line)])
    }
}
