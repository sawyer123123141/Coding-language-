//! Statements that touch module-level data: reference locals pointed at globals
//! (`Stmt::BindGlobal`, for heap constants) and the runtime's call line (`Stmt::SetLine`).

use cranelift_codegen::ir::{types, InstBuilder, MemFlags};
use cranelift_module::Module;
use kestrel_kir::{GlobalIdx, LocalIdx};

use super::{FuncLowerer, Slot};

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// Stores `line` in the runtime's `kestrel_rt_call_line` (see `Stmt::SetLine`).
    pub(crate) fn set_line(&mut self, line: u32) {
        let gv = self.module.declare_data_in_func(self.shared.rt.call_line, self.b.func);
        let addr = self.b.ins().symbol_value(self.ptr, gv);
        let line = self.b.ins().iconst(types::I32, i64::from(line));
        self.b.ins().store(MemFlags::trusted(), line, addr, 0);
    }

    /// Points the reference local `l` at global `g`, like `Stmt::Bind` points one at a place.
    pub(crate) fn bind_global(&mut self, l: LocalIdx, g: GlobalIdx) {
        let id = self.shared.globals[g.0 as usize];
        let gv = self.module.declare_data_in_func(id, self.b.func);
        let addr = self.b.ins().symbol_value(self.ptr, gv);
        let Slot::Ptr(v) = self.slots[l.0 as usize] else { unreachable!("binds a reference local") };
        self.b.def_var(v, addr);
    }
}
