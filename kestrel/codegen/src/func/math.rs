//! Float operations. `sqrt`, `floor`, `ceil`, `abs`, `min`, `max` and
//! `round` are Cranelift instructions; the rest call the runtime's
//! `kestrel_rt_math`, which works in `f64`.

use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, KTy, MathOp};
use kestrel_runtime::math as rt_op;

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `op` over `args`, evaluated in order. Operands and result have type `ty`.
    pub(crate) fn math(&mut self, op: MathOp, args: &[kir::Expr], ty: KTy) -> Result<Value, String> {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.expr(a)?);
        }
        let x = vals[0];
        Ok(match op {
            MathOp::Sqrt => self.b.ins().sqrt(x),
            MathOp::Floor => self.b.ins().floor(x),
            MathOp::Ceil => self.b.ins().ceil(x),
            MathOp::Abs => self.b.ins().fabs(x),
            MathOp::Min => self.b.ins().fmin(x, vals[1]),
            MathOp::Max => self.b.ins().fmax(x, vals[1]),
            MathOp::Round => self.round(x, ty),
            MathOp::Sin => self.runtime_math(rt_op::SIN, &vals, ty),
            MathOp::Cos => self.runtime_math(rt_op::COS, &vals, ty),
            MathOp::Tan => self.runtime_math(rt_op::TAN, &vals, ty),
            MathOp::Asin => self.runtime_math(rt_op::ASIN, &vals, ty),
            MathOp::Acos => self.runtime_math(rt_op::ACOS, &vals, ty),
            MathOp::Atan => self.runtime_math(rt_op::ATAN, &vals, ty),
            MathOp::Atan2 => self.runtime_math(rt_op::ATAN2, &vals, ty),
            MathOp::Exp => self.runtime_math(rt_op::EXP, &vals, ty),
            MathOp::Log => self.runtime_math(rt_op::LOG, &vals, ty),
            MathOp::Pow => self.runtime_math(rt_op::POW, &vals, ty),
        })
    }

    fn fconst(&mut self, ty: KTy, v: f64) -> Value {
        if ty == KTy::F32 {
            self.b.ins().f32const(v as f32)
        } else {
            self.b.ins().f64const(v)
        }
    }

    /// Rounds halves away from zero (`2.5` is `3`, `-2.5` is `-3`). Adding a
    /// half and flooring would round `0.49999997` up, so this keeps the
    /// fraction `x - trunc(x)` and compares it with a half.
    fn round(&mut self, x: Value, ty: KTy) -> Value {
        let t = self.b.ins().trunc(x);
        let d = self.b.ins().fsub(x, t);
        let ad = self.b.ins().fabs(d);
        let half = self.fconst(ty, 0.5);
        let ge = self.b.ins().fcmp(FloatCC::GreaterThanOrEqual, ad, half);
        let one = self.fconst(ty, 1.0);
        let one = self.b.ins().fcopysign(one, x);
        let up = self.b.ins().fadd(t, one);
        self.b.ins().select(ge, up, t)
    }

    /// Calls `kestrel_rt_math(op, a, b)`; an `f32` operand is widened to `f64` and the result narrowed back.
    fn runtime_math(&mut self, op: i32, vals: &[Value], ty: KTy) -> Value {
        let widen = |fl: &mut Self, v: Value| if ty == KTy::F32 { fl.b.ins().fpromote(types::F64, v) } else { v };
        let a = widen(self, vals[0]);
        let b = match vals.get(1) {
            Some(&v) => widen(self, v),
            None => self.b.ins().f64const(0.0),
        };
        let op = self.b.ins().iconst(types::I32, op as i64);
        let r = self.call_id(self.shared.rt.math, &[op, a, b]).expect("kestrel_rt_math returns a value");
        if ty == KTy::F32 {
            self.b.ins().fdemote(types::F32, r)
        } else {
            r
        }
    }
}
