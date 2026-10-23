use kestrel_check::Builtin;
use kestrel_syntax::ast;

use super::builtins::{binary, node, number, select};
use super::{kty, shift_amount, FnLowerer};
use crate::ir::*;
use crate::visit::Node;

/// One side of a vector operation, evaluated once.
pub(super) enum Operand {
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

/// Whether evaluating `e` may change a variable: it calls a function (which may change a `var`
/// argument, or what a closure captured) or writes a place.
fn may_write(e: &Expr) -> bool {
    let mut writes = false;
    crate::visit::walk_expr(e, &mut |n| match n {
        Node::Expr(x) => writes |= matches!(x.kind, ExprKind::Call(..) | ExprKind::CallIndirect { .. } | ExprKind::CallVirtual { .. }),
        Node::Place(_, written) => writes |= written,
        Node::Stmt(_) => {}
    });
    writes
}

impl<'a> FnLowerer<'a> {
    /// `v` as something to read components from. A place whose path computes
    /// an index or key (`vs[f()]`) is copied to a temporary first, so that
    /// expression runs once, now, and not again for every component.
    pub(super) fn operand(&mut self, v: Expr, pre: &mut Vec<Stmt>) -> Operand {
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

    /// Component `i` of `o`, whose components are `lane`s (a number is every component).
    pub(super) fn component(o: &Operand, i: u32, lane: KTy, line: u32) -> Expr {
        match o {
            Operand::Vector(p) => {
                let mut p = p.clone();
                p.path.push(Proj::Field(i));
                Expr { kind: ExprKind::Load(p), ty: lane, line }
            }
            Operand::Scalar(e) => e.clone(),
        }
    }

    /// `l op r` component by component, as a new vector of type `ty`. Each component goes through
    /// the scalar operation of its lane type, with that type's check (an int vector's `+`, `/` and
    /// `%` each check as `int` does). A shift's amount is one number, as wide as a lane.
    pub(crate) fn vec_combine(&mut self, op: BinOp, l: Expr, r: Expr, ty: KTy, line: u32) -> ExprKind {
        if ty.is_vector() {
            return Self::simd_binary(op, l, r, ty, line);
        }
        let KTy::Struct(s) = ty else { unreachable!("vectors are structs") };
        let (dims, lane) = (self.dims(s), self.lane(s));
        let mut pre = Vec::new();
        // The left side is read first, as for numbers: when the right side may change it
        // (`n - g(var n)`), it is copied before the right side runs.
        let l = if may_write(&r) && !matches!(l.kind, ExprKind::Int(_) | ExprKind::Float(_)) { self.spill(l, &mut pre) } else { l };
        let (lo, ro) = (self.operand(l, &mut pre), self.operand(r, &mut pre));
        let fields = (0..dims)
            .map(|i| {
                let (a, b) = (Self::component(&lo, i, lane, line), Self::component(&ro, i, lane, line));
                let b = shift_amount(op, b, lane);
                Expr { kind: ExprKind::Binary(op, Box::new(a), Box::new(b)), ty: lane, line }
            })
            .collect();
        Self::with_pre(pre, ExprKind::StructLit(s, fields), ty, line)
    }

    /// `-v` and `~v`, component by component.
    pub(crate) fn vec_unary(&mut self, op: UnOp, v: Expr, ty: KTy, line: u32) -> ExprKind {
        if ty.is_vector() {
            return ExprKind::Unary(op, Box::new(v));
        }
        let KTy::Struct(s) = ty else { unreachable!("vectors are structs") };
        let (dims, lane) = (self.dims(s), self.lane(s));
        let mut pre = Vec::new();
        let o = self.operand(v, &mut pre);
        let fields = (0..dims)
            .map(|i| Expr { kind: ExprKind::Unary(op, Box::new(Self::component(&o, i, lane, line))), ty: lane, line })
            .collect();
        Self::with_pre(pre, ExprKind::StructLit(s, fields), ty, line)
    }

    /// `v.xz`: picked components as a new vector (of the same kind as `base`'s).
    pub(crate) fn swizzle(&mut self, base: &ast::Expr, comps: &[u32], ty: KTy, line: u32) -> ExprKind {
        if ty.is_vector() || kty(self.t.ty(base.id)).is_vector() {
            return self.simd_swizzle(base, comps, ty, line);
        }
        let KTy::Struct(s) = ty else { unreachable!("vectors are structs") };
        let lane = self.lane(s);
        let mut pre = Vec::new();
        let v = self.expr(base);
        let o = self.operand(v, &mut pre);
        let fields = comps.iter().map(|&i| Self::component(&o, i, lane, line)).collect();
        Self::with_pre(pre, ExprKind::StructLit(s, fields), ty, line)
    }

    /// A vector method (`length`, `normalized`, `dot`, `distance_to`, and `cross` on a `vec3`,
    /// on float vectors; `sum` on every vector), with the receiver, then the argument, evaluated
    /// once, in that order.
    pub(crate) fn vector_method(&mut self, b: Builtin, receiver: &ast::Expr, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let v = self.expr(receiver);
        let vty = v.ty;
        if vty.is_vector() {
            return self.simd_method(b, v, args, ty, line);
        }
        let KTy::Struct(s) = vty else { unreachable!("vector methods are called on vectors") };
        let dims = self.dims(s);
        let kind = match b {
            Builtin::Sum => {
                let o = self.operand(v, &mut pre);
                Self::sum(&o, dims, self.lane(s), line).kind
            }
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
                let unit = (0..dims).map(|i| binary(BinOp::Div, &Self::component(&o, i, KTy::F32, line), &len, KTy::F32, line)).collect();
                let zero = (0..dims).map(|_| number(0, KTy::F32, line)).collect();
                select(positive, node(ExprKind::StructLit(s, unit), ty, line), node(ExprKind::StructLit(s, zero), ty, line))
            }
            Builtin::Cross => {
                let l = self.operand(v, &mut pre);
                let w = self.expr(&args[0].value);
                let r = self.operand(w, &mut pre);
                let c = |o: &Operand, i| Self::component(o, i, KTy::F32, line);
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
        let product = |i| binary(BinOp::Mul, &Self::component(l, i, KTy::F32, line), &Self::component(r, i, KTy::F32, line), KTy::F32, line);
        (0..dims).map(product).reduce(|sum, p| binary(BinOp::Add, &sum, &p, KTy::F32, line)).expect("vectors have components")
    }

    /// The length of a vector.
    fn length(o: &Operand, dims: u32, line: u32) -> ExprKind {
        ExprKind::Math(MathOp::Sqrt, vec![Self::dot_sum(o, o, dims, line)])
    }

    /// Component count of a vector struct.
    pub(super) fn dims(&self, s: StructIdx) -> u32 {
        self.t.strukt(kestrel_check::StructId(s.0)).fields.len() as u32
    }

    /// The type of a vector struct's components: `F32` or `I32`.
    pub(super) fn lane(&self, s: StructIdx) -> KTy {
        kty(self.t.strukt(kestrel_check::StructId(s.0)).fields[0].ty)
    }

    /// The vector type of an AST expression, if it is one.
    pub(crate) fn vec_ty(&self, e: &ast::Expr) -> Option<KTy> {
        let ty = self.t.ty(e.id);
        self.t.vector(ty).map(|_| kty(ty))
    }
}
