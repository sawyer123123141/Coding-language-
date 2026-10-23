//! SIMD vectors (`vec4`): one `F32X4` value, never split into lanes. Makes are a `VecLit` of the
//! lanes or a `Splat` of one number; a lane of a place is the place's last step (`Proj::Lane`),
//! and a lane of any other value is a `LaneOf`; a swizzle to a `vec4` is a `Shuffle`. Operators
//! are one `Binary` or `Unary` of the vector type, with a number operand made a `Splat`, so
//! codegen reads the left side before the right side runs, as for numbers.

use kestrel_syntax::ast;

use super::builtins::node;
use super::FnLowerer;
use crate::ir::*;
use crate::leaves::zero;

/// Whether every step of a place's path can be followed again with the same result: none is an
/// index or key expression that would run again.
fn rereadable(p: &Place) -> bool {
    p.path.iter().all(|step| step.at().is_none())
}

impl<'a> FnLowerer<'a> {
    /// `vec4(...)` (`Res::SimdMake`). One number per lane (bound like a struct's fields, maybe
    /// labelled) or none: each value is evaluated in source order, then the lanes are built in
    /// lane order. A fill, a copy or a mixed make: each part is evaluated once, in source order,
    /// then its lanes are read (a number is one lane, or every lane when it is alone).
    pub(crate) fn simd_make(&mut self, e: &ast::Expr, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        if let Some(slots) = self.t.ctor_args.get(&e.id) {
            if args.is_empty() {
                return ExprKind::Splat(Box::new(zero(ty.lane(), line)));
            }
            let in_order = slots.iter().enumerate().all(|(i, &s)| s == Some(i));
            let vals: Vec<Expr> = args
                .iter()
                .map(|a| {
                    let v = self.expr(&a.value);
                    if in_order {
                        v
                    } else {
                        self.once(v, &mut pre)
                    }
                })
                .collect();
            let lanes = slots.iter().map(|s| vals[s.expect("every lane is given")].clone()).collect();
            return Self::with_pre(pre, ExprKind::VecLit(lanes), ty, line);
        }
        let mut lanes = Vec::with_capacity(4);
        for a in args {
            let v = self.expr(&a.value);
            let v = self.once(v, &mut pre);
            match v.ty {
                KTy::Struct(from) => {
                    let (dims, lane) = (self.dims(from), self.lane(from));
                    let o = self.operand(v, &mut pre);
                    lanes.extend((0..dims).map(|i| Self::component(&o, i, lane, line)));
                }
                // One vector: a copy, or (of the other kind) a conversion.
                from if from.is_vector() => {
                    let kind = if from == ty { v.kind } else { ExprKind::Cast(Box::new(v)) };
                    return Self::with_pre(pre, kind, ty, line);
                }
                _ => lanes.push(v),
            }
        }
        let kind = match <[Expr; 1]>::try_from(lanes) {
            Ok([one]) => ExprKind::Splat(Box::new(one)),
            Err(lanes) => ExprKind::VecLit(lanes),
        };
        Self::with_pre(pre, kind, ty, line)
    }

    /// `v.x` (lane `lane` of the vector `base`): a load of the lane when `v` is a place, else a
    /// lane of its value.
    pub(crate) fn lane_read(&mut self, e: &ast::Expr, base: &ast::Expr, lane: u32) -> ExprKind {
        match self.place(e) {
            Some(p) => ExprKind::Load(p),
            None => ExprKind::LaneOf(Box::new(self.expr(base)), lane as u8),
        }
    }

    /// A swizzle with a SIMD vector on either side: `v.wzyx` (a `Shuffle`), `v.xy` (a `vec2` of
    /// lanes of `v`, evaluated once) or `v3.xyzz` (a `vec4` of components of `v3`, evaluated once).
    pub(crate) fn simd_swizzle(&mut self, base: &ast::Expr, comps: &[u32], ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let v = self.expr(base);
        let kind = if ty.is_vector() && v.ty.is_vector() {
            let mut lanes = [0u8; 4];
            for (l, &c) in lanes.iter_mut().zip(comps) {
                *l = c as u8;
            }
            ExprKind::Shuffle(Box::new(v), lanes)
        } else if ty.is_vector() {
            let KTy::Struct(from) = v.ty else { unreachable!("a swizzle of a struct vector") };
            let lane = self.lane(from);
            let o = self.operand(v, &mut pre);
            ExprKind::VecLit(comps.iter().map(|&i| Self::component(&o, i, lane, line)).collect())
        } else {
            let KTy::Struct(s) = ty else { unreachable!("a swizzle of a vec4 to a struct vector") };
            let lane = v.ty.lane();
            let at = match v.kind {
                ExprKind::Load(p) if rereadable(&p) => p,
                _ => {
                    let ExprKind::Load(p) = self.spill(v, &mut pre).kind else { unreachable!("spill loads its temporary") };
                    p
                }
            };
            let fields = comps
                .iter()
                .map(|&i| {
                    let mut p = at.clone();
                    p.path.push(Proj::Lane(i as u8));
                    node(ExprKind::Load(p), lane, line)
                })
                .collect();
            ExprKind::StructLit(s, fields)
        };
        Self::with_pre(pre, kind, ty, line)
    }

    /// `l op r` on SIMD vectors of type `ty` (one side may be a number, which fills every lane):
    /// one operation of the vector type.
    pub(crate) fn simd_binary(op: BinOp, l: Expr, r: Expr, ty: KTy, line: u32) -> ExprKind {
        let lane = |x: Expr| if x.ty.is_vector() { x } else { Self::splat(x, ty, line) };
        let r = super::shift_amount(op, r, ty);
        let r = if matches!(op, BinOp::Shl | BinOp::Shr) { r } else { lane(r) };
        ExprKind::Binary(op, Box::new(lane(l)), Box::new(r))
    }

    /// `l == r` (every lane equal) or, with `ne`, `l != r` (some lane differs) where one side is a
    /// SIMD vector (a number fills every lane): one `bool`.
    pub(crate) fn simd_equal(ne: bool, l: Expr, r: Expr, line: u32) -> ExprKind {
        let ty = if l.ty.is_vector() { l.ty } else { r.ty };
        let op = if ne { BinOp::Ne } else { BinOp::Eq };
        Self::simd_binary(op, l, r, ty, line)
    }

    /// The number `x` in every lane of a vector of type `ty`.
    fn splat(x: Expr, ty: KTy, line: u32) -> Expr {
        node(ExprKind::Splat(Box::new(x)), ty, line)
    }
}
