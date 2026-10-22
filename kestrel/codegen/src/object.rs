use cranelift_codegen::ir::{types, AbiParam, InstBuilder, Signature};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use kestrel_kir as kir;

use crate::{define_module, isa, Options};

/// Compiles `m` to a native object file whose C `main` runs the Kestrel `main`.
pub fn emit_object(m: &kir::Module, opts: Options) -> Result<Vec<u8>, String> {
    let builder = ObjectBuilder::new(isa::host_isa()?, "kestrel", cranelift_module::default_libcall_names())
        .map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);
    let ids = define_module(&mut module, m, opts, None)?;
    let main = m.main.ok_or("a library has no `main` to build an executable from")?;
    define_c_main(&mut module, ids[main.0 as usize])?;
    module.finish().emit().map_err(|e| e.to_string())
}

/// `int main(int argc, char **argv) { return kestrel_rt_start(k_main); }`
fn define_c_main(module: &mut ObjectModule, kestrel_main: FuncId) -> Result<(), String> {
    let cc = module.isa().default_call_conv();
    let ptr = module.target_config().pointer_type();

    let mut start_sig = Signature::new(cc);
    start_sig.params.push(AbiParam::new(ptr));
    start_sig.returns.push(AbiParam::new(types::I32));
    let start = module
        .declare_function("kestrel_rt_start", Linkage::Import, &start_sig)
        .map_err(|e| e.to_string())?;

    let mut sig = Signature::new(cc);
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(ptr));
    sig.returns.push(AbiParam::new(types::I32));
    let main_id = module.declare_function("main", Linkage::Export, &sig).map_err(|e| e.to_string())?;

    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut fb_ctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
    let block = b.create_block();
    b.append_block_params_for_function_params(block);
    b.switch_to_block(block);
    b.seal_block(block);
    let main_ref = module.declare_func_in_func(kestrel_main, b.func);
    let entry = b.ins().func_addr(ptr, main_ref);
    let start_ref = module.declare_func_in_func(start, b.func);
    let call = b.ins().call(start_ref, &[entry]);
    let code = b.inst_results(call)[0];
    b.ins().return_(&[code]);
    b.finalize(module.target_config());
    module.define_function(main_id, &mut ctx).map_err(|e| format!("codegen failed for C main: {e:?}"))
}
