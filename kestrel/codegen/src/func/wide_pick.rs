//! The picks of the inline `float` math: `if c { a } else { b }` for `f64` or `f32` values, with no
//! jump. Never a float `select`, which on x64 is a jump (`XmmCmove`).
//!
//! A pick on a float compare of the values being computed takes the compare's mask (`fmask`): a
//! vector `fcmp` of the two values (`cmpltpd` and the like) and a `bitselect` of lane 0, all in the
//! float registers. Cranelift makes the `bitselect` a `pblendvb`, or `minpd`/`maxpd` when the pick
//! is a min or a max of the compared values, or `andpd`/`andnpd`/`orpd` for a mask made by mask
//! logic. An integer `select` there would move both values to general registers and the result
//! back, on the critical path.
//!
//! A pick on an integer condition (an `i8`: a bit test, `tiny`, a sign) is an integer `select` on
//! the bits. A mask moved to the float registers instead (`bmask`, `scalar_to_vector`,
//! `bitselect`) measured 0 to 20% slower at every such site (Task 8 of the fast float math round):
//! these loops are short of float ports, not of integer ones.

use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{types, Endianness, InstBuilder, InstructionData, MemFlagsData, Opcode, Type, Value, ValueDef};
use cranelift_module::Module;

use super::FuncLowerer;

/// The vector bitcasts here keep lane 0 in the low bytes.
fn little() -> MemFlagsData {
    MemFlagsData::new().with_endianness(Endianness::Little)
}

/// The vector type whose lanes are `ty`'s (`F64X2` for `f64`, `F32X4` for `f32`).
fn vector_of(ty: Type) -> Type {
    ty.by(128 / ty.bits()).expect("a float type fills a 128-bit vector")
}

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// The mask of the float compare `x <cc> y` for the `f64`s or `f32`s `x` and `y`: a vector of
    /// their type whose lane 0 is all ones where the compare holds and zeros where it doesn't. Like
    /// the scalar `fcmp`, an ordered compare (`<`, `>`, `<=`, `>=`, `==`) is false when either value
    /// is NaN, and `NotEqual` is true. `x > y` is made `y < x`, the form of Cranelift's
    /// `minpd`/`maxpd` patterns.
    pub(super) fn fmask(&mut self, cc: FloatCC, x: Value, y: Value) -> Value {
        let (cc, x, y) = match cc {
            FloatCC::GreaterThan => (FloatCC::LessThan, y, x),
            FloatCC::GreaterThanOrEqual => (FloatCC::LessThanOrEqual, y, x),
            _ => (cc, x, y),
        };
        let (xv, yv) = (self.lanes(x), self.lanes(y));
        let m = self.b.ins().fcmp(cc, xv, yv);
        let ty = self.b.func.dfg.value_type(xv);
        self.b.ins().bitcast(ty, little(), m)
    }

    /// `if c { a } else { b }` for `f64` or `f32` values. `c` is a mask from `fmask` (or `band`,
    /// `bxor` and `band_not` of such masks), picked in the float registers, or an `i8`, picked by an
    /// integer `select` on the bits.
    pub(super) fn pick(&mut self, c: Value, a: Value, b: Value) -> Value {
        if self.b.func.dfg.value_type(c).is_vector() {
            return self.lane_pick(c, a, b);
        }
        // A constant condition (`atan` and `asin` pass `x_negative` as `0`) picks at once: Cranelift
        // would fold the `select` but keep its value's two bitcasts, a trip through a general register.
        if let Some(inst) = self.b.func.dfg.value_def(c).inst() {
            if let InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } = self.b.func.dfg.insts[inst] {
                return if imm.bits() != 0 { a } else { b };
            }
        }
        let ty = self.b.func.dfg.value_type(a);
        let int = if ty == types::F64 { types::I64 } else { types::I32 };
        let (ai, bi) = (self.bits(int, a), self.bits(int, b));
        let v = self.b.ins().select(c, ai, bi);
        self.float_of(ty, v)
    }

    /// Lane 0 of `bitselect(mask, a, b)`, the mask reshaped to `a`'s lanes: an `f64` mask's lane 0
    /// holds an `f32` mask's lane 0 (`%` compares in `f64` and picks its `f32` result), never the
    /// other way.
    fn lane_pick(&mut self, mask: Value, a: Value, b: Value) -> Value {
        let ty = vector_of(self.b.func.dfg.value_type(a));
        let mt = self.b.func.dfg.value_type(mask);
        assert!(mt.lane_bits() >= ty.lane_bits(), "an f32 mask can't pick f64 values");
        let mask = if mt == ty { mask } else { self.b.ins().bitcast(ty, little(), mask) };
        let (av, bv) = (self.lanes(a), self.lanes(b));
        let v = self.b.ins().bitselect(mask, av, bv);
        self.b.ins().extractlane(v, 0)
    }

    /// A vector of `v`'s type with `v` in lane 0: the vector `v` was taken from when it is lane 0
    /// of one (only lane 0 is ever read), else `v` in every lane (`pshufd`).
    fn lanes(&mut self, v: Value) -> Value {
        let dfg = &self.b.func.dfg;
        if let ValueDef::Result(inst, 0) = dfg.value_def(v) {
            if let InstructionData::BinaryImm8 { opcode: Opcode::Extractlane, arg, imm: 0 } = dfg.insts[inst] {
                return arg;
            }
        }
        let ty = vector_of(dfg.value_type(v));
        self.b.ins().splat(ty, v)
    }
}
