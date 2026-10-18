//! Function values: `{code, env}` in memory, built from a thunk's address, and calls through them.

use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlags, Signature, Type, Value};
use cranelift_codegen::isa::CallConv;
use cranelift_module::Module;
use kestrel_kir::{self as kir, FuncIdx, KSig};

use super::{cl_type, FuncLowerer};

/// The signature of the code of a function value of signature `sig`: a pointer to the result when
/// it is an aggregate, then the environment (an `i64`), then the parameters by the rules of
/// `signature`: a pointer for each by-reference one, else its register type.
pub(crate) fn fn_value_sig(sig: &KSig, cc: CallConv, ptr: Type) -> Signature {
    let mut out = Signature::new(cc);
    let ret = sig.ret.filter(|r| !r.is_aggregate());
    if sig.ret.is_some_and(|r| r.is_aggregate()) {
        out.params.push(AbiParam::new(ptr));
    }
    out.params.push(AbiParam::new(types::I64));
    for &(ty, by_ref) in &sig.params {
        out.params.push(AbiParam::new(if by_ref { ptr } else { cl_type(ty) }));
    }
    if let Some(r) = ret {
        out.returns.push(AbiParam::new(cl_type(r)));
    }
    out
}

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// Writes the function value of the thunk `thunk` at `dest`: its address and a null environment.
    pub(crate) fn fn_ref_into(&mut self, thunk: FuncIdx, dest: Value) {
        let fref = self.module.declare_func_in_func(self.shared.funcs[thunk.0 as usize], self.b.func);
        let code = self.b.ins().func_addr(self.ptr, fref);
        self.b.ins().store(MemFlags::trusted(), code, dest, 0);
        let env = self.b.ins().iconst(types::I64, 0);
        self.b.ins().store(MemFlags::trusted(), env, dest, 8);
    }

    /// The environment word of the function value at a place.
    pub(crate) fn fn_env(&mut self, p: &kir::Place) -> Result<Value, String> {
        let addr = self.place_addr(p)?;
        Ok(self.b.ins().load(types::I64, MemFlags::trusted(), addr, 8))
    }

    /// Calls the code of the function value at `callee` with its environment, then `args`. The code
    /// and the environment are read before the arguments are evaluated. An aggregate result is
    /// written to `dest`, or to a temporary when the caller doesn't need it.
    pub(crate) fn call_indirect(
        &mut self,
        callee: &kir::Place,
        args: &[kir::Arg],
        sig: &KSig,
        dest: Option<Value>,
    ) -> Result<Option<Value>, String> {
        let addr = self.place_addr(callee)?;
        let code = self.b.ins().load(self.ptr, MemFlags::trusted(), addr, 0);
        let env = self.b.ins().load(types::I64, MemFlags::trusted(), addr, 8);
        let mut vals = Vec::with_capacity(args.len() + 2);
        if let Some(r) = sig.ret.filter(|r| r.is_aggregate()) {
            vals.push(match dest {
                Some(d) => d,
                None => self.temp_addr(r),
            });
        }
        vals.push(env);
        vals.extend(self.call_args(args)?);
        let cc = self.module.isa().default_call_conv();
        let sigref = self.b.import_signature(fn_value_sig(sig, cc, self.ptr));
        let inst = self.b.ins().call_indirect(sigref, code, &vals);
        Ok(self.b.inst_results(inst).first().copied())
    }
}
