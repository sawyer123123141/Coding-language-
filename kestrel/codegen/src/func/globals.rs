//! Reference locals pointed at globals (`Stmt::BindGlobal`), for heap constants.

use cranelift_codegen::ir::InstBuilder;
use cranelift_module::Module;
use kestrel_kir::{GlobalIdx, LocalIdx};

use super::{FuncLowerer, Slot};

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// Points the reference local `l` at global `g`, like `Stmt::Bind` points one at a place.
    pub(crate) fn bind_global(&mut self, l: LocalIdx, g: GlobalIdx) {
        let id = self.shared.globals[g.0 as usize];
        let gv = self.module.declare_data_in_func(id, self.b.func);
        let addr = self.b.ins().symbol_value(self.ptr, gv);
        let Slot::Ptr(v) = self.slots[l.0 as usize] else { unreachable!("binds a reference local") };
        self.b.def_var(v, addr);
    }
}
