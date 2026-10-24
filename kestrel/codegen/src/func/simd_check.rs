//! The checks of `ivec4` arithmetic (spec R6-R8): a checked build stops when any lane of `+ - *`
//! or `-v` overflows `int`, or when a shift's one amount is out of range, exactly as `int` would.
//! x64 has no overflow flag for lanes and Cranelift lowers no `_overflow` instruction on vectors,
//! so each check is built from lane tricks with no branch per lane, and the value itself is the
//! plain instruction (it stays hoistable, as the scalar checks' design wants). Every check goes
//! through `fail_if`, so `Emit`, `Skip` and `Verify` all work, and each is a site that
//! `range::sites::checks_of` lists.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};
use cranelift_module::Module;
use kestrel_kir::BinOp;
use kestrel_runtime::{PANIC_OVERFLOW, PANIC_SHIFT};

use super::{Check, FuncLowerer};

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `a + b`, `a - b` or `a * b` on two `I32X4`s, overflow-checked lane by lane when `check`
    /// says so.
    pub(super) fn ivec4_arith(&mut self, op: BinOp, a: Value, b: Value, line: u32, check: Check) -> Value {
        let r = match op {
            BinOp::Add => self.b.ins().iadd(a, b),
            BinOp::Sub => self.b.ins().isub(a, b),
            _ => self.b.ins().imul(a, b),
        };
        if !check.arith(self.shared.opts.checked_arith) {
            return r;
        }
        let bad = match op {
            // A sum overflowed in a lane when the result's sign differs from both operands'.
            BinOp::Add => {
                let (x, y) = (self.b.ins().bxor(a, r), self.b.ins().bxor(b, r));
                self.any_sign(x, y)
            }
            // A difference overflowed when the operands' signs differ and the result's differs
            // from the left one's.
            BinOp::Sub => {
                let (x, y) = (self.b.ins().bxor(a, b), self.b.ins().bxor(a, r));
                self.any_sign(x, y)
            }
            _ => self.mul_overflows(a, b),
        };
        self.fail_if(bad, PANIC_OVERFLOW, line, check);
        r
    }

    /// Whether some lane of `x & y` has its sign bit set (a nonzero mask of the lanes' sign bits).
    fn any_sign(&mut self, x: Value, y: Value) -> Value {
        let both = self.b.ins().band(x, y);
        let signs = self.b.ins().vhigh_bits(types::I32, both);
        self.b.ins().icmp_imm_s(IntCC::NotEqual, signs, 0)
    }

    /// Whether some lane of `a * b` doesn't fit `int`: the exact 64-bit products of the low and the
    /// high pair of lanes, each plus 2^31, are 2^32 or more (seen unsigned, so a product below the
    /// lowest `int` counts too) exactly when the product is outside `int`.
    fn mul_overflows(&mut self, a: Value, b: Value) -> Value {
        let half = self.b.ins().iconst(types::I64, 1 << 31);
        let half = self.b.ins().splat(types::I64X2, half);
        let mut high = None;
        for low in [true, false] {
            let (x, y) = if low {
                (self.b.ins().swiden_low(a), self.b.ins().swiden_low(b))
            } else {
                (self.b.ins().swiden_high(a), self.b.ins().swiden_high(b))
            };
            let product = self.b.ins().imul(x, y);
            let shifted = self.b.ins().iadd(product, half);
            let above = self.b.ins().ushr_imm_s(shifted, 32);
            high = Some(match high {
                Some(h) => self.b.ins().bor(h, above),
                None => above,
            });
        }
        self.b.ins().vany_true(high.expect("two pairs"))
    }

    /// `-v` on an `I32X4`: it overflows in a lane holding the lowest `int`.
    pub(super) fn ivec4_neg(&mut self, v: Value, line: u32, check: Check) -> Value {
        let r = self.b.ins().ineg(v);
        if check.arith(self.shared.opts.checked_arith) {
            let min = self.b.ins().iconst(types::I32, i64::from(i32::MIN) & 0xFFFF_FFFF);
            let min = self.b.ins().splat(types::I32X4, min);
            let at_min = self.b.ins().icmp(IntCC::Equal, v, min);
            let bad = self.b.ins().vany_true(at_min);
            self.fail_if(bad, PANIC_OVERFLOW, line, check);
        }
        r
    }

    /// `v << n` or `v >> n` on an `I32X4` by one `int` amount `n`: every lane by the same amount,
    /// `>>` arithmetic as on `int`. With checks on, an amount below 0 or above 31 stops the
    /// program, as for `int`; otherwise the machine masks it.
    pub(super) fn ivec4_shift(&mut self, op: BinOp, v: Value, n: Value, line: u32, check: Check) -> Value {
        if check.arith(self.shared.opts.checked_arith) {
            // Unsigned compare: a negative amount looks huge.
            let bad = self.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, n, 32);
            self.fail_if(bad, PANIC_SHIFT, line, check);
        }
        if op == BinOp::Shl {
            self.b.ins().ishl(v, n)
        } else {
            self.b.ins().sshr(v, n)
        }
    }
}
