//! `float` math inline: the runtime's wide functions (`kestrel_runtime::math::wide`) emitted as
//! Cranelift IR. The fast path goes in the current block, pure and branch-free, and one `brif` on
//! the special-input test sends NaN, infinities and huge arguments to a cold block that calls the
//! runtime function (`kestrel_rt_<op>f`), whose own test takes today's path for them.
//!
//! Each helper here mirrors the Rust helper of the same name: the same IEEE operations in the same
//! order, on the same constants, read from `wide` and never copied. A pick between two values is
//! never a float `select`, which on x64 is a jump (`XmmCmove`), but `pick` (`wide_pick.rs`); there
//! is no `fmin`, `fmax` or float-to-int conversion either, which are jump sequences on x64.
//!
//! A constant added or multiplied goes second (`x * c`, `t + c`, where the Rust may write `c * x`
//! or `c + t`): x64 then reads it from memory (`mulsd (%rip)`), where first it is built in a general
//! register and moved over (`movabsq`, `movq`) at every use, in loops too. IEEE addition and
//! multiplication are commutative, so the bits are the same.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, MemFlagsData, Opcode, Type, Value, ValueDef};
use cranelift_module::{FuncId, Module};
use kestrel_kir::{KTy, MathOp};
use kestrel_runtime::math::wide;

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `op` on `float` operands `vals`: inline for every operation but `Pow`, which calls
    /// `kestrel_rt_powf`, and `tan` when `wide::TAN_INLINE` is false, which calls `kestrel_rt_tanf`.
    /// (`sqrt`, `floor` and the other instructions never come here.)
    pub(crate) fn wide_math(&mut self, op: MathOp, vals: &[Value]) -> Value {
        match op {
            MathOp::Sin | MathOp::Cos => self.sin_cos_tan(op, vals[0]),
            MathOp::Tan if wide::TAN_INLINE => self.sin_cos_tan(op, vals[0]),
            MathOp::Exp => self.wide_exp(vals[0]),
            MathOp::Log => self.wide_log(vals[0]),
            MathOp::Atan2 | MathOp::Atan | MathOp::Asin | MathOp::Acos => self.wide_atan_family(op, vals),
            _ => self.runtime_math(op, vals, KTy::F32),
        }
    }

    /// `fast` (an `f32`), unless `special` (an `i8`) is set: then the cold block's call of `slow`
    /// on `args`.
    pub(super) fn with_slow_path(&mut self, special: Value, fast: Value, slow: FuncId, args: &[Value]) -> Value {
        let (cold, merge) = (self.b.create_block(), self.b.create_block());
        let result = self.b.append_block_param(merge, types::F32);
        self.b.ins().brif(special, cold, &[], merge, &[fast.into()]);
        self.b.switch_to_block(cold);
        self.b.set_cold_block(cold);
        let r = self.call_id(slow, args).expect("a runtime math function returns a value");
        self.b.ins().jump(merge, &[r.into()]);
        self.b.switch_to_block(merge);
        result
    }

    /// The bits of the float `v` as the integer type `int` of its size. A float that `float_of`
    /// made from an `int` gives back that integer, which saves a round trip between registers
    /// (`flip` after `pick`); Cranelift doesn't fold the two bitcasts itself.
    pub(super) fn bits(&mut self, int: Type, v: Value) -> Value {
        let dfg = &self.b.func.dfg;
        if let ValueDef::Result(inst, 0) = dfg.value_def(v) {
            if dfg.insts[inst].opcode() == Opcode::Bitcast {
                let src = dfg.inst_args(inst)[0];
                if dfg.value_type(src) == int {
                    return src;
                }
            }
        }
        self.b.ins().bitcast(int, MemFlagsData::new(), v)
    }

    /// The float of type `ty` whose bits are the integer `v`.
    pub(super) fn float_of(&mut self, ty: Type, v: Value) -> Value {
        self.b.ins().bitcast(ty, MemFlagsData::new(), v)
    }

    /// Mirrors `wide::magnitude`: the bits of the `f32` `x` without the sign, an `i32`.
    pub(super) fn magnitude(&mut self, x: Value) -> Value {
        let b = self.bits(types::I32, x);
        self.b.ins().band_imm_u(b, wide::ABS_MASK as i64)
    }

    /// The `i8` flag `magnitude(x) >= bound` (unsigned): true for NaN and for `|x| >= bound`.
    /// With `bound` the bits of `SIN_FAST`, `COS_FAST` or `TAN_FAST`, it mirrors `wide::sin_special`,
    /// `wide::cos_special` and `wide::tan_special`; with `INF_BITS`, `wide::atan_special`.
    pub(super) fn magnitude_at_least(&mut self, x: Value, bound: u32) -> Value {
        self.magnitude_is(x, IntCC::UnsignedGreaterThanOrEqual, bound)
    }

    /// The `i8` flag `magnitude(x) <cc> bound`, an integer compare of the bits (`cc` unsigned).
    pub(super) fn magnitude_is(&mut self, x: Value, cc: IntCC, bound: u32) -> Value {
        let m = self.magnitude(x);
        self.b.ins().icmp_imm_u(cc, m, bound as i64)
    }

    /// Mirrors `wide::tiny`: the `i8` flag `magnitude(x) < bits(TINY)`.
    pub(super) fn tiny(&mut self, x: Value) -> Value {
        self.magnitude_is(x, IntCC::UnsignedLessThan, wide::TINY.to_bits())
    }

    /// Mirrors `wide::round_k`: `v` rounded to the nearest integer, as an `f64` `k` and an `i64`,
    /// for `|v| < 2^51`, by the `ROUND` add, with no float-to-int conversion.
    pub(super) fn round_k(&mut self, v: Value) -> (Value, Value) {
        let round = self.f64c(wide::ROUND);
        let t = self.b.ins().fadd(v, round);
        let k = self.b.ins().fsub(t, round);
        let tb = self.bits(types::I64, t);
        let rb = self.b.ins().iconst(types::I64, wide::ROUND.to_bits() as i64);
        (k, self.b.ins().isub(tb, rb))
    }

    /// Mirrors `wide::estrin`: `c[0] + c[1] x + ...` by Estrin's scheme, in the same order, for 8,
    /// 9 or 10 coefficients.
    pub(super) fn estrin<const N: usize>(&mut self, c: &[f64; N], x: Value) -> Value {
        let x2 = self.b.ins().fmul(x, x);
        let x4 = self.b.ins().fmul(x2, x2);
        let p01 = self.lin(c[0], c[1], x);
        let p23 = self.lin(c[2], c[3], x);
        let p45 = self.lin(c[4], c[5], x);
        let p67 = self.lin(c[6], c[7], x);
        let t = self.b.ins().fmul(p23, x2);
        let q0 = self.b.ins().fadd(p01, t);
        let t = self.b.ins().fmul(p67, x2);
        let q1 = self.b.ins().fadd(p45, t);
        let t = self.b.ins().fmul(q1, x4);
        let s0 = self.b.ins().fadd(q0, t);
        match N {
            8 => s0,
            9 => {
                let c8 = self.f64c(c[8]);
                let x8 = self.b.ins().fmul(x4, x4);
                let t = self.b.ins().fmul(x8, c8);
                self.b.ins().fadd(s0, t)
            }
            10 => {
                let p89 = self.lin(c[8], c[9], x);
                let x8 = self.b.ins().fmul(x4, x4);
                let t = self.b.ins().fmul(p89, x8);
                self.b.ins().fadd(s0, t)
            }
            _ => panic!("estrin takes 8, 9 or 10 coefficients"),
        }
    }

    /// `a + b * x`, in that order (`estrin`'s pairs), the constants second (`x * b`, `t + a`).
    pub(super) fn lin(&mut self, a: f64, b: f64, x: Value) -> Value {
        let (a, b) = (self.f64c(a), self.f64c(b));
        let t = self.b.ins().fmul(x, b);
        self.b.ins().fadd(t, a)
    }

    /// The `f64` constant `v`.
    pub(super) fn f64c(&mut self, v: f64) -> Value {
        self.b.ins().f64const(v)
    }
}
