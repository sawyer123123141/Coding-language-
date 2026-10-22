//! Thunks for `par for` bodies: what the runtime's thread pool calls.
//! A thunk is `extern "C" fn(env, start, end)`; it loads the captured
//! variables' addresses from `env` and calls the outlined body with them.

use std::collections::HashMap;

use cranelift_codegen::ir::{types, AbiParam, Function, InstBuilder, MemFlagsData, Signature};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};
use kestrel_kir as kir;

/// Declares and defines a thunk for each `par for` body; `ids` are the
/// functions'. Returns each body's thunk, by `FuncIdx`.
pub(crate) fn define_par_thunks<M: Module>(module: &mut M, m: &kir::Module, ids: &[FuncId]) -> Result<HashMap<u32, FuncId>, String> {
    let ptr = module.target_config().pointer_type();
    let mut sig = Signature::new(module.isa().default_call_conv());
    sig.params.extend([AbiParam::new(ptr), AbiParam::new(types::I64), AbiParam::new(types::I64)]);
    let mut thunks = HashMap::new();
    let mut fb_ctx = FunctionBuilderContext::new();
    for &fi in &m.par_bodies {
        let f = &m.funcs[fi.0 as usize];
        let id = module
            .declare_function(&format!("k_thunk_{}", f.name), Linkage::Local, &sig)
            .map_err(|e| e.to_string())?;
        let mut func = Function::with_name_signature(Default::default(), sig.clone());
        {
            let mut b = FunctionBuilder::new(&mut func, &mut fb_ctx);
            let block = b.create_block();
            b.append_block_params_for_function_params(block);
            b.switch_to_block(block);
            b.seal_block(block);
            let [env, start, end] = b.block_params(block)[..] else { unreachable!("three parameters") };
            // The body's parameters: one address per capture, then `start` and `end`.
            let captures = f.params.len() - 2;
            let mut args: Vec<_> = (0..captures)
                .map(|k| b.ins().load(ptr, MemFlagsData::trusted(), env, (k * ptr.bytes() as usize) as i32))
                .collect();
            args.extend([start, end]);
            let callee = module.declare_func_in_func(ids[fi.0 as usize], b.func);
            b.ins().call(callee, &args);
            b.ins().return_(&[]);
            b.finalize(module.target_config());
        }
        let mut ctx = module.make_context();
        ctx.func = func;
        module.define_function(id, &mut ctx).map_err(|e| format!("par for thunk for `{}`: {e:?}", f.name))?;
        thunks.insert(fi.0, id);
    }
    Ok(thunks)
}
