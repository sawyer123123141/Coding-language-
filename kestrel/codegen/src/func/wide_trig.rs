//! `sin`, `cos` and `tan` of a `float` inline, mirroring `kestrel_runtime::math::wide::trig`: one
//! range reduction and one pair of polynomials, which Cranelift's GVN shares between `sin(a)` and
//! `cos(a)` because the fast code of each sits in the block that dominates what follows.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};
use cranelift_module::Module;
use kestrel_kir::MathOp;
use kestrel_runtime::math::wide;

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// Mirrors `wide::reduce`: the `f32` `x` as `k π/2 + r`, giving `(r, k)` with `r` an `f64`
    /// and `k` an `i64`.
    fn reduce(&mut self, x: Value) -> (Value, Value) {
        let xd = self.b.ins().fpromote(types::F64, x);
        let inv = self.f64c(wide::INV_PIO2);
        let v = self.b.ins().fmul(xd, inv);
        let (k, ki) = self.round_k(v);
        let p1 = self.f64c(wide::PIO2_1);
        let a = self.b.ins().fmul(k, p1);
        let d = self.b.ins().fsub(xd, a);
        let p1t = self.f64c(wide::PIO2_1T);
        let b = self.b.ins().fmul(k, p1t);
        (self.b.ins().fsub(d, b), ki)
    }

    /// Mirrors `wide::sin_cos_poly`: `(sin r, cos r)` for the `f64` `r`.
    fn sin_cos_poly(&mut self, r: Value) -> (Value, Value) {
        let z = self.b.ins().fmul(r, r);
        let w = self.b.ins().fmul(z, z);
        let s = self.b.ins().fmul(z, r);
        // sin = (r + s * (S1 + z * S2)) + (s * w) * (S3 + z * S4)
        let (s1, s2) = (self.f64c(wide::S1), self.f64c(wide::S2));
        let t = self.b.ins().fmul(z, s2);
        let t = self.b.ins().fadd(s1, t);
        let t = self.b.ins().fmul(s, t);
        let lo = self.b.ins().fadd(r, t);
        let sw = self.b.ins().fmul(s, w);
        let (s3, s4) = (self.f64c(wide::S3), self.f64c(wide::S4));
        let t = self.b.ins().fmul(z, s4);
        let t = self.b.ins().fadd(s3, t);
        let hi = self.b.ins().fmul(sw, t);
        let sin = self.b.ins().fadd(lo, hi);
        // cos = ((1 + z * C0) + w * C1) + (w * z) * (C2 + z * C3)
        let (one, c0) = (self.f64c(1.0), self.f64c(wide::C0));
        let t = self.b.ins().fmul(z, c0);
        let t = self.b.ins().fadd(one, t);
        let c1 = self.f64c(wide::C1);
        let u = self.b.ins().fmul(w, c1);
        let lo = self.b.ins().fadd(t, u);
        let wz = self.b.ins().fmul(w, z);
        let (c2, c3) = (self.f64c(wide::C2), self.f64c(wide::C3));
        let t = self.b.ins().fmul(z, c3);
        let t = self.b.ins().fadd(c2, t);
        let hi = self.b.ins().fmul(wz, t);
        let cos = self.b.ins().fadd(lo, hi);
        (sin, cos)
    }

    /// Mirrors `wide::flip`: the `f64` `v` with its sign bit xored with `(k & 2) << 62`.
    fn flip(&mut self, v: Value, k: Value) -> Value {
        let k2 = self.b.ins().band_imm_u(k, 2);
        let sign = self.b.ins().ishl_imm_u(k2, 62);
        let vb = self.bits(types::I64, v);
        let fb = self.b.ins().bxor(vb, sign);
        self.float_of(types::F64, fb)
    }

    /// The `i8` flag `k & 1 == 0`.
    fn even(&mut self, k: Value) -> Value {
        let low = self.b.ins().band_imm_u(k, 1);
        self.b.ins().icmp_imm_u(IntCC::Equal, low, 0)
    }

    /// `sin`, `cos` or `tan` of the `float` `x`, mirroring `wide::sinf`, `cosf` and `tanf`: the
    /// fast path inline, the special inputs (`|x|` at or above the function's bound, or NaN) by a
    /// call of the runtime function.
    pub(super) fn sin_cos_tan(&mut self, op: MathOp, x: Value) -> Value {
        let (r, k) = self.reduce(x);
        let (s, c) = self.sin_cos_poly(r);
        let (fast, bound) = match op {
            MathOp::Sin => {
                let even = self.even(k);
                let v = self.pick(even, s, c);
                let v = self.flip(v, k);
                let v = self.b.ins().fdemote(types::F32, v);
                let tiny = self.tiny(x);
                (self.pick(tiny, x, v), wide::SIN_FAST)
            }
            MathOp::Cos => {
                let k = self.b.ins().iadd_imm_s(k, 1);
                let even = self.even(k);
                let v = self.pick(even, s, c);
                let v = self.flip(v, k);
                (self.b.ins().fdemote(types::F32, v), wide::COS_FAST)
            }
            MathOp::Tan => {
                let even = self.even(k);
                let neg_c = self.b.ins().fneg(c);
                let n = self.pick(even, s, neg_c);
                let d = self.pick(even, c, s);
                let q = self.b.ins().fdiv(n, d);
                let v = self.b.ins().fdemote(types::F32, q);
                let tiny = self.tiny(x);
                (self.pick(tiny, x, v), wide::TAN_FAST)
            }
            _ => unreachable!("sin_cos_tan emits sin, cos and tan only"),
        };
        let special = self.magnitude_at_least(x, bound.to_bits());
        let slow = self.math_fn(op, kestrel_kir::KTy::F32);
        self.with_slow_path(special, fast, slow, &[x])
    }
}
