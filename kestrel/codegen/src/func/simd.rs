//! SIMD vectors (`vec4`): one Cranelift `F32X4` value, in a register like a number. Lanes are
//! `extractlane`/`insertlane` on a register and a 4-byte access at `4 * lane` in memory; a swizzle
//! is a byte `shuffle` of the value seen as `I8X16`; `==` compares every lane and asks whether all
//! are equal.
//!
//! **Memory access.** Every load and store whose type may be a vector goes through `mem_flags`
//! (or `value_flags`): a vector access carries no `aligned` flag, so a layout that misses the
//! 16-byte alignment is slower, never a fault (Cranelift folds only flagged addresses into the
//! non-VEX SSE instructions that fault when misaligned). A stack slot is sized and aligned by
//! `size_align` and keeps Cranelift's own flags.

use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{types, ConstantData, Endianness, InstBuilder, MemFlags, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, BinOp, ExprKind, KTy};

use super::{cl_type, FuncLowerer};

/// The lane names, in order, as a vector prints them.
const LANES: [&str; 4] = ["x", "y", "z", "w"];

/// The flags of a load or store of a value of type `ty`: without `aligned` for a vector,
/// `MemFlags::trusted()` for anything else.
pub(crate) fn mem_flags(ty: KTy) -> MemFlags {
    if ty.is_vector() {
        MemFlags::new().with_notrap()
    } else {
        MemFlags::trusted()
    }
}

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// The flags of a store of `v`: by its Cranelift type, as `mem_flags` gives them.
    pub(crate) fn value_flags(&self, v: Value) -> MemFlags {
        if self.b.func.dfg.value_type(v).is_vector() {
            MemFlags::new().with_notrap()
        } else {
            MemFlags::trusted()
        }
    }

    /// Loads a number or a vector of type `ty` at `addr + offset`.
    pub(crate) fn load_at(&mut self, ty: KTy, addr: Value, offset: i32) -> Value {
        self.b.ins().load(cl_type(ty), mem_flags(ty), addr, offset)
    }

    /// Stores the number or vector `v` at `addr + offset`.
    pub(crate) fn store_value(&mut self, v: Value, addr: Value, offset: i32) {
        let flags = self.value_flags(v);
        self.b.ins().store(flags, v, addr, offset);
    }

    /// A vector made (`VecLit`, `Splat`), shuffled (`Shuffle`) or taken apart (`LaneOf`).
    pub(crate) fn simd_expr(&mut self, e: &kir::Expr) -> Result<Value, String> {
        let ty = cl_type(e.ty);
        Ok(match &e.kind {
            ExprKind::VecLit(lanes) => {
                let vals = lanes.iter().map(|l| self.expr(l)).collect::<Result<Vec<_>, _>>()?;
                let mut v = self.b.ins().splat(ty, vals[0]);
                for (i, &x) in vals.iter().enumerate().skip(1) {
                    v = self.b.ins().insertlane(v, x, i as u8);
                }
                v
            }
            ExprKind::Splat(x) => {
                let x = self.expr(x)?;
                self.b.ins().splat(ty, x)
            }
            ExprKind::Shuffle(x, lanes) => {
                let v = self.expr(x)?;
                self.shuffle(v, *lanes)
            }
            ExprKind::LaneOf(x, lane) => {
                let v = self.expr(x)?;
                self.b.ins().extractlane(v, *lane)
            }
            _ => unreachable!("not a vector form"),
        })
    }

    /// The vector whose lane `i` is lane `lanes[i]` of `v`: a byte shuffle of `v` seen as 16
    /// bytes (little-endian, so lane `l` is bytes `4 * l` to `4 * l + 3`).
    fn shuffle(&mut self, v: Value, lanes: [u8; 4]) -> Value {
        let ty = self.b.func.dfg.value_type(v);
        let little = MemFlags::new().with_endianness(Endianness::Little);
        let bytes = self.b.ins().bitcast(types::I8X16, little, v);
        let mask: Vec<u8> = lanes.iter().flat_map(|&l| (0..4).map(move |k| 4 * l + k)).collect();
        let mask = self.b.func.dfg.immediates.push(ConstantData::from(mask));
        let shuffled = self.b.ins().shuffle(bytes, bytes, mask);
        self.b.ins().bitcast(ty, little, shuffled)
    }

    /// `a op b` on two vectors of type `t`: lane by lane, or (`==`, `!=`) one `bool` for all the
    /// lanes: equal when every lane is (a NaN lane is unequal), different when some lane is.
    pub(crate) fn simd_binary(&mut self, op: BinOp, a: Value, b: Value, t: KTy) -> Value {
        match (t, op) {
            (KTy::F32X4, BinOp::Add) => self.b.ins().fadd(a, b),
            (KTy::F32X4, BinOp::Sub) => self.b.ins().fsub(a, b),
            (KTy::F32X4, BinOp::Mul) => self.b.ins().fmul(a, b),
            (KTy::F32X4, BinOp::Div) => self.b.ins().fdiv(a, b),
            (KTy::F32X4, BinOp::Eq | BinOp::Ne) => {
                let same = self.b.ins().fcmp(FloatCC::Equal, a, b);
                let all = self.b.ins().vall_true(same);
                if op == BinOp::Ne {
                    self.b.ins().bxor_imm(all, 1)
                } else {
                    all
                }
            }
            _ => unreachable!("the checker allows no `{op:?}` on {t:?}"),
        }
    }

    /// Prints the vector `v` of type `ty` as `vec4(x: 1, y: 2, z: 3, w: 4)`, each lane as its
    /// lane type prints.
    pub(crate) fn print_vector(&mut self, v: Value, ty: KTy) -> Result<(), String> {
        let name = if ty == KTy::I32X4 { "ivec4" } else { "vec4" };
        for (i, lane) in LANES.iter().enumerate() {
            let label = if i == 0 { format!("{name}(x: ") } else { format!(", {lane}: ") };
            self.print_text(&label)?;
            let x = self.b.ins().extractlane(v, i as u8);
            self.print_scalar(x, ty.lane())?;
        }
        self.print_text(")")
    }
}
