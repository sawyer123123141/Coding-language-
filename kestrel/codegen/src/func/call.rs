use cranelift_codegen::ir::{InstBuilder, Value};
use cranelift_module::{FuncId, Module};
use kestrel_kir as kir;

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// Calls a Kestrel function. A struct result is written to `dest`, or to
    /// a temporary when the caller doesn't need it. Struct arguments are
    /// passed by pointer.
    pub(crate) fn call(&mut self, fi: kir::FuncIdx, args: &[kir::Arg], dest: Option<Value>) -> Result<Option<Value>, String> {
        let ret = self.shared.kir.funcs[fi.0 as usize].ret;
        let mut vals = Vec::with_capacity(args.len() + 1);
        if let Some(r) = ret.filter(|r| r.is_aggregate()) {
            let d = match dest {
                Some(d) => d,
                None => self.temp_addr(r),
            };
            vals.push(d);
        }
        vals.extend(self.call_args(args)?);
        let id = self.shared.funcs[fi.0 as usize];
        let result = self.call_id(id, &vals);
        // Only built-in modules declare natives. The window library's graphics driver may change
        // the float mode on this thread: put it back (cheap, so after the runtime's `file` natives too).
        if self.shared.kir.funcs[fi.0 as usize].native.is_some() {
            self.call_id(self.shared.rt.fpu_pin, &[]);
        }
        Ok(result)
    }

    /// The values a call passes for `args`, in order: a pointer for a struct or a `var` argument.
    pub(crate) fn call_args(&mut self, args: &[kir::Arg]) -> Result<Vec<Value>, String> {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(match a {
                kir::Arg::Value(e) if e.ty.is_aggregate() => self.materialize(e)?,
                kir::Arg::Value(e) => self.expr(e)?,
                kir::Arg::Ref(p) => self.place_addr(p)?,
            });
        }
        Ok(vals)
    }

    pub(crate) fn call_id(&mut self, id: FuncId, args: &[Value]) -> Option<Value> {
        let fref = self.module.declare_func_in_func(id, self.b.func);
        let inst = self.b.ins().call(fref, args);
        self.b.inst_results(inst).first().copied()
    }
}
