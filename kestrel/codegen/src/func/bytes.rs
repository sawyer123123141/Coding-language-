//! The list operations behind `to_bytes` (`kir::lower::bytes`): giving its `list[u8]` exactly the
//! room it needs (`ListOp::Reserve`), and writing a value's bytes into it at a byte index
//! (`ListOp::WriteBytes`, `ListOp::WriteCount`). The writes are unaligned little-endian stores into
//! the list's buffer, never checked: the list's length already covers them. And the reads behind
//! `from_bytes` (`kir::lower::bytes_read`, `ExprKind::ReadBytes`): unaligned little-endian loads,
//! never checked either (the read walk checks the length first), and a string made from bytes.

use cranelift_codegen::ir::{types, Endianness, InstBuilder, MemFlagsData, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, KTy, ListOp, Place};

use super::{cl_type, FuncLowerer};

/// The flags of a load from or store into a byte buffer: any alignment, little-endian.
fn byte_flags() -> MemFlagsData {
    MemFlagsData::new().with_notrap().with_endianness(Endianness::Little)
}

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `Reserve`, `WriteBytes` or `WriteCount` on the list at `list`. The arguments are evaluated
    /// first, then the place is found (as for every list operation; see `list_op`).
    pub(crate) fn bytes_op(&mut self, op: ListOp, list: &Place, args: &[kir::Expr]) -> Result<(), String> {
        match op {
            ListOp::Reserve => {
                let ty = self.place_ty(list);
                let (size, align) = self.elem_size_align(ty);
                let n = self.expr(&args[0])?;
                let header = self.place_addr(list)?;
                let (s, a) = (self.b.ins().iconst(types::I64, size), self.b.ins().iconst(types::I64, align));
                self.call_id(self.shared.rt.list_reserve, &[header, n, s, a]);
            }
            ListOp::WriteBytes => {
                let at = self.expr(&args[0])?;
                let value = &args[1];
                if value.ty == KTy::Str {
                    let (ptr, len) = self.str_parts(value)?;
                    let dst = self.byte_at(list, at)?;
                    self.call_id(self.shared.rt.bytes_write_str, &[dst, ptr, len]);
                } else {
                    let v = self.expr(value)?;
                    let dst = self.byte_at(list, at)?;
                    self.b.ins().store(byte_flags(), v, dst, 0);
                }
            }
            ListOp::WriteCount => {
                let at = self.expr(&args[0])?;
                let n = self.expr(&args[1])?;
                let n = self.b.ins().ireduce(types::I32, n);
                let dst = self.byte_at(list, at)?;
                self.b.ins().store(byte_flags(), n, dst, 0);
            }
            other => unreachable!("{other:?} isn't a `to_bytes` operation"),
        }
        Ok(())
    }

    /// `ReadBytes` of a number or vector (`ty`) from the `list[u8]` at `list`. The index is
    /// evaluated first, then the place is found.
    pub(crate) fn read_bytes(&mut self, list: &Place, args: &[kir::Expr], ty: KTy) -> Result<Value, String> {
        let at = self.expr(&args[0])?;
        let src = self.byte_at(list, at)?;
        Ok(self.b.ins().load(cl_type(ty), byte_flags(), src, 0))
    }

    /// `ReadBytes` of a string from the `list[u8]` at `list`, written at `dest`: the empty string
    /// when the bytes aren't UTF-8.
    pub(crate) fn read_str_into(&mut self, list: &Place, args: &[kir::Expr], dest: Value) -> Result<(), String> {
        let at = self.expr(&args[0])?;
        let n = self.expr(&args[1])?;
        let src = self.byte_at(list, at)?;
        self.call_id(self.shared.rt.bytes_read_str, &[src, n, dest]);
        Ok(())
    }

    /// The address of byte `at` (an `i64`) of the `list[u8]` at `list`.
    fn byte_at(&mut self, list: &Place, at: Value) -> Result<Value, String> {
        let header = self.place_addr(list)?;
        let items = self.b.ins().load(self.ptr, MemFlagsData::trusted(), header, 0);
        Ok(self.b.ins().iadd(items, at))
    }
}
