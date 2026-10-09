use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::FuncId;
use kestrel_kir as kir;

use crate::{define_module, isa, rt, Options};

/// A module compiled into this process's memory.
pub struct JitProgram {
    module: JITModule,
    main: FuncId,
    funcs: Vec<(String, FuncId)>,
}

impl JitProgram {
    pub fn compile(m: &kir::Module, opts: Options) -> Result<Self, String> {
        let mut builder = JITBuilder::with_isa(isa::host_isa()?, cranelift_module::default_libcall_names());
        for (name, addr) in rt::jit_symbols() {
            builder.symbol(name, addr);
        }
        let mut module = JITModule::new(builder);
        let ids = define_module(&mut module, m, opts)?;
        module.finalize_definitions().map_err(|e| e.to_string())?;
        let funcs = m.funcs.iter().zip(&ids).map(|(f, &id)| (f.name.clone(), id)).collect();
        Ok(JitProgram { main: ids[m.main.0 as usize], module, funcs })
    }

    /// Machine code address of a Kestrel function (tests call these directly).
    pub fn fn_ptr(&self, name: &str) -> Option<*const u8> {
        let (_, id) = self.funcs.iter().find(|(n, _)| n == name)?;
        Some(self.module.get_finalized_function(*id))
    }

    /// Runs `main` through the runtime entry point; returns the exit code.
    /// The compiled code stays mapped until the process exits.
    pub fn run_main(&self) -> i32 {
        let code = self.module.get_finalized_function(self.main);
        // SAFETY: the checker requires `main()` to take no parameters and
        // return nothing, and it was compiled with the host's C convention.
        let entry: extern "C" fn() = unsafe { std::mem::transmute(code) };
        kestrel_runtime::start::kestrel_rt_start(entry)
    }
}
