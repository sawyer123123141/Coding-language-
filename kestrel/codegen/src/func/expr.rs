use cranelift_codegen::ir::{types, InstBuilder, MemFlags, TrapCode, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, ExprKind, KTy, UnOp};

use super::{cl_type, FuncLowerer};

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// Lowers a scalar-valued expression. Struct values go through `lower_into`.
    pub(crate) fn expr(&mut self, e: &kir::Expr) -> Result<Value, String> {
        let ty = cl_type(e.ty);
        Ok(match &e.kind {
            ExprKind::Int(n) => self.iconst(e.ty, *n),
            ExprKind::Float(f) => {
                if e.ty == KTy::F32 {
                    self.b.ins().f32const(*f as f32)
                } else {
                    self.b.ins().f64const(*f)
                }
            }
            ExprKind::Bool(v) => self.b.ins().iconst(types::I8, *v as i64),
            ExprKind::Load(p) => self.load_place(p)?,
            ExprKind::Len(p) => self.len(p)?,
            ExprKind::Map(op, p, args) => self.map_op(*op, p, args)?,
            ExprKind::Str(op, args) => self.str_scalar(*op, args)?,
            ExprKind::FieldOf(base, i) => {
                let addr = self.field_addr(base, *i)?;
                self.b.ins().load(ty, MemFlags::trusted(), addr, 0)
            }
            ExprKind::Seq(stmts, value) => {
                self.stmts(stmts)?;
                self.expr(value)?
            }
            ExprKind::Tag(p) => {
                let addr = self.place_addr(p)?;
                self.b.ins().load(types::I8, MemFlags::trusted(), addr, 0)
            }
            ExprKind::StructLit(..)
            | ExprKind::VariantLit(..)
            | ExprKind::StrLit(_)
            | ExprKind::Format(_)
            | ExprKind::ArrayNew(_)
            | ExprKind::ArrayLit(_)
            | ExprKind::ListNew
            | ExprKind::MapNew
            | ExprKind::NewArena
            | ExprKind::StaticArena
            | ExprKind::Some(_)
            | ExprKind::NoneLit
            | ExprKind::Move(_) => unreachable!("aggregate values are built in memory"),
            ExprKind::Unary(UnOp::Not, inner) => {
                let v = self.expr(inner)?;
                self.b.ins().bxor_imm(v, 1)
            }
            ExprKind::Unary(UnOp::BitNot, inner) => {
                let v = self.expr(inner)?;
                self.b.ins().bnot(v)
            }
            ExprKind::Unary(UnOp::Neg, inner) => {
                let v = self.expr(inner)?;
                if e.ty.is_float() {
                    self.b.ins().fneg(v)
                } else {
                    let zero = self.iconst(e.ty, 0);
                    self.int_arith(kir::BinOp::Sub, zero, v, e.ty, e.line)
                }
            }
            ExprKind::Binary(op, l, r) if l.ty == KTy::Str => self.str_compare(*op, l, r)?,
            ExprKind::Binary(op, l, r) => {
                let a = self.expr(l)?;
                let b = self.expr(r)?;
                let const_divisor = if let ExprKind::Int(c) = r.kind { Some(c) } else { None };
                self.binary(*op, a, b, l.ty, e.line, const_divisor)
            }
            ExprKind::And(l, r) => self.short_circuit(l, r, true)?,
            ExprKind::Or(l, r) => self.short_circuit(l, r, false)?,
            ExprKind::Cast(inner) => {
                let v = self.expr(inner)?;
                self.cast(v, inner.ty, e.ty, e.line)
            }
            ExprKind::Math(op, args) => self.math(*op, args, e.ty)?,
            ExprKind::RandFloat => self.rand_float(e.ty),
            ExprKind::RandInt(lo, hi) => self.rand_int(lo, hi, e.ty)?,
            ExprKind::Call(fi, args) => self.call(*fi, args, None)?.expect("value-returning call"),
            ExprKind::If { cond, then_body, then_val, else_body, else_val } => {
                let c = self.expr(cond)?;
                let (then_b, else_b, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                let result = self.b.append_block_param(merge, ty);
                self.b.ins().brif(c, then_b, &[], else_b, &[]);
                for (block, body, val) in [(then_b, then_body, then_val), (else_b, else_body, else_val)] {
                    self.b.switch_to_block(block);
                    if self.stmts(body)? {
                        match val {
                            Some(v) => {
                                let x = self.expr(v)?;
                                self.b.ins().jump(merge, &[x]);
                            }
                            None => {
                                self.b.ins().trap(TrapCode::unwrap_user(1));
                            }
                        }
                    }
                }
                self.b.switch_to_block(merge);
                result
            }
        })
    }

    /// An integer constant, masked to the type's width as Cranelift expects.
    pub(crate) fn iconst(&mut self, ty: KTy, n: i64) -> Value {
        let masked = match ty {
            KTy::Bool | KTy::U8 => n & 0xFF,
            KTy::I32 => n & 0xFFFF_FFFF,
            _ => n,
        };
        self.b.ins().iconst(cl_type(ty), masked)
    }

    fn short_circuit(&mut self, l: &kir::Expr, r: &kir::Expr, is_and: bool) -> Result<Value, String> {
        let a = self.expr(l)?;
        let (rhs_b, merge) = (self.b.create_block(), self.b.create_block());
        let result = self.b.append_block_param(merge, types::I8);
        if is_and {
            self.b.ins().brif(a, rhs_b, &[], merge, &[a]);
        } else {
            self.b.ins().brif(a, merge, &[a], rhs_b, &[]);
        }
        self.b.switch_to_block(rhs_b);
        let b = self.expr(r)?;
        self.b.ins().jump(merge, &[b]);
        self.b.switch_to_block(merge);
        Ok(result)
    }
}
