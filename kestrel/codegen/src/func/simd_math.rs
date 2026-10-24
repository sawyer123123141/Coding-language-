//! The `Math` operations on SIMD vectors (`vec4`, `ivec4`), each one instruction or a short
//! sequence with no call: `sqrt`, `floor`, `ceil`, `abs`, `min` and `max` on `F32X4` (the same
//! Cranelift instructions as for a float, so a NaN lane behaves as the scalar does), `min` and
//! `max` on `I32X4` (`smin`, `smax`), and `round`, the scalar formula done on every lane at once.

use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{types, ConstantData, Endianness, InstBuilder, MemFlagsData, Type, Value};
use cranelift_module::Module;
use kestrel_kir::{KTy, MathOp};

use super::FuncLowerer;

/// The sign bit of an `f32` lane.
const SIGN: u32 = 0x8000_0000;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `op` over the vector values `vals` (already evaluated, in order) of type `ty`.
    pub(crate) fn simd_math(&mut self, op: MathOp, vals: &[Value], ty: KTy) -> Value {
        let x = vals[0];
        match (ty, op) {
            (KTy::F32X4, MathOp::Sqrt) => self.b.ins().sqrt(x),
            (KTy::F32X4, MathOp::Floor) => self.b.ins().floor(x),
            (KTy::F32X4, MathOp::Ceil) => self.b.ins().ceil(x),
            (KTy::F32X4, MathOp::Abs) => self.b.ins().fabs(x),
            (KTy::F32X4, MathOp::Min) => self.b.ins().fmin(x, vals[1]),
            (KTy::F32X4, MathOp::Max) => self.b.ins().fmax(x, vals[1]),
            (KTy::F32X4, MathOp::Round) => self.simd_round(x),
            (KTy::I32X4, MathOp::Min) => self.b.ins().smin(x, vals[1]),
            (KTy::I32X4, MathOp::Max) => self.b.ins().smax(x, vals[1]),
            _ => unreachable!("the checker allows no `{op:?}` on {ty:?}"),
        }
    }

    /// `round` on four lanes: halves away from zero, as the scalar one. The fraction `x - trunc(x)`
    /// is compared with a half (adding a half and flooring would round `0.49999997` up), and when
    /// it is at least that, one with the sign of `x` is added to the truncated value. A NaN or
    /// infinite lane compares false and stays as `trunc` leaves it.
    fn simd_round(&mut self, x: Value) -> Value {
        let little = MemFlagsData::new().with_endianness(Endianness::Little);
        let t = self.b.ins().trunc(x);
        let d = self.b.ins().fsub(x, t);
        let ad = self.b.ins().fabs(d);
        let half = self.splat_bits(0.5f32.to_bits(), types::F32X4);
        let at_least_half = self.b.ins().fcmp(FloatCC::GreaterThanOrEqual, ad, half);
        // `copysign(1.0, x)`: the sign bit of `x` on the bits of 1.0.
        let sign = self.splat_bits(SIGN, types::I32X4);
        let x_bits = self.b.ins().bitcast(types::I32X4, little, x);
        let x_sign = self.b.ins().band(x_bits, sign);
        let one_bits = self.splat_bits(1.0f32.to_bits(), types::I32X4);
        let signed_bits = self.b.ins().bor(one_bits, x_sign);
        let signed_one = self.b.ins().bitcast(types::F32X4, little, signed_bits);
        let up = self.b.ins().fadd(t, signed_one);
        let t_bits = self.b.ins().bitcast(types::I32X4, little, t);
        let up_bits = self.b.ins().bitcast(types::I32X4, little, up);
        let picked = self.b.ins().bitselect(at_least_half, up_bits, t_bits);
        self.b.ins().bitcast(types::F32X4, little, picked)
    }

    /// The vector constant of type `ty` with `bits` in every lane.
    fn splat_bits(&mut self, bits: u32, ty: Type) -> Value {
        let bytes: Vec<u8> = (0..4).flat_map(|_| bits.to_le_bytes()).collect();
        let c = self.b.func.dfg.constants.insert(ConstantData::from(bytes));
        self.b.ins().vconst(ty, c)
    }
}
