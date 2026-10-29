//! `float` math inline: the runtime's wide functions (`kestrel_runtime::math::wide`) emitted as
//! Cranelift IR. The fast path goes in the current block, pure and branch-free, and one `brif` on
//! the special-input test sends NaN, infinities and huge arguments to a cold block that calls the
//! runtime function (`kestrel_rt_<op>f`), whose own test takes today's path for them.
//!
//! Each helper here mirrors the Rust helper of the same name: the same IEEE operations in the same
//! order, on the same constants, read from `wide` and never copied. A pick between two values is an
//! integer `select` on their bits, never a float `select`, which on x64 is a jump (`XmmCmove`);
//! there is no `fmin`, `fmax` or float-to-int conversion either, which are jump sequences on x64.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, MemFlagsData, Opcode, Type, Value, ValueDef};
use cranelift_module::{FuncId, Module};
use kestrel_kir::MathOp;
use kestrel_runtime::math::wide;

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `op` on `float` operands `vals`: inline for every operation but `Pow`, which calls
    /// `kestrel_rt_powf`. (Until their emitters exist, the operations other than `sin`, `cos` and
    /// `tan` call their runtime function too, and so does `tan` when `wide::TAN_INLINE` is false.)
    pub(crate) fn wide_math(&mut self, op: MathOp, vals: &[Value]) -> Value {
        match op {
            MathOp::Sin | MathOp::Cos => self.sin_cos_tan(op, vals[0]),
            MathOp::Tan if wide::TAN_INLINE => self.sin_cos_tan(op, vals[0]),
            _ => self.runtime_math(op, vals, kestrel_kir::KTy::F32),
        }
    }

    /// The runtime's `float` function for `op` (`kestrel_rt_<op>f`).
    pub(super) fn mathf_id(&self, op: MathOp) -> FuncId {
        let (_, id) = *self.shared.rt.mathf.iter().find(|(o, _)| *o == op).expect("a runtime function for every float operation");
        id
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

    /// `if c { a } else { b }` for `f64` or `f32` values, as an integer `select` on their bits:
    /// never a float `select`.
    pub(super) fn pick(&mut self, c: Value, a: Value, b: Value) -> Value {
        let ty = self.b.func.dfg.value_type(a);
        let int = if ty == types::F64 { types::I64 } else { types::I32 };
        let (ai, bi) = (self.bits(int, a), self.bits(int, b));
        let v = self.b.ins().select(c, ai, bi);
        self.float_of(ty, v)
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
    /// `wide::cos_special` and `wide::tan_special`.
    pub(super) fn magnitude_at_least(&mut self, x: Value, bound: u32) -> Value {
        let m = self.magnitude(x);
        self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, m, bound as i64)
    }

    /// Mirrors `wide::tiny`: the `i8` flag `magnitude(x) < bits(TINY)`.
    pub(super) fn tiny(&mut self, x: Value) -> Value {
        let m = self.magnitude(x);
        self.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, m, wide::TINY.to_bits() as i64)
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
    #[expect(dead_code, reason = "the exp, log and arctangent emitters use it")]
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
                let t = self.b.ins().fmul(c8, x8);
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

    /// `a + b * x`, in that order (`estrin`'s pairs).
    #[expect(dead_code, reason = "estrin uses it")]
    pub(super) fn lin(&mut self, a: f64, b: f64, x: Value) -> Value {
        let (a, b) = (self.f64c(a), self.f64c(b));
        let t = self.b.ins().fmul(b, x);
        self.b.ins().fadd(a, t)
    }

    /// The `f64` constant `v`.
    pub(super) fn f64c(&mut self, v: f64) -> Value {
        self.b.ins().f64const(v)
    }
}
