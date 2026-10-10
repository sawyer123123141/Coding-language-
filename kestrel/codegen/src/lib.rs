pub mod export;
mod func;
mod isa;
pub mod jit;
pub mod object;
mod rt;

use cranelift_codegen::ir::Function;
use cranelift_frontend::FunctionBuilderContext;
use cranelift_module::{FuncId, Linkage, Module};
use kestrel_kir as kir;

pub use export::{ExportInfo, ExportTy};
pub use jit::JitProgram;
pub use object::emit_object;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Panic on integer overflow (exit 101). Off for `--release`.
    pub checked_arith: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { checked_arith: true }
    }
}

/// Symbol name of a Kestrel function, kept apart from C names like `main`.
pub(crate) fn symbol(name: &str) -> String {
    format!("k_{name}")
}

/// Declares and defines every function of `m` in `module`.
/// Returns their ids, indexed by `FuncIdx`.
pub(crate) fn define_module<M: Module>(module: &mut M, m: &kir::Module, opts: Options) -> Result<Vec<FuncId>, String> {
    let cc = module.isa().default_call_conv();
    let ptr = module.target_config().pointer_type();
    let rt = rt::RtFuncs::declare(module)?;
    let mut ids = Vec::with_capacity(m.funcs.len());
    for f in &m.funcs {
        let sig = func::signature(f, cc, ptr);
        let id = module.declare_function(&symbol(&f.name), Linkage::Local, &sig).map_err(|e| e.to_string())?;
        ids.push(id);
    }
    let shared = func::Shared { funcs: &ids, rt: &rt, opts, kir: m };
    let mut strings = func::Strings::default();
    let mut ctx = module.make_context();
    let mut fb_ctx = FunctionBuilderContext::new();
    for (f, &id) in m.funcs.iter().zip(&ids) {
        ctx.func = Function::with_name_signature(Default::default(), func::signature(f, cc, ptr));
        func::lower_func(module, &mut strings, &shared, &mut ctx.func, &mut fb_ctx, f)?;
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("codegen failed for `{}`: {e:?}", f.name))?;
        module.clear_context(&mut ctx);
    }
    Ok(ids)
}
