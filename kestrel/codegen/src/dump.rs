//! A function's code as Cranelift leaves it, for tests and benchmarks that check what the
//! compiler made of a loop: its IR after optimisation, then its machine code; or its IR as Kestrel
//! hands it to Cranelift, before Cranelift optimises it.

use cranelift_codegen::Context;
use cranelift_object::{ObjectBuilder, ObjectModule};
use kestrel_kir as kir;

use crate::{define_module, isa, Options};

pub(crate) struct Dump<'a> {
    pub name: &'a str,
    pub text: String,
    /// Record the IR Cranelift is given instead (`record_input`), not what it makes of it.
    pub lowered: bool,
}

impl Dump<'_> {
    /// Adds the IR of `func` before optimisation, if this dump wants it.
    pub(crate) fn record_input(&mut self, func: &cranelift_codegen::ir::Function) {
        if self.lowered {
            self.text.push_str(&func.display().to_string());
        }
    }

    /// Adds the optimised IR and the machine code of the function `ctx` just compiled.
    pub(crate) fn record(&mut self, ctx: &Context) {
        if self.lowered {
            return;
        }
        self.text.push_str(&ctx.func.display().to_string());
        if let Some(asm) = ctx.compiled_code().and_then(|c| c.vcode.as_deref()) {
            self.text.push('\n');
            self.text.push_str(asm);
        }
    }
}

/// The optimised Cranelift IR and the machine code of the function named `name` in `m`.
pub fn dump(m: &kir::Module, opts: Options, name: &str) -> Result<String, String> {
    dump_with(m, opts, name, false)
}

/// The Cranelift IR of the function named `name` in `m` as Kestrel hands it to Cranelift (lowered,
/// then shaped by `phis` and `loop_exits`), before optimisation.
pub fn dump_lowered(m: &kir::Module, opts: Options, name: &str) -> Result<String, String> {
    dump_with(m, opts, name, true)
}

fn dump_with(m: &kir::Module, opts: Options, name: &str, lowered: bool) -> Result<String, String> {
    let builder = ObjectBuilder::new(isa::host_isa()?, "kestrel", cranelift_module::default_libcall_names()).map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);
    let mut d = Dump { name, text: String::new(), lowered };
    define_module(&mut module, m, opts, Some(&mut d))?;
    if d.text.is_empty() {
        return Err(format!("no function named `{name}`"));
    }
    Ok(d.text)
}
