use kestrel_check::{ConstId, Res, StrMethod, Tables, Ty};
use kestrel_syntax::ast;

use super::builtins::constant_value;
use super::consts::{decl, hoisted};
use super::{binop, kty, FnLowerer};
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// Lowers `e` and applies any implicit conversion the checker recorded.
    pub(crate) fn expr(&mut self, e: &ast::Expr) -> Expr {
        let Some(&to) = self.t.coerce.get(&e.id) else { return self.expr_raw(e) };
        // A struct or enum value used as an interface value (or an optional of one).
        if let Some(v) = self.iface_coercion(e, to) {
            return v;
        }
        // A `T` used as a `T?`: wrap it (converting it to the inner type first).
        if let Ty::Optional(inner) = to {
            let inner = kty(self.t.elem(inner));
            let v = if kty(self.t.ty(e.id)) == inner { self.expr_raw(e) } else { self.converted(e, inner) };
            let line = v.line;
            return Expr { kind: ExprKind::Some(Box::new(v)), ty: kty(to), line };
        }
        self.converted(e, kty(to))
    }

    /// `e` converted to the type `to`.
    fn converted(&mut self, e: &ast::Expr, to: KTy) -> Expr {
        // A literal (or `PI` / `TAU`) converted to a float is emitted at the target type,
        // read from the source, so an f64 never inherits the rounding of an f32 value.
        if to.is_float() {
            if let Some(v) = literal_as(self.t, e, to) {
                return Expr { kind: ExprKind::Float(v), ty: to, line: self.line(e.span) };
            }
        }
        let raw = self.expr_raw(e);
        Expr { ty: to, line: raw.line, kind: ExprKind::Cast(Box::new(raw)) }
    }

    pub(crate) fn expr_raw(&mut self, e: &ast::Expr) -> Expr {
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
            ast::ExprKind::None => ExprKind::NoneLit,
            ast::ExprKind::Str(s) => ExprKind::StrLit(s.clone()),
            ast::ExprKind::Interp(parts) => ExprKind::Format(self.interp_parts(parts)),
            ast::ExprKind::Ident(_) => match self.t.res[&e.id] {
                Res::Local(_) if self.t.moves.contains(&e.id) => ExprKind::Move(self.place(e).expect("a local is a place")),
                Res::Local(_) => ExprKind::Load(self.place(e).expect("a local is a place")),
                // Built once, before `main`: a use reads the global in place (see `consts`).
                Res::Const(c) if hoisted(self.t, c) => ExprKind::Load(Place::local(self.const_local(c))),
                Res::Const(c) => return self.const_value(c),
                Res::Builtin(b) => Self::builtin_constant(b),
                Res::FnValue(f) => self.fn_ref(f),
                other => unreachable!("name resolved to {other:?}"),
            },
            ast::ExprKind::Unary { op: ast::UnaryOp::Neg, expr } if self.vec_ty(e).is_some() => {
                let v = self.expr(expr);
                self.vec_negate(v, ty, line)
            }
            ast::ExprKind::Unary { op, expr } => {
                let op = match op {
                    ast::UnaryOp::Neg => UnOp::Neg,
                    ast::UnaryOp::Not => UnOp::Not,
                    ast::UnaryOp::BitNot => UnOp::BitNot,
                };
                ExprKind::Unary(op, Box::new(self.expr(expr)))
            }
            ast::ExprKind::Binary { op: ast::BinaryOp::Add, lhs, rhs } if ty == KTy::Str => self.str_concat(lhs, rhs),
            ast::ExprKind::Binary { op, lhs, rhs } if self.vec_ty(e).is_some() => {
                let (l, r) = (self.expr(lhs), self.expr(rhs));
                self.vec_combine(binop(*op), l, r, ty, line)
            }
            ast::ExprKind::Binary { op, lhs, rhs } => {
                let l = Box::new(self.expr(lhs));
                match op {
                    ast::BinaryOp::And => ExprKind::And(l, Box::new(self.conditional(rhs))),
                    ast::BinaryOp::Or => ExprKind::Or(l, Box::new(self.conditional(rhs))),
                    other => {
                        let r = Box::new(self.expr(rhs));
                        let op = binop(*other);
                        let r = Box::new(super::shift_amount(op, *r, l.ty));
                        ExprKind::Binary(op, l, r)
                    }
                }
            }
            // A call through a function value: a local or constant by name, a field, or any expression.
            ast::ExprKind::Call { .. } | ast::ExprKind::MethodCall { .. } | ast::ExprKind::CallValue { .. }
                if matches!(self.t.res.get(&e.id), Some(Res::CallValue(_))) =>
            {
                self.call_value(e, ty, line)
            }
            ast::ExprKind::Call { args, .. } => match self.t.res[&e.id] {
                Res::Convert(_) => ExprKind::Cast(Box::new(self.expr(&args[0].value))),
                Res::NewArena => ExprKind::NewArena,
                Res::Builtin(b) => self.builtin_call(b, args, ty, line),
                Res::Fn(f) if self.wants_scratch(f, args) => self.scratch_call_value(e, f, args, ty, line),
                Res::Fn(f) => {
                    let (pre, fi, a) = self.call_parts(e, f, args, None);
                    Self::with_pre(pre, ExprKind::Call(fi, a), ty, line)
                }
                Res::Construct(sid) => {
                    let (pre, kind) = self.construct(e, sid, args);
                    Self::with_pre(pre, kind, ty, line)
                }
                other => unreachable!("{other:?} used as a value"),
            },
            ast::ExprKind::MethodCall { receiver, name, args, .. }
                if matches!(self.t.res.get(&e.id), Some(Res::MapRead | Res::MapRemove)) =>
            {
                self.map_method(receiver, name, args, ty, line)
            }
            ast::ExprKind::MethodCall { receiver, args, .. } => match self.t.res[&e.id] {
                Res::Fn(f) => {
                    let (pre, fi, a) = self.call_parts(e, f, args, Some(receiver));
                    Self::with_pre(pre, ExprKind::Call(fi, a), ty, line)
                }
                Res::IfaceCall { iface, slot } => self.iface_call_value(e, iface, slot, ty, line),
                Res::Variant(id, v) => {
                    let (pre, kind) = self.variant_construct(e, id, v, args);
                    Self::with_pre(pre, kind, ty, line)
                }
                Res::Builtin(b) => self.vector_method(b, receiver, args, ty, line),
                Res::List(method) => self.list_value(method, receiver, args, ty, line),
                Res::Str(method) => self.str_method(method, receiver, args, ty, line),
                other => unreachable!("method call resolved to {other:?}"),
            },
            ast::ExprKind::Field { base, .. } if matches!(self.t.ty(base.id), Ty::Array(_) | Ty::List(_) | Ty::Map(..)) => {
                self.len_value(base, line)
            }
            ast::ExprKind::Field { base, .. } if matches!(self.t.res.get(&e.id), Some(Res::Str(StrMethod::Len))) => {
                self.str_len(base, line)
            }
            ast::ExprKind::Index { base, index } if matches!(self.t.ty(base.id), Ty::Map(..)) => {
                self.map_get(base, index, ty, line)
            }
            ast::ExprKind::Index { base, index } if matches!(self.t.res.get(&e.id), Some(Res::Str(StrMethod::At))) => {
                self.str_at(base, index)
            }
            ast::ExprKind::Index { base, index } => self.index_value(e, base, index, ty, line),
            ast::ExprKind::ArrayLit(items) => ExprKind::ArrayLit(items.iter().map(|i| self.value(i)).collect()),
            ast::ExprKind::TypeCall { args, .. } => match ty {
                KTy::Array(_) => ExprKind::ArrayNew(Box::new(self.expr(&args[0].value))),
                KTy::Map(..) => ExprKind::MapNew,
                _ => ExprKind::ListNew,
            },
            ast::ExprKind::Field { base, .. } if self.t.swizzles.contains_key(&e.id) => {
                let comps = self.t.swizzles[&e.id].clone();
                self.swizzle(base, &comps, ty, line)
            }
            ast::ExprKind::Field { base, .. } => match (self.t.res.get(&e.id), self.place(e)) {
                (Some(Res::Variant(id, v)), _) => ExprKind::VariantLit(EnumIdx(id.0), *v, Vec::new()),
                (_, Some(place)) => ExprKind::Load(place),
                (_, None) => {
                    let base = self.expr(base);
                    self.field_of(base, self.t.field_index[&e.id], ty, line)
                }
            },
            ast::ExprKind::Match { .. } => self.match_value(e),
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
            ast::ExprKind::MapLit(entries) => self.map_lit(entries, ty, line),
            ast::ExprKind::CallValue { .. } => unreachable!("the checker resolves a call of a value to `Res::CallValue`"),
            ast::ExprKind::Closure(lit) => self.make_closure(e, lit),
        };
        Expr { kind, ty, line }
    }

    /// The initializer of a constant that holds no array, list or map, written out at each use.
    pub(super) fn const_value(&mut self, c: ConstId) -> Expr {
        let (t, program) = (self.t, self.program);
        self.expr(&decl(program, t, c).value)
    }
}

/// The value of a numeric literal or `PI` / `TAU` (possibly negated) as float type `to`,
/// or `None` if `e` is anything else.
fn literal_as(t: &Tables, e: &ast::Expr, to: KTy) -> Option<f64> {
    match &e.kind {
        ast::ExprKind::Int(n) if to == KTy::F32 => Some(*n as f32 as f64),
        ast::ExprKind::Int(n) => Some(*n as f64),
        ast::ExprKind::Float(f) => Some(*f),
        ast::ExprKind::Ident(_) => match t.res.get(&e.id) {
            Some(Res::Builtin(b)) => constant_value(*b),
            _ => None,
        },
        ast::ExprKind::Unary { op: ast::UnaryOp::Neg, expr } => literal_as(t, expr, to).map(|v| -v),
        _ => None,
    }
}
