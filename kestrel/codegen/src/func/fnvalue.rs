//! Function values: `{code, env}` in memory, built from a thunk's or a closure's address, and calls
//! through them. A closure that captures values has a heap environment: a block in the current
//! arena with a 16-byte header (the address of its copy function, then the block's size) and the
//! values after it (see KIR's `lower::env`). A closure literal passed straight to a call has a stack
//! environment instead: the same block in a stack slot of the caller, holding shallow copies.

use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlags, Signature, StackSlotData, StackSlotKind, Type, Value};
use cranelift_codegen::isa::CallConv;
use cranelift_module::Module;
use kestrel_kir::{self as kir, FuncIdx, KSig, Place};

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
    /// The address of function `f`.
    pub(crate) fn func_address(&mut self, f: FuncIdx) -> Value {
        let fref = self.module.declare_func_in_func(self.shared.funcs[f.0 as usize], self.b.func);
        self.b.ins().func_addr(self.ptr, fref)
    }

    /// Writes the function value of the thunk `thunk` at `dest`: its address and a null environment.
    pub(crate) fn fn_ref_into(&mut self, thunk: FuncIdx, dest: Value) {
        let code = self.func_address(thunk);
        self.b.ins().store(MemFlags::trusted(), code, dest, 0);
        let env = self.b.ins().iconst(types::I64, 0);
        self.b.ins().store(MemFlags::trusted(), env, dest, 8);
    }

    /// Writes the function value of closure code `code` with a new heap environment at `dest`:
    /// the block is allocated in the current arena, then filled (see `fill_env`).
    pub(crate) fn heap_fn_into(&mut self, code: FuncIdx, env: &kir::Env, line: u32, dest: Value) -> Result<(), String> {
        let (one, size) = (self.b.ins().iconst(types::I64, 1), self.b.ins().iconst(types::I64, env.layout.size as i64));
        let align = self.b.ins().iconst(types::I64, 8);
        let (l, f) = self.loc_args(line);
        let block = self.call_id(self.shared.rt.alloc, &[one, size, align, l, f]).expect("returns a pointer");
        self.fill_env(code, env, block, size, dest)
    }

    /// Writes the function value of closure code `code` with a stack environment at `dest`: a stack
    /// slot of this function, filled like a heap environment (so a callee that stores the value
    /// copies it out through the copy function in the header), with shallow copies of the values.
    pub(crate) fn stack_fn_into(&mut self, code: FuncIdx, env: &kir::Env, dest: Value) -> Result<(), String> {
        let data = StackSlotData::new(StackSlotKind::ExplicitSlot, env.layout.size, 3);
        let slot = self.b.create_sized_stack_slot(data);
        let block = self.b.ins().stack_addr(self.ptr, slot, 0);
        let size = self.b.ins().iconst(types::I64, env.layout.size as i64);
        self.fill_env(code, env, block, size, dest)
    }

    /// Fills the environment `block` (of `size` bytes): the header (the copy function's address,
    /// then the size), then each value at its offset. Then writes `{code, block}` at `dest`, last,
    /// since a value may read `dest`.
    fn fill_env(&mut self, code: FuncIdx, env: &kir::Env, block: Value, size: Value, dest: Value) -> Result<(), String> {
        let copy = self.func_address(env.copy);
        self.b.ins().store(MemFlags::trusted(), copy, block, 0);
        self.b.ins().store(MemFlags::trusted(), size, block, 8);
        for (v, &offset) in env.values.iter().zip(&env.layout.offsets) {
            self.store_at(v, block, offset as i32)?;
        }
        let code = self.func_address(code);
        self.b.ins().store(MemFlags::trusted(), code, dest, 0);
        self.b.ins().store(MemFlags::trusted(), block, dest, 8);
        Ok(())
    }

    /// A copy of the environment block at `env` (an `i64` expression), in the current arena; its
    /// size is in its header.
    pub(crate) fn env_dup(&mut self, env: &kir::Expr) -> Result<Value, String> {
        let src = self.expr(env)?;
        let size = self.b.ins().load(types::I64, MemFlags::trusted(), src, 8);
        let align = self.b.ins().iconst(types::I64, 8);
        Ok(self.call_id(self.shared.rt.dup, &[src, size, align]).expect("returns a pointer"))
    }

    /// Points reference local `local` at `offset` bytes into the environment at `env`.
    pub(crate) fn bind_env(&mut self, local: kir::LocalIdx, env: &kir::Expr, offset: u32) -> Result<(), String> {
        let env = self.expr(env)?;
        let addr = self.b.ins().iadd_imm(env, offset as i64);
        self.bind_ref(local, addr);
        Ok(())
    }

    /// Gives the function value at `p` its own copy of its environment, through the copy function
    /// in the environment's header. A value without one (0) is left alone.
    pub(crate) fn copy_fn_env(&mut self, p: &Place) -> Result<(), String> {
        let addr = self.place_addr(p)?;
        let env = self.b.ins().load(types::I64, MemFlags::trusted(), addr, 8);
        let (copy_b, merge) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(env, copy_b, &[], merge, &[]);
        self.b.switch_to_block(copy_b);
        let copy = self.b.ins().load(self.ptr, MemFlags::trusted(), env, 0);
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let sigref = self.b.import_signature(sig);
        let inst = self.b.ins().call_indirect(sigref, copy, &[env]);
        let fresh = self.b.inst_results(inst)[0];
        self.b.ins().store(MemFlags::trusted(), fresh, addr, 8);
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(merge);
        Ok(())
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
