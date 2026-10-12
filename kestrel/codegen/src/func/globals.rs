//! Statements that touch module-level data: reference locals pointed at globals
//! (`Stmt::BindGlobal`, for heap constants) and the runtime's call line and file
//! (`Stmt::SetLine`).

use cranelift_codegen::ir::{types, InstBuilder, MemFlags, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, GlobalIdx, LocalIdx};

use super::{FuncLowerer, Slot};

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// A packed KIR line (see `kir::pack_line`) as runtime errors take it: the line, and a pointer
    /// to the file's `FileName` record (null when there is no name).
    pub(crate) fn loc_args(&mut self, loc: u32) -> (Value, Value) {
        let (line, file) = kir::unpack_line(loc);
        let line = self.b.ins().iconst(types::I32, i64::from(line));
        let record = (file != kir::NO_FILE).then(|| self.shared.files.get(file as usize).copied()).flatten();
        let file = match record {
            Some(id) => {
                let gv = self.module.declare_data_in_func(id, self.b.func);
                self.b.ins().symbol_value(self.ptr, gv)
            }
            None => self.b.ins().iconst(self.ptr, 0),
        };
        (line, file)
    }

    /// Stores the line and file in the runtime's `kestrel_rt_call_line` and `kestrel_rt_call_file`
    /// (see `Stmt::SetLine`).
    pub(crate) fn set_line(&mut self, loc: u32) {
        let (line, file) = self.loc_args(loc);
        for (data, value) in [(self.shared.rt.call_line, line), (self.shared.rt.call_file, file)] {
            let gv = self.module.declare_data_in_func(data, self.b.func);
            let addr = self.b.ins().symbol_value(self.ptr, gv);
            self.b.ins().store(MemFlags::trusted(), value, addr, 0);
        }
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
