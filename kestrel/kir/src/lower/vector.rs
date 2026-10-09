use kestrel_syntax::ast;

use super::{kty, FnLowerer};
use crate::ir::*;

/// One side of a vector operation, evaluated once.
enum Operand {
    /// A vector stored at a place; components are read from it.
    Vector(Place),
    /// A number applied to every component.
    Scalar(Expr),
}

impl<'a> FnLowerer<'a> {
    fn operand(&mut self, v: Expr, pre: &mut Vec<Stmt>) -> Operand {
        if v.ty.is_aggregate() {
            return match v.kind {
                ExprKind::Load(p) => Operand::Vector(p),
                _ => {
                    let ExprKind::Load(p) = self.spill(v, pre).kind else { unreachable!("spill loads its temporary") };
                    Operand::Vector(p)
                }
            };
        }
        match v.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Load(_) => Operand::Scalar(v),
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
