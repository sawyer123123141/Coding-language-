//! String operations (`ExprKind::Str`): the runtime (`runtime/src/string.rs`) counts and cuts
//! characters; a string is a pointer and a byte length, like an array of bytes.

use cranelift_codegen::ir::{types, InstBuilder, MemFlags, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, StrOp};

use super::collections::LEN;
use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// The pointer and byte length of the string value `e`, read now (before any later operand
    /// is evaluated, which could change the place it came from).
    fn str_parts(&mut self, e: &kir::Expr) -> Result<(Value, Value), String> {
        let addr = self.materialize(e)?;
        let ptr = self.b.ins().load(self.ptr, MemFlags::trusted(), addr, 0);
        let len = self.b.ins().load(types::I64, MemFlags::trusted(), addr, LEN);
        Ok((ptr, len))
    }

    /// An integer operand as an i64.
    fn str_int(&mut self, e: &kir::Expr) -> Result<Value, String> {
        let v = self.expr(e)?;
        Ok(self.widen_i64(v, e.ty))
    }

    /// A string operation with a number as its result.
    pub(crate) fn str_scalar(&mut self, op: StrOp, args: &[kir::Expr]) -> Result<Value, String> {
        let rt = self.shared.rt;
        let (ptr, len) = self.str_parts(&args[0])?;
        Ok(match op {
            StrOp::Chars => self.call_id(rt.str_chars, &[ptr, len]),
            StrOp::CharLen => {
                let pos = self.str_int(&args[1])?;
                self.call_id(rt.str_char_len, &[ptr, len, pos])
            }
            StrOp::At | StrOp::Piece => unreachable!("{op:?} builds a string in memory"),
        }
        .expect("returns a number"))
    }

    /// A string operation with a new string as its result, written at `dest`.
    pub(crate) fn str_into(&mut self, op: StrOp, args: &[kir::Expr], line: u32, dest: Value) -> Result<(), String> {
        let rt = self.shared.rt;
        let (ptr, len) = self.str_parts(&args[0])?;
        match op {
            StrOp::At => {
                let i = self.str_int(&args[1])?;
                let (l, f) = self.loc_args(line);
                self.call_id(rt.str_at, &[ptr, len, i, l, f, dest]);
            }
            StrOp::Piece => {
                let pos = self.str_int(&args[1])?;
                let n = self.str_int(&args[2])?;
                self.call_id(rt.str_piece, &[ptr, len, pos, n, dest]);
            }
            StrOp::Chars | StrOp::CharLen => unreachable!("{op:?} is a number"),
        }
        Ok(())
    }
}
