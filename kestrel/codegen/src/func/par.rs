//! `par for`: hands the outlined body's thunk, the captured variables'
//! addresses and the iteration count to the runtime's thread pool.

use cranelift_codegen::ir::{InstBuilder, StackSlotData, StackSlotKind};
use cranelift_module::Module;
use kestrel_kir::{self as kir, FuncIdx};

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    pub(crate) fn par_for(&mut self, func: FuncIdx, args: &[kir::Arg], count: &kir::Expr) -> Result<(), String> {
        let word = self.ptr.bytes();
        let data = StackSlotData::new(StackSlotKind::ExplicitSlot, (args.len() as u32 * word).max(word), word.trailing_zeros() as u8);
        let env = self.b.create_sized_stack_slot(data);
        for (k, a) in args.iter().enumerate() {
            let kir::Arg::Ref(p) = a else { unreachable!("par for passes captures by pointer") };
            let addr = self.place_addr(p)?;
            self.b.ins().stack_store(addr, env, (k as u32 * word) as i32);
        }
        let n = self.expr(count)?;
        let thunk = self.shared.par_thunks[&func.0];
        let fref = self.module.declare_func_in_func(thunk, self.b.func);
        let body = self.b.ins().func_addr(self.ptr, fref);
        let env = self.b.ins().stack_addr(self.ptr, env, 0);
        self.call_id(self.shared.rt.par_for, &[n, body, env]);
        Ok(())
    }
}
