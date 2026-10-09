use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, InstBuilder, TrapCode, Value};
use cranelift_module::Module;
use kestrel_kir::{BinOp, KTy};
use kestrel_runtime::{PANIC_DIV_ZERO, PANIC_OVERFLOW};

use super::{cl_type, FuncLowerer};

fn bits(t: KTy) -> u32 {
    match t {
        KTy::Bool | KTy::U8 => 8,
        KTy::I32 | KTy::F32 => 32,
        KTy::I64 | KTy::F64 => 64,
    }
}

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `a op b` where both operands have type `t`.
    pub(crate) fn binary(&mut self, op: BinOp, a: Value, b: Value, t: KTy, line: u32) -> Value {
        if op.is_comparison() {
            return self.compare(op, a, b, t);
        }
        if t.is_float() {
            return match op {
                BinOp::Add => self.b.ins().fadd(a, b),
                BinOp::Sub => self.b.ins().fsub(a, b),
                BinOp::Mul => self.b.ins().fmul(a, b),
                BinOp::Div => self.b.ins().fdiv(a, b),
                BinOp::Rem => {
                    let f = if t == KTy::F32 { self.shared.rt.fmod_f32 } else { self.shared.rt.fmod_f64 };
                    self.call_id(f, &[a, b]).expect("fmod returns a value")
                }
                _ => unreachable!("comparisons handled above"),
            };
        }
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul => self.int_arith(op, a, b, t, line),
            BinOp::Div | BinOp::Rem => self.int_div(op, a, b, t, line),
            _ => unreachable!("comparisons handled above"),
        }
    }

    /// Integer `+ - *`, overflow-checked when `checked_arith` is on.
    pub(crate) fn int_arith(&mut self, op: BinOp, a: Value, b: Value, t: KTy, line: u32) -> Value {
        let plain = |fl: &mut Self, x: Value, y: Value| match op {
            BinOp::Add => fl.b.ins().iadd(x, y),
            BinOp::Sub => fl.b.ins().isub(x, y),
            _ => fl.b.ins().imul(x, y),
        };
        if !self.shared.opts.checked_arith {
            return plain(self, a, b);
        }
        if t == KTy::U8 {
            // Compute in 32 bits; anything outside 0..=255 (including
            // negative results, which look huge unsigned) overflowed.
            let x = self.b.ins().uextend(types::I32, a);
            let y = self.b.ins().uextend(types::I32, b);
            let r = plain(self, x, y);
            let bad = self.b.ins().icmp_imm(IntCC::UnsignedGreaterThan, r, 255);
            self.panic_if(bad, PANIC_OVERFLOW, line);
            return self.b.ins().ireduce(types::I8, r);
        }
        let (r, overflowed) = match op {
            BinOp::Add => self.b.ins().sadd_overflow(a, b),
            BinOp::Sub => self.b.ins().ssub_overflow(a, b),
            _ => self.b.ins().smul_overflow(a, b),
        };
        self.panic_if(overflowed, PANIC_OVERFLOW, line);
        r
    }

    fn int_div(&mut self, op: BinOp, a: Value, b: Value, t: KTy, line: u32) -> Value {
        let is_zero = self.b.ins().icmp_imm(IntCC::Equal, b, 0);
        self.panic_if(is_zero, PANIC_DIV_ZERO, line);
        if !t.is_signed() {
            return if op == BinOp::Div { self.b.ins().udiv(a, b) } else { self.b.ins().urem(a, b) };
        }
        // MIN / -1 overflows (and traps in hardware).
        let min = if t == KTy::I32 { i32::MIN as i64 } else { i64::MIN };
        let min_v = self.iconst(t, min);
        let neg_one = self.iconst(t, -1);
        let a_is_min = self.b.ins().icmp(IntCC::Equal, a, min_v);
        let b_is_neg1 = self.b.ins().icmp(IntCC::Equal, b, neg_one);
        let both = self.b.ins().band(a_is_min, b_is_neg1);
        let divisor = if self.shared.opts.checked_arith {
            self.panic_if(both, PANIC_OVERFLOW, line);
            b
        } else {
            // Wrapping: MIN / 1 == MIN and MIN % 1 == 0, matching wrapping semantics.
            let one = self.iconst(t, 1);
            self.b.ins().select(both, one, b)
        };
        if op == BinOp::Div {
            self.b.ins().sdiv(a, divisor)
        } else {
            self.b.ins().srem(a, divisor)
        }
    }

    /// Calls the runtime panic (which doesn't return) when `cond` is true.
    fn panic_if(&mut self, cond: Value, kind: i32, line: u32) {
        let (fail, ok) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(cond, fail, &[], ok, &[]);
        self.b.switch_to_block(fail);
        self.b.set_cold_block(fail);
        let k = self.b.ins().iconst(types::I32, kind as i64);
        let l = self.b.ins().iconst(types::I32, line as i64);
        self.call_id(self.shared.rt.panic, &[k, l]);
        self.b.ins().trap(TrapCode::unwrap_user(2));
        self.b.switch_to_block(ok);
    }

    fn compare(&mut self, op: BinOp, a: Value, b: Value, t: KTy) -> Value {
        if t.is_float() {
            let cc = match op {
                BinOp::Eq => FloatCC::Equal,
                BinOp::Ne => FloatCC::NotEqual,
                BinOp::Lt => FloatCC::LessThan,
                BinOp::Gt => FloatCC::GreaterThan,
                BinOp::Le => FloatCC::LessThanOrEqual,
                _ => FloatCC::GreaterThanOrEqual,
            };
            return self.b.ins().fcmp(cc, a, b);
        }
        let signed = t.is_signed();
        let cc = match op {
            BinOp::Eq => IntCC::Equal,
            BinOp::Ne => IntCC::NotEqual,
            BinOp::Lt if signed => IntCC::SignedLessThan,
            BinOp::Lt => IntCC::UnsignedLessThan,
            BinOp::Gt if signed => IntCC::SignedGreaterThan,
            BinOp::Gt => IntCC::UnsignedGreaterThan,
            BinOp::Le if signed => IntCC::SignedLessThanOrEqual,
            BinOp::Le => IntCC::UnsignedLessThanOrEqual,
            _ if signed => IntCC::SignedGreaterThanOrEqual,
            _ => IntCC::UnsignedGreaterThanOrEqual,
        };
        self.b.ins().icmp(cc, a, b)
    }

    /// Converts `v` from `from` to `to`: int widening/narrowing (narrowing
    /// truncates), int<->float, float->int saturating (NaN -> 0).
    pub(crate) fn cast(&mut self, v: Value, from: KTy, to: KTy) -> Value {
        if from == to {
            return v;
        }
        let to_ty = cl_type(to);
        match (from.is_float(), to.is_float()) {
            (false, false) => {
                let (fb, tb) = (bits(from), bits(to));
                if tb > fb {
                    if from.is_signed() {
                        self.b.ins().sextend(to_ty, v)
                    } else {
                        self.b.ins().uextend(to_ty, v)
                    }
                } else if tb < fb {
                    self.b.ins().ireduce(to_ty, v)
                } else {
                    v
                }
            }
            (false, true) => {
                if from.is_signed() {
                    self.b.ins().fcvt_from_sint(to_ty, v)
                } else {
                    let wide = self.b.ins().uextend(types::I32, v);
                    self.b.ins().fcvt_from_sint(to_ty, wide)
                }
            }
            (true, false) => {
                if to == KTy::U8 {
                    let wide = self.b.ins().fcvt_to_sint_sat(types::I32, v);
                    let zero = self.b.ins().iconst(types::I32, 0);
                    let max = self.b.ins().iconst(types::I32, 255);
                    let lo = self.b.ins().smax(wide, zero);
                    let clamped = self.b.ins().smin(lo, max);
                    self.b.ins().ireduce(types::I8, clamped)
                } else {
                    self.b.ins().fcvt_to_sint_sat(to_ty, v)
                }
            }
            (true, true) => {
                if to == KTy::F64 {
                    self.b.ins().fpromote(types::F64, v)
                } else {
                    self.b.ins().fdemote(types::F32, v)
                }
            }
        }
    }
}
