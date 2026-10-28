//! `ExprKind::Hash`: one step of `checksum`'s fold, as the runtime's `checksum` defines it. A
//! number becomes one 64-bit word (a NaN the canonical quiet NaN first) and is mixed in place; a
//! string's bytes go to `kestrel_rt_hash_bytes`.

use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{types, InstBuilder, MemFlagsData, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, HashOp, KTy};

use super::collections::LEN;
use super::FuncLowerer;

/// The multiplier of one fold step (the runtime's `checksum::MULTIPLIER`).
const MULTIPLIER: i64 = 0x9E37_79B9_7F4A_7C15_u64 as i64;
/// SplitMix64's finaliser: its two multipliers.
const FINISH_1: i64 = 0xBF58_476D_1CE4_E5B9_u64 as i64;
const FINISH_2: i64 = 0x94D0_49BB_1331_11EB_u64 as i64;
/// The canonical quiet NaNs a `float` and an `f64` NaN become.
const NAN_32: i64 = 0x7FC0_0000;
const NAN_64: i64 = 0x7FF8_0000_0000_0000;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `Hash(op, args)`, an `i64`.
    pub(crate) fn hash(&mut self, op: HashOp, args: &[kir::Expr]) -> Result<Value, String> {
        let h = self.expr(&args[0])?;
        Ok(match op {
            HashOp::Mix => self.mix_value(h, &args[1])?,
            HashOp::Finish => self.finish(h),
        })
    }

    /// The value `x` mixed into `h`.
    fn mix_value(&mut self, h: Value, x: &kir::Expr) -> Result<Value, String> {
        Ok(match x.ty {
            KTy::Str => {
                let addr = self.materialize(x)?;
                let ptr = self.b.ins().load(self.ptr, MemFlagsData::trusted(), addr, 0);
                let len = self.b.ins().load(types::I64, MemFlagsData::trusted(), addr, LEN);
                self.call_id(self.shared.rt.hash_bytes, &[h, ptr, len]).expect("returns the fold")
            }
            KTy::F32X4 | KTy::I32X4 => {
                let v = self.expr(x)?;
                let mut h = h;
                for lane in 0..4 {
                    let l = self.b.ins().extractlane(v, lane);
                    let w = self.word(l, x.ty.lane());
                    h = self.mix(h, w);
                }
                h
            }
            ty => {
                let v = self.expr(x)?;
                let w = self.word(v, ty);
                self.mix(h, w)
            }
        })
    }

    /// The 64-bit word of the number `v` of type `ty`.
    fn word(&mut self, v: Value, ty: KTy) -> Value {
        match ty {
            KTy::Bool | KTy::U8 => self.b.ins().uextend(types::I64, v),
            KTy::I32 => self.b.ins().sextend(types::I64, v),
            KTy::I64 => v,
            KTy::F32 => {
                let bits = self.canonical_bits(v, types::I32, NAN_32);
                self.b.ins().uextend(types::I64, bits)
            }
            KTy::F64 => self.canonical_bits(v, types::I64, NAN_64),
            other => unreachable!("`Hash(Mix)` of a {other:?}, which isn't one word"),
        }
    }

    /// The bits (an integer of type `int`) of the float `v`, or `nan` when `v` is a NaN.
    fn canonical_bits(&mut self, v: Value, int: types::Type, nan: i64) -> Value {
        let is_nan = self.b.ins().fcmp(FloatCC::Unordered, v, v);
        let bits = self.b.ins().bitcast(int, MemFlagsData::new(), v);
        let canonical = self.b.ins().iconst(int, nan);
        self.b.ins().select(is_nan, canonical, bits)
    }

    /// One fold step: `x = (h ^ w) * MULTIPLIER; x ^ (x >> 32)`, wrapping, unsigned shift.
    fn mix(&mut self, h: Value, w: Value) -> Value {
        let x = self.b.ins().bxor(h, w);
        let x = self.b.ins().imul_imm_s(x, MULTIPLIER);
        let high = self.b.ins().ushr_imm_s(x, 32);
        self.b.ins().bxor(x, high)
    }

    /// SplitMix64's finaliser of `h`.
    fn finish(&mut self, h: Value) -> Value {
        let mut z = h;
        for (shift, multiplier) in [(30, Some(FINISH_1)), (27, Some(FINISH_2)), (31, None)] {
            let high = self.b.ins().ushr_imm_s(z, shift);
            z = self.b.ins().bxor(z, high);
            if let Some(m) = multiplier {
                z = self.b.ins().imul_imm_s(z, m);
            }
        }
        z
    }
}
