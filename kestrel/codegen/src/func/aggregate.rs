use cranelift_codegen::ir::{types, InstBuilder, MemFlags, TrapCode, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, EnvKind, ExprKind, KTy};

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// Writes the struct value of `e` into memory at `dest`.
    pub(crate) fn lower_into(&mut self, e: &kir::Expr, dest: Value) -> Result<(), String> {
        match &e.kind {
            ExprKind::Load(p) => {
                let src = self.place_addr(p)?;
                self.copy_aggregate(dest, src, e.ty);
            }
            ExprKind::StructLit(s, fields) => {
                let kir = self.shared.kir;
                for (f, layout) in fields.iter().zip(&kir.layout(*s).fields) {
                    self.store_at(f, dest, layout.offset as i32)?;
                }
            }
            ExprKind::VariantLit(en, v, fields) => {
                let kir = self.shared.kir;
                let tag = self.b.ins().iconst(types::I8, *v as i64);
                self.b.ins().store(MemFlags::trusted(), tag, dest, 0);
                for (f, layout) in fields.iter().zip(&kir.enum_layout(*en).variants[*v as usize].fields) {
                    self.store_at(f, dest, layout.offset as i32)?;
                }
            }
            ExprKind::ArrayNew(len) => self.array_new(len, e.ty, e.line, dest)?,
            ExprKind::ArrayLit(items) => self.array_lit(items, e.ty, e.line, dest)?,
            ExprKind::ListNew => self.list_new(dest),
            ExprKind::MapNew => self.map_new(dest),
            ExprKind::Map(kir::MapOp::Get, p, args) => self.map_get_into(p, &args[0], e.ty, dest)?,
            ExprKind::Str(op, args) => self.str_into(*op, args, e.ty, e.line, dest)?,
            ExprKind::Move(p) => self.move_into(p, e.ty, dest)?,
            ExprKind::Some(v) => {
                let KTy::Optional(el) = e.ty else { unreachable!("`Some` builds an optional") };
                let one = self.b.ins().iconst(types::I8, 1);
                self.b.ins().store(MemFlags::trusted(), one, dest, 0);
                let at = self.shared.kir.payload_offset(el) as i32;
                self.store_at(v, dest, at)?;
            }
            ExprKind::NoneLit => {
                let zero = self.b.ins().iconst(types::I8, 0);
                self.b.ins().store(MemFlags::trusted(), zero, dest, 0);
            }
            ExprKind::NewArena => {
                let a = self.new_arena();
                self.b.ins().store(MemFlags::trusted(), a, dest, 0);
            }
            ExprKind::StaticArena => {
                let a = self.call_id(self.shared.rt.arena_new_static, &[]).expect("returns an arena");
                self.b.ins().store(MemFlags::trusted(), a, dest, 0);
            }
            ExprKind::StrLit(s) => {
                let (p, n) = self.string(s)?;
                self.b.ins().store(MemFlags::trusted(), p, dest, 0);
                self.b.ins().store(MemFlags::trusted(), n, dest, 8);
            }
            ExprKind::Format(parts) => self.format(parts, dest)?,
            ExprKind::FieldOf(base, i) => {
                let src = self.field_addr(base, *i)?;
                self.copy_aggregate(dest, src, e.ty);
            }
            ExprKind::Call(fi, args) => {
                self.call(*fi, args, Some(dest))?;
            }
            ExprKind::CallIndirect { callee, args, sig } => {
                self.call_indirect(callee, args, sig, Some(dest))?;
            }
            ExprKind::CallVirtual { recv, slot, args, sig, .. } => {
                self.call_virtual(recv, *slot, args, sig, Some(dest))?;
            }
            ExprKind::MakeIface { table, data } => self.make_iface_into(*table, data, e.line, dest)?,
            ExprKind::FnRef(thunk) => self.fn_ref_into(*thunk, dest),
            // A closure with nothing to capture: its code and a zero environment, like a named function.
            ExprKind::MakeFn { code, env: EnvKind::None } => self.fn_ref_into(*code, dest),
            ExprKind::MakeFn { code, env: EnvKind::Heap(env) } => self.heap_fn_into(*code, env, e.line, dest)?,
            ExprKind::MakeFn { code, env: EnvKind::Stack(env) } => self.stack_fn_into(*code, env, dest)?,
            ExprKind::Seq(stmts, value) => {
                self.stmts(stmts)?;
                self.lower_into(value, dest)?;
            }
            ExprKind::If { cond, then_body, then_val, else_body, else_val } => {
                let c = self.expr(cond)?;
                let (then_b, else_b, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.ins().brif(c, then_b, &[], else_b, &[]);
                for (block, body, val) in [(then_b, then_body, then_val), (else_b, else_body, else_val)] {
                    self.b.switch_to_block(block);
                    if self.stmts(body)? {
                        match val {
                            Some(v) => {
                                self.lower_into(v, dest)?;
                                self.b.ins().jump(merge, &[]);
                            }
                            None => {
                                self.b.ins().trap(TrapCode::unwrap_user(1));
                            }
                        }
                    }
                }
                self.b.switch_to_block(merge);
            }
            other => unreachable!("{other:?} is not a struct value"),
        }
        Ok(())
    }

    /// Stores `e` (a scalar or a struct) at `base + offset`.
    pub(crate) fn store_at(&mut self, e: &kir::Expr, base: Value, offset: i32) -> Result<(), String> {
        if e.ty.is_aggregate() {
            let dest = if offset == 0 { base } else { self.b.ins().iadd_imm(base, offset as i64) };
            self.lower_into(e, dest)
        } else {
            let v = self.expr(e)?;
            self.b.ins().store(MemFlags::trusted(), v, base, offset);
            Ok(())
        }
    }

    /// The address of a struct value: a place's own memory, or a temporary
    /// it is built in. Only for reading.
    pub(crate) fn materialize(&mut self, e: &kir::Expr) -> Result<Value, String> {
        if let ExprKind::Load(p) = &e.kind {
            return self.place_addr(p);
        }
        let tmp = self.temp_addr(e.ty);
        self.lower_into(e, tmp)?;
        Ok(tmp)
    }

    /// The address of field `i` of the struct value `base`.
    pub(crate) fn field_addr(&mut self, base: &kir::Expr, i: u32) -> Result<Value, String> {
        let KTy::Struct(s) = base.ty else { unreachable!("field of a non-struct") };
        let offset = self.shared.kir.layout(s).fields[i as usize].offset;
        let addr = self.materialize(base)?;
        Ok(if offset == 0 { addr } else { self.b.ins().iadd_imm(addr, offset as i64) })
    }
}
