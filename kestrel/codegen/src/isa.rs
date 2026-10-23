use cranelift_codegen::isa::{OwnedTargetIsa, TargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use kestrel_kir::{self as kir, KTy};

/// The host machine, with the flags every Kestrel build uses.
pub(crate) fn host_isa() -> Result<OwnedTargetIsa, String> {
    let mut flags = settings::builder();
    // Probestack: Windows grows the stack one guard page at a time, so large
    // frames must touch each page in order.
    for (key, value) in [
        ("opt_level", "speed"),
        ("is_pic", "true"),
        ("enable_probestack", "true"),
        ("probestack_strategy", "inline"),
    ] {
        flags.set(key, value).map_err(|e| e.to_string())?;
    }
    cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())
}

/// C1: SIMD vectors need SSE4.1 (`pmulld`, `pminsd`, `roundps`, `ptest`; without it Cranelift
/// would call rounding functions Kestrel doesn't provide).
const NO_SSE41: &str = "`vec4` and `ivec4` need a CPU with SSE4.1 (every x64 CPU since about 2011); this one doesn't have it, so use `vec2`, `vec3` or plain numbers instead";

/// Refuses a module that uses a SIMD vector (`vec4`, `ivec4`) for an x64 `isa` without SSE4.1.
pub(crate) fn check_simd(isa: &dyn TargetIsa, m: &kir::Module) -> Result<(), String> {
    let sse41 = isa.isa_flags().iter().any(|f| f.name == "has_sse41" && f.as_bool() == Some(true));
    if isa.name() == "x64" && !sse41 && uses_vectors(m) {
        return Err(NO_SSE41.to_string());
    }
    Ok(())
}

/// Whether any value of `m` is a SIMD vector: a local, a result, a field, an element, or any
/// expression.
fn uses_vectors(m: &kir::Module) -> bool {
    let fields = m.structs.iter().flat_map(|s| &s.fields).chain(m.enums.iter().flat_map(|e| e.variants.iter().flat_map(|v| &v.fields)));
    if m.elems.iter().copied().chain(fields.map(|f| f.ty)).any(KTy::is_vector) {
        return true;
    }
    m.funcs.iter().any(|f| {
        let mut found = f.ret.is_some_and(KTy::is_vector) || f.locals.iter().any(|l| l.ty.is_vector());
        kir::visit::walk(&f.body, &mut |n| {
            if let kir::visit::Node::Expr(e) = n {
                found |= e.ty.is_vector();
            }
        });
        found
    })
}
