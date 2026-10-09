use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, InstBuilder, TrapCode, Value};
use cranelift_module::Module;
use kestrel_kir::{BinOp, KTy};
use kestrel_runtime::{PANIC_NARROW_INT, PANIC_NARROW_U8, PANIC_OVERFLOW, PANIC_SHIFT};

use super::{cl_type, FuncLowerer};

fn bits(t: KTy) -> u32 {
    match t {
        KTy::Bool | KTy::U8 => 8,
        KTy::I32 | KTy::F32 => 32,
        KTy::I64 | KTy::F64 => 64,
        KTy::Struct(_) | KTy::Enum(_) | KTy::Str | KTy::Array(_) | KTy::List(_) | KTy::Arena | KTy::Optional(_) => {
            unreachable!("aggregates and arenas aren't numbers")
        }
    }
}

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `a op b` where both operands have type `t`. `const_divisor` is `b`'s
    /// value when it is an integer literal (used only by `/` and `%`).
    pub(crate) fn binary(
        &mut self,
        op: BinOp,
        a: Value,
        b: Value,
        t: KTy,
        line: u32,
        const_divisor: Option<i64>,
    ) -> Value {
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
            BinOp::Div | BinOp::Rem => self.int_div(op, a, b, t, line, const_divisor),
            BinOp::BitAnd => self.b.ins().band(a, b),
            BinOp::BitOr => self.b.ins().bor(a, b),
            BinOp::BitXor => self.b.ins().bxor(a, b),
            BinOp::Shl | BinOp::Shr => self.shift(op, a, b, t, line),
            _ => unreachable!("comparisons handled above"),
        }
    }

    /// `a << b` / `a >> b` (arithmetic for signed types). With checks on, an
    /// amount that's negative or at least the width panics; otherwise the
    /// machine masks it.
    fn shift(&mut self, op: BinOp, a: Value, b: Value, t: KTy, line: u32) -> Value {
        if self.shared.opts.checked_arith {
            // Unsigned compare: a negative amount looks huge.
            let bad = self.b.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, b, bits(t) as i64);
            self.panic_if(bad, PANIC_SHIFT, line);
        }
        match (op, t.is_signed()) {
            (BinOp::Shl, _) => self.b.ins().ishl(a, b),
            (_, true) => self.b.ins().sshr(a, b),
            _ => self.b.ins().ushr(a, b),
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

    /// Calls the runtime panic (which doesn't return) when `cond` is true.
    pub(super) fn panic_if(&mut self, cond: Value, kind: i32, line: u32) {
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
    /// panics on a value that doesn't fit when `checked_arith` is on, and
    /// truncates otherwise), int<->float, float->int saturating (NaN -> 0).
    pub(crate) fn cast(&mut self, v: Value, from: KTy, to: KTy, line: u32) -> Value {
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
                    let r = self.b.ins().ireduce(to_ty, v);
                    if self.shared.opts.checked_arith {
                        // It fit if widening it back gives the original value.
                        let back = if to.is_signed() {
                            self.b.ins().sextend(cl_type(from), r)
                        } else {
                            self.b.ins().uextend(cl_type(from), r)
                        };
                        let bad = self.b.ins().icmp(IntCC::NotEqual, back, v);
                        let kind = if to == KTy::U8 { PANIC_NARROW_U8 } else { PANIC_NARROW_INT };
                        self.panic_if(bad, kind, line);
                    }
                    r
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
