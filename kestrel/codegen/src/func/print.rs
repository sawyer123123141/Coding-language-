use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, MemFlags, Value};
use cranelift_module::{DataDescription, Module};
use kestrel_kir::{self as kir, FieldLayout, KTy};

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `print(...)` (`line`: spaces between arguments, then a newline), or
    /// a generated printer's `Write` (neither).
    pub(crate) fn print(&mut self, args: &[kir::PrintArg], line: bool) -> Result<(), String> {
        let pieces = pieces(args, line);
        let vals = self.eval_pieces(&pieces)?;
        self.write_pieces(&pieces, &vals)?;
        if line {
            self.call_id(self.shared.rt.print_newline, &[]);
        }
        Ok(())
    }

    /// An interpolated string `"hp {hp}"` into the string at `dest`. Its
    /// pieces are evaluated first, so output from a call inside one goes to
    /// the program's output and not into the string. Then the print code
    /// writes them into a runtime buffer, which `str_end` copies into the
    /// current arena.
    pub(crate) fn format(&mut self, parts: &[kir::PrintArg], dest: Value) -> Result<(), String> {
        let pieces = pieces(parts, false);
        let vals = self.eval_pieces(&pieces)?;
        let rt = self.shared.rt;
        self.call_id(rt.str_begin, &[]);
        self.write_pieces(&pieces, &vals)?;
        self.call_id(rt.str_end, &[dest]);
        Ok(())
    }

    /// Evaluates every piece before anything is written, so output (or a
    /// panic) inside one can't interleave with the rest. Aggregates are
    /// copied out so a later piece can't change them first. A text piece has
    /// no value.
    pub(crate) fn eval_pieces(&mut self, pieces: &[(&kir::PrintArg, bool)]) -> Result<Vec<Option<Value>>, String> {
        let mut vals = Vec::with_capacity(pieces.len());
        for &(a, _) in pieces {
            vals.push(match a {
                kir::PrintArg::Value(e) if e.ty.is_aggregate() => {
                    let tmp = self.temp_addr(e.ty);
                    self.lower_into(e, tmp)?;
                    Some(tmp)
                }
                kir::PrintArg::Value(e) => Some(self.expr(e)?),
                kir::PrintArg::Str(_) => None,
                kir::PrintArg::Interp(_) => unreachable!("interpolations don't nest"),
            });
        }
        Ok(vals)
    }

    /// Prints pieces already evaluated by `eval_pieces`, each after a space
    /// if its flag is set.
    pub(crate) fn write_pieces(&mut self, pieces: &[(&kir::PrintArg, bool)], vals: &[Option<Value>]) -> Result<(), String> {
        let rt = self.shared.rt;
        for (&(a, space), &val) in pieces.iter().zip(vals) {
            if space {
                self.call_id(rt.print_space, &[]);
            }
            match (a, val) {
                (kir::PrintArg::Value(e), Some(v)) if e.ty.is_aggregate() => self.print_aggregate(v, 0, e.ty)?,
                (kir::PrintArg::Value(e), Some(v)) => self.print_scalar(v, e.ty)?,
                (kir::PrintArg::Str(s), _) => self.print_text(s)?,
                _ => unreachable!("value pieces are evaluated above"),
            }
        }
        Ok(())
    }

    pub(crate) fn print_scalar(&mut self, v: Value, ty: KTy) -> Result<(), String> {
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
            KTy::F32X4 | KTy::I32X4 => return self.print_vector(v, ty),
            KTy::Struct(_)
            | KTy::Enum(_)
            | KTy::Str
            | KTy::Array(_)
            | KTy::List(_)
            | KTy::Map(..)
            | KTy::Arena
            | KTy::Gpu
            | KTy::Optional(_)
            | KTy::Fn
            | KTy::Iface => {
                unreachable!("aggregates are printed from memory")
            }
        };
        Ok(())
    }

    /// Prints the value at `addr + offset`: `Point(x: 1, y: 2)`, `Block.Air`,
    /// `Block.Stone(hardness: 3)`, or a string's text.
    fn print_aggregate(&mut self, addr: Value, offset: i32, ty: KTy) -> Result<(), String> {
        let kir = self.shared.kir;
        match ty {
            KTy::Arena => self.print_text("Arena"),
            // The code's address and environment mean nothing to the reader.
            KTy::Fn => self.print_text("<function>"),
            // The concrete value, through the print function in its table.
            KTy::Iface => {
                let at = self.offset(addr, offset);
                self.print_iface(at);
                Ok(())
            }
            KTy::Gpu => {
                let id = self.b.ins().load(types::I64, MemFlags::trusted(), addr, offset);
                self.call_id(self.shared.rt.print_i64, &[id]);
                Ok(())
            }
            KTy::Optional(el) => {
                let tag = self.b.ins().load(types::I8, MemFlags::trusted(), addr, offset);
                let (some, none, done) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.ins().brif(tag, some, &[], none, &[]);
                self.b.switch_to_block(none);
                self.print_text("none")?;
                self.b.ins().jump(done, &[]);
                self.b.switch_to_block(some);
                let (inner, at) = (kir.elem(el), offset + kir.payload_offset(el) as i32);
                if inner.is_aggregate() {
                    self.print_aggregate(addr, at, inner)?;
                } else {
                    let v = self.load_at(inner, addr, at);
                    self.print_scalar(v, inner)?;
                }
                self.b.ins().jump(done, &[]);
                self.b.switch_to_block(done);
                Ok(())
            }
            KTy::Array(_) | KTy::List(_) | KTy::Map(..) => {
                let at = self.offset(addr, offset);
                let id = self.shared.funcs[kir.print_fns[&ty].0 as usize];
                self.call_id(id, &[at]);
                Ok(())
            }
            KTy::Str => {
                let p = self.b.ins().load(self.ptr, MemFlags::trusted(), addr, offset);
                let n = self.b.ins().load(types::I64, MemFlags::trusted(), addr, offset + 8);
                self.call_id(self.shared.rt.print_str, &[p, n]);
                Ok(())
            }
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
            _ => unreachable!("not an aggregate"),
        }
    }

    fn print_fields(&mut self, addr: Value, offset: i32, fields: &[FieldLayout]) -> Result<(), String> {
        for (i, f) in fields.iter().enumerate() {
            let label = if i == 0 { format!("{}: ", f.name) } else { format!(", {}: ", f.name) };
            self.print_text(&label)?;
            let at = offset + f.offset as i32;
            if f.ty == KTy::Str {
                // Quoted, so `name: "a, b: 1"` can't pass for two fields.
                self.print_text("\"")?;
                self.print_aggregate(addr, at, f.ty)?;
                self.print_text("\"")?;
            } else if f.ty.is_aggregate() {
                self.print_aggregate(addr, at, f.ty)?;
            } else {
                let v = self.load_at(f.ty, addr, at);
                self.print_scalar(v, f.ty)?;
            }
        }
        Ok(())
    }

    pub(crate) fn print_text(&mut self, s: &str) -> Result<(), String> {
        let (p, n) = self.string(s)?;
        self.call_id(self.shared.rt.print_str, &[p, n]);
        Ok(())
    }

    /// Pointer and length of a string literal's bytes.
    pub(crate) fn string(&mut self, s: &str) -> Result<(Value, Value), String> {
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

/// What `print` writes, flattened: each piece with whether a space goes before
/// it. Arguments are separated by spaces (when `line`); an interpolated
/// string's parts aren't.
fn pieces(args: &[kir::PrintArg], line: bool) -> Vec<(&kir::PrintArg, bool)> {
    let mut pieces = Vec::with_capacity(args.len());
    for (i, a) in args.iter().enumerate() {
        let space = line && i > 0;
        match a {
            kir::PrintArg::Interp(parts) => pieces.extend(parts.iter().enumerate().map(|(j, p)| (p, space && j == 0))),
            _ => pieces.push((a, space)),
        }
    }
    pieces
}
