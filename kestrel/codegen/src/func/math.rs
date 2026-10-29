//! Float operations. `sqrt`, `floor`, `ceil`, `abs`, `min`, `max` and
//! `round` are Cranelift instructions. Every other `float` operation is inline
//! code (`wide`, `wide_trig`, `wide_exp`, `wide_atan`) but `pow`, and `tan`
//! when `wide::TAN_INLINE` is false. The rest call the runtime's function for
//! that operation: the `libm` one for `f64`, and `kestrel_rt_powf` (or
//! `kestrel_rt_tanf`) for `float`, which works in `f32` (no widening).
//! The random numbers call the runtime's generator.

use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};
use cranelift_module::{FuncId, Module};
use kestrel_kir::{self as kir, KTy, MathOp};

use super::{cl_type, FuncLowerer};

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `op` over `args`, evaluated in order. Operands and result have type `ty`.
    pub(crate) fn math(&mut self, op: MathOp, args: &[kir::Expr], ty: KTy) -> Result<Value, String> {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.expr(a)?);
        }
        if ty.is_vector() {
            return Ok(self.simd_math(op, &vals, ty));
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
            // Inline, but for `pow` (and `tan` when `wide::TAN_INLINE` is false).
            _ if ty == KTy::F32 => self.wide_math(op, &vals),
            MathOp::Sin
            | MathOp::Cos
            | MathOp::Tan
            | MathOp::Asin
            | MathOp::Acos
            | MathOp::Atan
            | MathOp::Atan2
            | MathOp::Exp
            | MathOp::Log
            | MathOp::Pow => self.runtime_math(op, &vals, ty),
        })
    }

    /// A float from 0 up to (not including) 1.
    pub(crate) fn rand_float(&mut self, ty: KTy) -> Value {
        let r = self.call_id(self.shared.rt.rand_float, &[]).expect("kestrel_rt_rand_float returns a value");
        if ty != KTy::F32 {
            return r;
        }
        // Rounding to the nearest f32 turns a value just below 1 into 1, so
        // pull that back to the largest f32 below 1.
        let r = self.b.ins().fdemote(types::F32, r);
        let one = self.b.ins().f32const(1.0);
        let below_one = self.b.ins().f32const(f32::from_bits(1.0f32.to_bits() - 1));
        let reached_one = self.b.ins().fcmp(FloatCC::GreaterThanOrEqual, r, one);
        self.b.ins().select(reached_one, below_one, r)
    }

    /// An integer between `lo` and `hi`, both included, of type `ty`. The
    /// runtime works in `i64`; the result is within the operands, so
    /// narrowing it back is safe.
    pub(crate) fn rand_int(&mut self, lo: &kir::Expr, hi: &kir::Expr, ty: KTy) -> Result<Value, String> {
        let lo_v = self.expr(lo)?;
        let hi_v = self.expr(hi)?;
        let lo_v = self.widen_i64(lo_v, lo.ty);
        let hi_v = self.widen_i64(hi_v, hi.ty);
        let r = self.call_id(self.shared.rt.rand_int, &[lo_v, hi_v]).expect("kestrel_rt_rand_int returns a value");
        Ok(if ty == KTy::I64 { r } else { self.b.ins().ireduce(cl_type(ty), r) })
    }

    /// `rand_seed(n)`: the same seed makes the same sequence.
    pub(crate) fn rand_seed(&mut self, n: &kir::Expr) -> Result<(), String> {
        let v = self.expr(n)?;
        let v = self.widen_i64(v, n.ty);
        self.call_id(self.shared.rt.rand_seed, &[v]);
        Ok(())
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

    /// Calls the runtime's function for `op`. `f64` operands call the `libm` function; `float`
    /// operands call the wide function over `f32`, with no widening, so the result is computed in
    /// `float`.
    pub(super) fn runtime_math(&mut self, op: MathOp, vals: &[Value], ty: KTy) -> Value {
        let id = self.math_fn(op, ty);
        self.call_id(id, vals).expect("a runtime math function returns a value")
    }

    /// The runtime's function for `op` on `ty` operands: `kestrel_rt_<op>f` for `float`, the
    /// `libm` one for `f64`.
    pub(super) fn math_fn(&self, op: MathOp, ty: KTy) -> FuncId {
        let table = if ty == KTy::F32 { &self.shared.rt.mathf } else { &self.shared.rt.math };
        let (_, id) = *table.iter().find(|(o, _)| *o == op).expect("a runtime function for every float operation");
        id
    }
}
