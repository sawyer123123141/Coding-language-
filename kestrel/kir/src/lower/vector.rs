use kestrel_check::Builtin;
use kestrel_syntax::ast;

use super::builtins::{binary, node, number, select};
use super::{kty, FnLowerer};
use crate::ir::*;

/// One side of a vector operation, evaluated once.
enum Operand {
    /// A vector stored at a place; components are read from it.
    Vector(Place),
    /// A number applied to every component.
    Scalar(Expr),
}

/// Whether a place can be read again and again: nothing in its path (an
/// index, a map key) is an expression that would be evaluated again.
fn rereadable(p: &Place) -> bool {
    p.path.iter().all(|step| step.at().is_none())
}

impl<'a> FnLowerer<'a> {
    /// `v` as something to read components from. A place whose path computes
    /// an index or key (`vs[f()]`) is copied to a temporary first, so that
    /// expression runs once, now, and not again for every component.
    fn operand(&mut self, v: Expr, pre: &mut Vec<Stmt>) -> Operand {
        if v.ty.is_aggregate() {
            return match v.kind {
                ExprKind::Load(p) if rereadable(&p) => Operand::Vector(p),
                _ => {
                    let ExprKind::Load(p) = self.spill(v, pre).kind else { unreachable!("spill loads its temporary") };
                    Operand::Vector(p)
                }
            };
        }
        match &v.kind {
            ExprKind::Int(_) | ExprKind::Float(_) => Operand::Scalar(v),
            ExprKind::Load(p) if rereadable(p) => Operand::Scalar(v),
            _ => Operand::Scalar(self.spill(v, pre)),
        }
    }

    fn component(o: &Operand, i: u32, line: u32) -> Expr {
        match o {
            Operand::Vector(p) => {
                let mut p = p.clone();
                p.path.push(Proj::Field(i));
                Expr { kind: ExprKind::Load(p), ty: KTy::F32, line }
            }
            Operand::Scalar(e) => e.clone(),
        }
    }

    /// `l op r` component by component, as a new vector of type `ty`.
    pub(crate) fn vec_combine(&mut self, op: BinOp, l: Expr, r: Expr, ty: KTy, line: u32) -> ExprKind {
        let KTy::Struct(s) = ty else { unreachable!("vectors are structs") };
        let dims = self.dims(s);
        let mut pre = Vec::new();
        let (lo, ro) = (self.operand(l, &mut pre), self.operand(r, &mut pre));
        let fields = (0..dims)
            .map(|i| {
                let (a, b) = (Self::component(&lo, i, line), Self::component(&ro, i, line));
                Expr { kind: ExprKind::Binary(op, Box::new(a), Box::new(b)), ty: KTy::F32, line }
            })
            .collect();
        Self::with_pre(pre, ExprKind::StructLit(s, fields), ty, line)
    }

    /// `-v`
    pub(crate) fn vec_negate(&mut self, v: Expr, ty: KTy, line: u32) -> ExprKind {
        let KTy::Struct(s) = ty else { unreachable!("vectors are structs") };
        let dims = self.dims(s);
        let mut pre = Vec::new();
        let o = self.operand(v, &mut pre);
        let fields = (0..dims)
            .map(|i| Expr { kind: ExprKind::Unary(UnOp::Neg, Box::new(Self::component(&o, i, line))), ty: KTy::F32, line })
            .collect();
        Self::with_pre(pre, ExprKind::StructLit(s, fields), ty, line)
    }

    /// `v.xz`: picked components as a new vector.
    pub(crate) fn swizzle(&mut self, base: &ast::Expr, comps: &[u32], ty: KTy, line: u32) -> ExprKind {
        let KTy::Struct(s) = ty else { unreachable!("vectors are structs") };
        let mut pre = Vec::new();
        let v = self.expr(base);
        let o = self.operand(v, &mut pre);
        let fields = comps.iter().map(|&i| Self::component(&o, i, line)).collect();
        Self::with_pre(pre, ExprKind::StructLit(s, fields), ty, line)
    }

    /// A `vec2` / `vec3` method (`length`, `normalized`, `dot`, `distance_to`,
    /// and `cross` on a `vec3`), with the receiver, then the argument,
    /// evaluated once, in that order.
    pub(crate) fn vector_method(&mut self, b: Builtin, receiver: &ast::Expr, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let v = self.expr(receiver);
        let vty = v.ty;
        let KTy::Struct(s) = vty else { unreachable!("vector methods are called on vectors") };
        let dims = self.dims(s);
        let kind = match b {
            Builtin::Length => {
                let o = self.operand(v, &mut pre);
                Self::length(&o, dims, line)
            }
            Builtin::Dot => {
                let l = self.operand(v, &mut pre);
                let w = self.expr(&args[0].value);
                let r = self.operand(w, &mut pre);
                Self::dot_sum(&l, &r, dims, line).kind
            }
            Builtin::DistanceTo => {
                let w = self.expr(&args[0].value);
                let diff = node(self.vec_combine(BinOp::Sub, v, w, vty, line), vty, line);
                let d = self.operand(diff, &mut pre);
                Self::length(&d, dims, line)
            }
            Builtin::Normalized => {
                let o = self.operand(v, &mut pre);
                let len = node(Self::length(&o, dims, line), KTy::F32, line);
                let len = self.spill(len, &mut pre);
                // A zero vector stays zero.
                let positive = binary(BinOp::Gt, &len, &number(0, KTy::F32, line), KTy::Bool, line);
                let unit = (0..dims).map(|i| binary(BinOp::Div, &Self::component(&o, i, line), &len, KTy::F32, line)).collect();
                let zero = (0..dims).map(|_| number(0, KTy::F32, line)).collect();
                select(positive, node(ExprKind::StructLit(s, unit), ty, line), node(ExprKind::StructLit(s, zero), ty, line))
            }
            Builtin::Cross => {
                let l = self.operand(v, &mut pre);
                let w = self.expr(&args[0].value);
                let r = self.operand(w, &mut pre);
                let c = |o: &Operand, i| Self::component(o, i, line);
                let term = |i, j| {
                    let ij = binary(BinOp::Mul, &c(&l, i), &c(&r, j), KTy::F32, line);
                    let ji = binary(BinOp::Mul, &c(&l, j), &c(&r, i), KTy::F32, line);
                    binary(BinOp::Sub, &ij, &ji, KTy::F32, line)
                };
                ExprKind::StructLit(s, vec![term(1, 2), term(2, 0), term(0, 1)])
            }
            other => unreachable!("{other:?} isn't a vector method"),
        };
        Self::with_pre(pre, kind, ty, line)
    }

    /// The sum of the products of the components of two vectors.
    fn dot_sum(l: &Operand, r: &Operand, dims: u32, line: u32) -> Expr {
        let product = |i| binary(BinOp::Mul, &Self::component(l, i, line), &Self::component(r, i, line), KTy::F32, line);
        (0..dims).map(product).reduce(|sum, p| binary(BinOp::Add, &sum, &p, KTy::F32, line)).expect("vectors have components")
    }

    /// The length of a vector.
    fn length(o: &Operand, dims: u32, line: u32) -> ExprKind {
        ExprKind::Math(MathOp::Sqrt, vec![Self::dot_sum(o, o, dims, line)])
    }

    /// Component count of a vector struct.
    fn dims(&self, s: StructIdx) -> u32 {
        self.t.strukt(kestrel_check::StructId(s.0)).fields.len() as u32
    }

    /// The vector type of an AST expression, if it is one.
    pub(crate) fn vec_ty(&self, e: &ast::Expr) -> Option<KTy> {
        let ty = self.t.ty(e.id);
        self.t.vec_dims(ty).map(|_| kty(ty))
    }
}
