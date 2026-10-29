//! `atan2`, `atan`, `asin` and `acos` of a `float` inline, mirroring
//! `kestrel_runtime::math::wide::trig`: one arctangent core (`atan_core`) with the swap and the
//! `π/4` reduction picked by float compares, as the Rust does, kept in the float registers
//! (`fmask`; the swap's two picks are `minpd` and `maxpd`). Every special test, the sign pick and
//! the tiny pick are integer tests on the bits.

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, InstBuilder, Value};
use cranelift_module::Module;
use kestrel_kir::{KTy, MathOp};
use kestrel_runtime::math::wide;

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// Mirrors `wide::atan_core`: `atan2(ay, ax)` for the `f64`s `ay, ax >= 0`, or `π` minus it when
    /// the `i8` `x_negative` is set.
    pub(super) fn atan_core(&mut self, ay: Value, ax: Value, x_negative: Value) -> Value {
        let swap = self.fmask(FloatCC::GreaterThan, ay, ax);
        let num = self.pick(swap, ax, ay);
        let den = self.pick(swap, ay, ax);
        let t8 = self.f64c(wide::TAN_PI_8);
        let t8d = self.b.ins().fmul(den, t8);
        let big = self.fmask(FloatCC::GreaterThan, num, t8d);
        let diff = self.b.ins().fsub(num, den);
        let tn = self.pick(big, diff, num);
        let sum = self.b.ins().fadd(num, den);
        let td = self.pick(big, sum, den);
        let t = self.b.ins().fdiv(tn, td);
        let tt = self.b.ins().fmul(t, t);
        let p = self.estrin(&wide::A7, tt);
        let a = self.b.ins().fmul(t, p);
        let pi_4 = self.f64c(wide::PI_4);
        let shifted = self.b.ins().fadd(a, pi_4);
        let a = self.pick(big, shifted, a);
        let pi_2 = self.f64c(wide::PI_2);
        let rest = self.b.ins().fsub(pi_2, a);
        let a = self.pick(swap, rest, a);
        let pi = self.f64c(wide::PI);
        let other = self.b.ins().fsub(pi, a);
        self.pick(x_negative, other, a)
    }

    /// `atan2(y, x)`, `atan(x)`, `asin(x)` or `acos(x)` of `float`s, mirroring `wide::atan2f`,
    /// `atanf`, `asinf` and `acosf`: the fast path inline, the special inputs (`wide::atan2_special`,
    /// `atan_special`, `asin_special`) by a call of the runtime function.
    pub(super) fn wide_atan_family(&mut self, op: MathOp, vals: &[Value]) -> Value {
        let x = vals[0];
        let (fast, special) = match op {
            MathOp::Atan2 => {
                let (y, x) = (vals[0], vals[1]);
                let ay = self.widened_abs(y);
                let ax = self.widened_abs(x);
                let x_negative = self.sign_negative(x);
                let v = self.atan_core(ay, ax, x_negative);
                let v = self.b.ins().fdemote(types::F32, v);
                let fast = self.b.ins().fcopysign(v, y);
                // my >= INF_BITS || mx >= INF_BITS || (my | mx) == 0
                let y_big = self.magnitude_is(y, IntCC::UnsignedGreaterThanOrEqual, wide::INF_BITS);
                let x_big = self.magnitude_is(x, IntCC::UnsignedGreaterThanOrEqual, wide::INF_BITS);
                let (my, mx) = (self.magnitude(y), self.magnitude(x));
                let both = self.b.ins().bor(my, mx);
                let zeros = self.b.ins().icmp_imm_u(IntCC::Equal, both, 0);
                let s = self.b.ins().bor(y_big, x_big);
                (fast, self.b.ins().bor(s, zeros))
            }
            MathOp::Atan => {
                let a = self.widened_abs(x);
                let one = self.f64c(1.0);
                let no = self.b.ins().iconst(types::I8, 0);
                let v = self.atan_core(a, one, no);
                let fast = self.narrowed_with_sign(v, x);
                (fast, self.magnitude_at_least(x, wide::INF_BITS))
            }
            MathOp::Asin => {
                let (a, c) = self.asin_sides(x);
                let no = self.b.ins().iconst(types::I8, 0);
                let v = self.atan_core(a, c, no);
                let fast = self.narrowed_with_sign(v, x);
                (fast, self.magnitude_is(x, IntCC::UnsignedGreaterThan, wide::ONE_BITS))
            }
            MathOp::Acos => {
                let (a, c) = self.asin_sides(x);
                let x_negative = self.sign_negative(x);
                let v = self.atan_core(c, a, x_negative);
                let fast = self.b.ins().fdemote(types::F32, v);
                (fast, self.magnitude_is(x, IntCC::UnsignedGreaterThan, wide::ONE_BITS))
            }
            _ => unreachable!("wide_atan_family emits atan2, atan, asin and acos only"),
        };
        let slow = self.math_fn(op, KTy::F32);
        self.with_slow_path(special, fast, slow, vals)
    }

    /// `x.abs() as f64` for the `f32` `x`.
    fn widened_abs(&mut self, x: Value) -> Value {
        let a = self.b.ins().fabs(x);
        self.b.ins().fpromote(types::F64, a)
    }

    /// `x.is_sign_negative()` for the `f32` `x`, as an `i8`: its bits are negative as an `i32`.
    fn sign_negative(&mut self, x: Value) -> Value {
        let xb = self.bits(types::I32, x);
        self.b.ins().icmp_imm_s(IntCC::SignedLessThan, xb, 0)
    }

    /// The `(a, c)` of `wide::asinf` and `acosf` for the `f32` `x`: `a = x.abs() as f64` and
    /// `c = ((1.0 - a) * (1.0 + a)).sqrt()`.
    fn asin_sides(&mut self, x: Value) -> (Value, Value) {
        let a = self.widened_abs(x);
        let one = self.f64c(1.0);
        let l = self.b.ins().fsub(one, a);
        let r = self.b.ins().fadd(a, one);
        let p = self.b.ins().fmul(l, r);
        (a, self.b.ins().sqrt(p))
    }

    /// `if tiny(x) { x } else { (v as f32).copysign(x) }`: `wide::atanf` and `asinf` after their
    /// special test.
    fn narrowed_with_sign(&mut self, v: Value, x: Value) -> Value {
        let v = self.b.ins().fdemote(types::F32, v);
        let v = self.b.ins().fcopysign(v, x);
        let tiny = self.tiny(x);
        self.pick(tiny, x, v)
    }
}
