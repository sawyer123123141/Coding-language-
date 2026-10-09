use cranelift_codegen::ir::{InstBuilder, MemFlags, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, KTy, Place};

use super::{cl_type, FuncLowerer, Slot};

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// The type of the value stored at a place, and its byte offset from the root.
    fn place_ty_offset(&self, p: &Place) -> (KTy, i32) {
        let kir = self.shared.kir;
        let mut ty = self.f.locals[p.root.0 as usize].ty;
        let mut offset = 0;
        for &fi in &p.fields {
            let KTy::Struct(s) = ty else { unreachable!("field path through a non-struct") };
            let field = &kir.layout(s).fields[fi as usize];
            offset += field.offset as i32;
            ty = field.ty;
        }
        (ty, offset)
    }

    /// Address of a place. Locals kept in registers have none.
    pub(crate) fn place_addr(&mut self, p: &Place) -> Value {
        let base = match self.slots[p.root.0 as usize] {
            Slot::Stack(ss) => self.b.ins().stack_addr(self.ptr, ss, 0),
            Slot::Ptr(v) => self.b.use_var(v),
            Slot::Var(_) => unreachable!("KIR puts every addressed local in memory"),
        };
        let (_, offset) = self.place_ty_offset(p);
        if offset == 0 {
            base
        } else {
            self.b.ins().iadd_imm(base, offset as i64)
        }
    }

    /// Loads the scalar stored at a place.
    pub(crate) fn load_place(&mut self, p: &Place) -> Value {
        if let (Slot::Var(v), true) = (self.slots[p.root.0 as usize], p.fields.is_empty()) {
            return self.b.use_var(v);
        }
        let (ty, _) = self.place_ty_offset(p);
        let addr = self.place_addr(p);
        self.b.ins().load(cl_type(ty), MemFlags::trusted(), addr, 0)
    }

    pub(crate) fn store_place(&mut self, p: &Place, val: Value) {
        if let (Slot::Var(v), true) = (self.slots[p.root.0 as usize], p.fields.is_empty()) {
            self.b.def_var(v, val);
            return;
        }
        let addr = self.place_addr(p);
        self.b.ins().store(MemFlags::trusted(), val, addr, 0);
    }

    /// Copies a struct of type `ty` field by field from `src` to `dst`.
    pub(crate) fn copy_aggregate(&mut self, dst: Value, src: Value, ty: KTy) {
        self.copy_fields(dst, src, 0, ty);
    }

    fn copy_fields(&mut self, dst: Value, src: Value, offset: i32, ty: KTy) {
        let kir = self.shared.kir;
        let KTy::Struct(s) = ty else { unreachable!("copying a non-struct") };
        for f in &kir.layout(s).fields {
            let at = offset + f.offset as i32;
            if f.ty.is_aggregate() {
                self.copy_fields(dst, src, at, f.ty);
            } else {
                let v = self.b.ins().load(cl_type(f.ty), MemFlags::trusted(), src, at);
                self.b.ins().store(MemFlags::trusted(), v, dst, at);
            }
        }
    }

    pub(crate) fn assign(&mut self, p: &Place, e: &kir::Expr) -> Result<(), String> {
        if !e.ty.is_aggregate() {
            let v = self.expr(e)?;
            self.store_place(p, v);
            return Ok(());
        }
        let src = match &e.kind {
            // Two places of one struct type are the same memory or don't overlap.
            kir::ExprKind::Load(from) => self.place_addr(from),
            // Built in a temporary first: the new value may read the place it replaces.
            _ => {
                let tmp = self.temp_addr(e.ty);
                self.lower_into(e, tmp)?;
                tmp
            }
        };
        let dst = self.place_addr(p);
        self.copy_aggregate(dst, src, e.ty);
        Ok(())
    }
}
