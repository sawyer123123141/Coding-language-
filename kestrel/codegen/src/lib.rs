mod dump;
pub mod export;
mod func;
mod globals;
mod isa;
mod native;
mod par_thunk;
pub mod jit;
pub mod object;
mod rt;

use cranelift_codegen::ir::Function;
use cranelift_frontend::FunctionBuilderContext;
use cranelift_module::{FuncId, Linkage, Module};
use kestrel_kir as kir;

#[doc(hidden)]
pub use dump::dump;
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

/// Declares every function of `m` in `module` and defines the ones with bodies
/// (natives are imports under their own symbols), and defines its globals. Returns
/// the functions' ids, indexed by `FuncIdx`. With `dump`, the named function's code is
/// recorded in it as it is compiled.
pub(crate) fn define_module<M: Module>(
    module: &mut M,
    m: &kir::Module,
    opts: Options,
    mut dump: Option<&mut dump::Dump>,
) -> Result<Vec<FuncId>, String> {
    let cc = module.isa().default_call_conv();
    let ptr = module.target_config().pointer_type();
    let rt = rt::RtFuncs::declare(module)?;
    let mut ids = Vec::with_capacity(m.funcs.len());
    for f in &m.funcs {
        let id = match &f.native {
            Some(import) => module.declare_function(import, Linkage::Import, &native::signature(f, cc, ptr)),
            None => module.declare_function(&symbol(&f.name), Linkage::Local, &func::signature(f, cc, ptr)),
        };
        ids.push(id.map_err(|e| e.to_string())?);
    }
    let par_thunks = par_thunk::define_par_thunks(module, m, &ids)?;
    let globals = globals::define_globals(module, m)?;
    let shared = func::Shared { funcs: &ids, par_thunks: &par_thunks, rt: &rt, globals: &globals, opts, kir: m };
    let mut strings = func::Strings::default();
    let mut ctx = module.make_context();
    let mut fb_ctx = FunctionBuilderContext::new();
    for (f, &id) in m.funcs.iter().zip(&ids).filter(|(f, _)| f.native.is_none()) {
        ctx.func = Function::with_name_signature(Default::default(), func::signature(f, cc, ptr));
        func::lower_func(module, &mut strings, &shared, &mut ctx.func, &mut fb_ctx, f)?;
        let dumping = dump.as_ref().is_some_and(|d| d.name == f.name);
        ctx.set_disasm(dumping);
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("codegen failed for `{}`: {e:?}", f.name))?;
        if let Some(d) = dump.as_deref_mut().filter(|_| dumping) {
            d.record(&ctx);
        }
        module.clear_context(&mut ctx);
    }
    Ok(ids)
}
