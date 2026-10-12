//! A function's code as Cranelift leaves it, for tests and benchmarks that check what the
//! compiler made of a loop: its IR after optimisation, then its machine code.

use cranelift_codegen::Context;
use cranelift_object::{ObjectBuilder, ObjectModule};
use kestrel_kir as kir;

use crate::{define_module, isa, Options};

pub(crate) struct Dump<'a> {
    pub name: &'a str,
    pub text: String,
}

impl Dump<'_> {
    /// Adds the optimised IR and the machine code of the function `ctx` just compiled.
    pub(crate) fn record(&mut self, ctx: &Context) {
        self.text.push_str(&ctx.func.display().to_string());
        if let Some(asm) = ctx.compiled_code().and_then(|c| c.vcode.as_deref()) {
            self.text.push('\n');
            self.text.push_str(asm);
        }
    }
}

/// The optimised Cranelift IR and the machine code of the function named `name` in `m`.
pub fn dump(m: &kir::Module, opts: Options, name: &str) -> Result<String, String> {
    let builder = ObjectBuilder::new(isa::host_isa()?, "kestrel", cranelift_module::default_libcall_names()).map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);
    let mut d = Dump { name, text: String::new() };
    define_module(&mut module, m, opts, Some(&mut d))?;
    if d.text.is_empty() {
        return Err(format!("no function named `{name}`"));
    }
    Ok(d.text)
}
