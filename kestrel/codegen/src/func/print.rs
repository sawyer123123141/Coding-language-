use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, MemFlags, Value};
use cranelift_module::{DataDescription, Module};
use kestrel_kir::{self as kir, FieldLayout, KTy};

use super::{cl_type, FuncLowerer};

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    pub(crate) fn print(&mut self, args: &[kir::PrintArg]) -> Result<(), String> {
        // Evaluate every argument before writing anything, so output (or a
        // panic) inside an argument can't interleave with this line. Structs
        // are copied out so a later argument can't change them first.
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(match a {
                kir::PrintArg::Value(e) if e.ty.is_aggregate() => {
                    let tmp = self.temp_addr(e.ty);
                    self.lower_into(e, tmp)?;
                    Some(tmp)
                }
                kir::PrintArg::Value(e) => Some(self.expr(e)?),
                kir::PrintArg::Str(_) => None,
            });
        }
        let rt = self.shared.rt;
        for (i, (a, val)) in args.iter().zip(vals).enumerate() {
            if i > 0 {
                self.call_id(rt.print_space, &[]);
            }
            match a {
                kir::PrintArg::Str(s) => self.print_text(s)?,
                kir::PrintArg::Value(e) => {
                    let v = val.expect("value arguments are evaluated above");
                    if e.ty.is_aggregate() {
                        self.print_aggregate(v, 0, e.ty)?;
                    } else {
                        self.print_scalar(v, e.ty);
                    }
                }
            }
        }
        self.call_id(rt.print_newline, &[]);
        Ok(())
    }

    fn print_scalar(&mut self, v: Value, ty: KTy) {
        let rt = self.shared.rt;
        match ty {
            KTy::Bool => self.call_id(rt.print_bool, &[v]),
            KTy::U8 => {
                let w = self.b.ins().uextend(types::I64, v);
                self.call_id(rt.print_i64, &[w])
            }
            KTy::I32 => {
                let w = self.b.ins().sextend(types::I64, v);
                self.call_id(rt.print_i64, &[w])
            }
            KTy::I64 => self.call_id(rt.print_i64, &[v]),
            KTy::F32 => self.call_id(rt.print_f32, &[v]),
            KTy::F64 => self.call_id(rt.print_f64, &[v]),
            KTy::Struct(_) | KTy::Enum(_) => unreachable!("structs and enums are printed field by field"),
        };
    }

    /// Prints the struct or enum at `addr + offset`: `Point(x: 1, y: 2)`,
    /// `Block.Air`, `Block.Stone(hardness: 3)`.
    fn print_aggregate(&mut self, addr: Value, offset: i32, ty: KTy) -> Result<(), String> {
        let kir = self.shared.kir;
        match ty {
            KTy::Struct(s) => {
                let layout = kir.layout(s);
                self.print_text(&format!("{}(", layout.name))?;
                self.print_fields(addr, offset, &layout.fields)?;
                self.print_text(")")
            }
            KTy::Enum(e) => {
                let layout = kir.enum_layout(e);
                let tag = self.b.ins().load(types::I8, MemFlags::trusted(), addr, offset);
                let done = self.b.create_block();
                for (vi, variant) in layout.variants.iter().enumerate() {
                    let (yes, next) = (self.b.create_block(), self.b.create_block());
                    let is = self.b.ins().icmp_imm(IntCC::Equal, tag, vi as i64);
                    self.b.ins().brif(is, yes, &[], next, &[]);
                    self.b.switch_to_block(yes);
                    let name = format!("{}.{}", layout.name, variant.name);
                    if variant.fields.is_empty() {
                        self.print_text(&name)?;
                    } else {
                        self.print_text(&format!("{name}("))?;
                        self.print_fields(addr, offset, &variant.fields)?;
                        self.print_text(")")?;
                    }
                    self.b.ins().jump(done, &[]);
                    self.b.switch_to_block(next);
                }
                self.b.ins().jump(done, &[]);
                self.b.switch_to_block(done);
                Ok(())
            }
            _ => unreachable!("not a struct or enum"),
        }
    }

    fn print_fields(&mut self, addr: Value, offset: i32, fields: &[FieldLayout]) -> Result<(), String> {
        for (i, f) in fields.iter().enumerate() {
            let label = if i == 0 { format!("{}: ", f.name) } else { format!(", {}: ", f.name) };
            self.print_text(&label)?;
            let at = offset + f.offset as i32;
            if f.ty.is_aggregate() {
                self.print_aggregate(addr, at, f.ty)?;
            } else {
                let v = self.b.ins().load(cl_type(f.ty), MemFlags::trusted(), addr, at);
                self.print_scalar(v, f.ty);
            }
        }
        Ok(())
    }

    fn print_text(&mut self, s: &str) -> Result<(), String> {
        let (p, n) = self.string(s)?;
        self.call_id(self.shared.rt.print_str, &[p, n]);
        Ok(())
    }

    /// Pointer and length of a string literal's bytes.
    fn string(&mut self, s: &str) -> Result<(Value, Value), String> {
        let id = match self.strings.ids.get(s) {
            Some(&id) => id,
            None => {
                let id = self.module.declare_anonymous_data(false, false).map_err(|e| e.to_string())?;
                let mut desc = DataDescription::new();
                // Zero-sized data objects aren't allowed; the length stays 0.
                let bytes = if s.is_empty() { vec![0] } else { s.as_bytes().to_vec() };
                desc.define(bytes.into_boxed_slice());
                self.module.define_data(id, &desc).map_err(|e| e.to_string())?;
                self.strings.ids.insert(s.to_string(), id);
                id
            }
        };
        let gv = self.module.declare_data_in_func(id, self.b.func);
        let p = self.b.ins().symbol_value(self.ptr, gv);
        let n = self.b.ins().iconst(types::I64, s.len() as i64);
        Ok((p, n))
    }
}
