use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::FuncId;
use kestrel_kir as kir;

use crate::export::{define_thunks, ExportInfo};
use crate::{define_module, isa, rt, Options};

/// A module compiled into this process's memory.
pub struct JitProgram {
    module: JITModule,
    main: Option<FuncId>,
    funcs: Vec<(String, FuncId)>,
    exports: Vec<(ExportInfo, FuncId)>,
}

/// A host's way into an exported function: arguments in, result out (see `ExportInfo`).
pub type Thunk = unsafe extern "C" fn(args: *const u8, ret: *mut u8);

impl JitProgram {
    pub fn compile(m: &kir::Module, opts: Options) -> Result<Self, String> {
        let mut builder = JITBuilder::with_isa(isa::host_isa()?, cranelift_module::default_libcall_names());
        for (name, addr) in rt::jit_symbols() {
            builder.symbol(name, addr);
        }
        let mut module = JITModule::new(builder);
        let ids = define_module(&mut module, m, opts)?;
        let exports = define_thunks(&mut module, m, &ids)?;
        module.finalize_definitions().map_err(|e| e.to_string())?;
        let funcs = m.funcs.iter().zip(&ids).map(|(f, &id)| (f.name.clone(), id)).collect();
        Ok(JitProgram { main: m.main.map(|f| ids[f.0 as usize]), module, funcs, exports })
    }

    /// The `pub` functions of the entry file, with the thunks that call them.
    pub fn exports(&self) -> Vec<(ExportInfo, Thunk)> {
        let thunk = |id: FuncId| {
            let code = self.module.get_finalized_function(id);
            // SAFETY: every thunk was defined with the `Thunk` signature.
            unsafe { std::mem::transmute::<*const u8, Thunk>(code) }
        };
        self.exports.iter().map(|(info, id)| (info.clone(), thunk(*id))).collect()
    }

    /// Machine code address of a Kestrel function (tests call these directly).
    pub fn fn_ptr(&self, name: &str) -> Option<*const u8> {
        let (_, id) = self.funcs.iter().find(|(n, _)| n == name)?;
        Some(self.module.get_finalized_function(*id))
    }

    /// Runs `main` through the runtime entry point; returns the exit code.
    /// The compiled code stays mapped until the process exits.
    pub fn run_main(&self) -> i32 {
        let code = self.module.get_finalized_function(self.main.expect("a program (not a library) has `main`"));
        // SAFETY: the checker requires `main()` to take no parameters and
        // return nothing, and it was compiled with the host's C convention.
        let entry: extern "C" fn() = unsafe { std::mem::transmute(code) };
        kestrel_runtime::start::kestrel_rt_start(entry)
    }
}
