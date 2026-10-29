//! `exp`, `log` and the remainder `%` of a `float` inline, mirroring `kestrel_runtime::math::wide`:
//! `exp_core`, `log_core`, `expf`, `logf` and `fmodf`, the same operations in the same order on the
//! same constants. The `exp` clamps (`maxpd` and `minpd`) and the `%` fix-up compare floats, as the
//! Rust does, but only to pick a value, in the float registers (`fmask`); every special test is an
//! integer test on the bits.

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, InstBuilder, Value};
use cranelift_module::Module;
use kestrel_kir::{KTy, MathOp};
use kestrel_runtime::math::wide;

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// Mirrors `wide::exp_core`: `e^z` for the `f64` `z`, clamped to `[EXP_LO, EXP_HI]` by picks
    /// (never `fmin`/`fmax`), reduced by `k ln2`, `c` evaluated on the rest and scaled by `2^k`.
    pub(super) fn exp_core<const N: usize>(&mut self, z: Value, c: &[f64; N]) -> Value {
        let lo = self.f64c(wide::EXP_LO);
        let below = self.fmask(FloatCC::LessThan, z, lo);
        let z = self.pick(below, lo, z);
        let hi = self.f64c(wide::EXP_HI);
        let above = self.fmask(FloatCC::GreaterThan, z, hi);
        let z = self.pick(above, hi, z);
        let log2e = self.f64c(wide::LOG2E);
        let v = self.b.ins().fmul(z, log2e);
        let (k, ki) = self.round_k(v);
        let ln2 = self.f64c(wide::LN2);
        let t = self.b.ins().fmul(k, ln2);
        let r = self.b.ins().fsub(z, t);
        let p = self.estrin(c, r);
        // f64::from_bits(((ki + EXP_BIAS) as u64) << EXP_SHIFT)
        let e = self.b.ins().iadd_imm_s(ki, wide::EXP_BIAS);
        let e = self.b.ins().ishl_imm_u(e, wide::EXP_SHIFT as i64);
        let scale = self.float_of(types::F64, e);
        self.b.ins().fmul(p, scale)
    }

    /// Mirrors `wide::log_core`: `ln x` for a positive normal `f64` `x`, from its exponent and
    /// `2 atanh(s)` of the reduced mantissa. The exponent becomes an `f64` by `fcvt_from_sint` of an
    /// `i64` (`cvtsi2sd`, no branch).
    pub(super) fn log_core(&mut self, x: Value) -> Value {
        let xb = self.bits(types::I64, x);
        let ix = self.b.ins().iadd_imm_u(xb, wide::ONE_BITS_64.wrapping_sub(wide::SQRT_HALF_BITS) as i64);
        let e = self.b.ins().ushr_imm_u(ix, wide::EXP_SHIFT as i64);
        let e = self.b.ins().iadd_imm_s(e, -wide::EXP_BIAS);
        let mb = self.b.ins().band_imm_u(ix, wide::MANTISSA_MASK_64 as i64);
        let mb = self.b.ins().iadd_imm_u(mb, wide::SQRT_HALF_BITS as i64);
        let m = self.float_of(types::F64, mb);
        let (one, two) = (self.f64c(1.0), self.f64c(2.0));
        let f = self.b.ins().fsub(m, one);
        let d = self.b.ins().fadd(f, two);
        let s = self.b.ins().fdiv(f, d);
        let u = self.b.ins().fmul(s, s);
        // g = L5[0] + u * (L5[1] + u * (L5[2] + u * (L5[3] + u * (L5[4] + u * L5[5]))))
        let c5 = self.f64c(wide::L5[5]);
        let mut g = self.b.ins().fmul(u, c5);
        for i in (0..5).rev() {
            let ci = self.f64c(wide::L5[i]);
            g = self.b.ins().fadd(g, ci);
            if i > 0 {
                g = self.b.ins().fmul(u, g);
            }
        }
        // e as f64 * LN2 + 2.0 * s * g
        let ef = self.b.ins().fcvt_from_sint(types::F64, e);
        let ln2 = self.f64c(wide::LN2);
        let a = self.b.ins().fmul(ef, ln2);
        let two_s = self.b.ins().fmul(s, two);
        let b = self.b.ins().fmul(two_s, g);
        self.b.ins().fadd(a, b)
    }

    /// `exp` of the `float` `x`, mirroring `wide::expf`: NaN (`wide::exp_special`) calls
    /// `kestrel_rt_expf`; the infinities stay on the fast path, through the clamps.
    pub(super) fn wide_exp(&mut self, x: Value) -> Value {
        let xd = self.b.ins().fpromote(types::F64, x);
        let v = self.exp_core(xd, &wide::E8);
        let fast = self.b.ins().fdemote(types::F32, v);
        let special = self.magnitude_is(x, IntCC::UnsignedGreaterThan, wide::INF_BITS);
        let slow = self.math_fn(MathOp::Exp, KTy::F32);
        self.with_slow_path(special, fast, slow, &[x])
    }

    /// `log` of the `float` `x`, mirroring `wide::logf`: zero, negative, infinite and NaN inputs
    /// (`wide::log_special`, `bits(x) - 1 >= NOT_POSITIVE_FINITE`, wrapping and unsigned) call
    /// `kestrel_rt_logf`.
    pub(super) fn wide_log(&mut self, x: Value) -> Value {
        let xd = self.b.ins().fpromote(types::F64, x);
        let v = self.log_core(xd);
        let fast = self.b.ins().fdemote(types::F32, v);
        let xb = self.bits(types::I32, x);
        let less = self.b.ins().iadd_imm_s(xb, -1);
        let special = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, less, wide::NOT_POSITIVE_FINITE as i64);
        let slow = self.math_fn(MathOp::Log, KTy::F32);
        self.with_slow_path(special, fast, slow, &[x])
    }

    /// The `float` remainder `a % b`, mirroring `wide::fmodf`: `a - q b` in `f64` with
    /// `q = trunc(a/b)` (`roundsd`), the fix-up and `over` steps, and a zero with `a`'s sign.
    /// `wide::fmod_special` inputs call `kestrel_rt_fmod_f32`. The two steps can't fire for a
    /// quotient below `FMOD_Q_LIMIT`: the division can't round `a/b` up to an integer `n` unless
    /// `n > 2^30`, and `n + 1` is representable (`wide::fmodf` gives the bounds). They are kept as
    /// a guard and so that this mirrors `wide::fmodf` step for step.
    pub(crate) fn wide_fmod(&mut self, a: Value, b: Value) -> Value {
        let ad = self.b.ins().fpromote(types::F64, a);
        let bd = self.b.ins().fpromote(types::F64, b);
        let qd = self.b.ins().fdiv(ad, bd);
        let q = self.b.ins().trunc(qd);
        let special = self.fmod_special(a, b, q);
        let qb = self.b.ins().fmul(q, bd);
        let r = self.b.ins().fsub(ad, qb);
        let bd_abs = self.b.ins().fabs(bd);
        let ab = self.b.ins().fcopysign(bd_abs, ad);
        // fix = r != 0.0 && (r < 0.0) != (ad < 0.0), as masks
        let zero = self.f64c(0.0);
        let nonzero = self.fmask(FloatCC::NotEqual, r, zero);
        let r_neg = self.fmask(FloatCC::LessThan, r, zero);
        let a_neg = self.fmask(FloatCC::LessThan, ad, zero);
        let signs = self.b.ins().bxor(r_neg, a_neg);
        let fix = self.b.ins().band(nonzero, signs);
        // over = !fix && r.abs() >= bd.abs()
        let r_abs = self.b.ins().fabs(r);
        let ge = self.fmask(FloatCC::GreaterThanOrEqual, r_abs, bd_abs);
        let over = self.b.ins().band_not(ge, fix);
        let up = self.b.ins().fadd(r, ab);
        let down = self.b.ins().fsub(r, ab);
        let r = {
            let rest = self.pick(over, down, r);
            self.pick(fix, up, rest)
        };
        // if r == 0.0 { 0f32.copysign(a) } else { r as f32 }
        let is_zero = self.fmask(FloatCC::Equal, r, zero);
        let zero32 = self.b.ins().f32const(0.0);
        let signed_zero = self.b.ins().fcopysign(zero32, a);
        let narrow = self.b.ins().fdemote(types::F32, r);
        let fast = self.pick(is_zero, signed_zero, narrow);
        let slow = self.shared.rt.fmod_f32;
        self.with_slow_path(special, fast, slow, &[a, b])
    }

    /// Mirrors `wide::fmod_special`: `a` infinite or NaN, `b` zero, infinite or NaN, or the `f64`
    /// quotient `q`'s magnitude bits at or above `bits(FMOD_Q_LIMIT)` (NaN too). Integer tests only.
    fn fmod_special(&mut self, a: Value, b: Value, q: Value) -> Value {
        let a_big = self.magnitude_is(a, IntCC::UnsignedGreaterThanOrEqual, wide::INF_BITS);
        let b_zero = self.magnitude_is(b, IntCC::Equal, 0);
        let b_big = self.magnitude_is(b, IntCC::UnsignedGreaterThanOrEqual, wide::INF_BITS);
        let qb = self.bits(types::I64, q);
        let qm = self.b.ins().band_imm_u(qb, wide::ABS_MASK_64 as i64);
        let q_big = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, qm, wide::FMOD_Q_LIMIT.to_bits() as i64);
        let s = self.b.ins().bor(a_big, b_zero);
        let s = self.b.ins().bor(s, b_big);
        self.b.ins().bor(s, q_big)
    }
}
