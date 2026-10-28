use cranelift_codegen::isa::{OwnedTargetIsa, TargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use kestrel_kir::{self as kir, KTy};

/// The CPUs a Kestrel build runs on, as `kestrel build` says it.
pub const TARGET_CPUS: &str = "x86-64 CPUs with SSE4.1 (every one since about 2011)";

/// The one ISA every Kestrel build compiles for, the JIT and exes alike: x86-64 with SSE3, SSSE3
/// and SSE4.1 and nothing newer (no SSE4.2, POPCNT, AVX, FMA, BMI), so the same KIR gives the same
/// instructions on every machine. On another kind of host it is that host, as before. The flags
/// are the ones every Kestrel build uses.
pub(crate) fn target_isa() -> Result<OwnedTargetIsa, String> {
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
    let builder = if cfg!(target_arch = "x86_64") {
        // No feature detection: the baseline is the same on every host.
        let mut builder = cranelift_native::builder_with_options(false).map_err(|e| e.to_string())?;
        builder.enable("sse41").map_err(|e| e.to_string())?;
        builder
    } else {
        cranelift_native::builder().map_err(|e| e.to_string())?
    };
    builder.finish(settings::Flags::new(flags)).map_err(|e| e.to_string())
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

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::target_isa;

    #[test]
    fn the_target_isa_is_the_sse41_baseline() {
        let isa = target_isa().unwrap();
        let flags = isa.isa_flags();
        let has = |name: &str| flags.iter().any(|f| f.name == name && f.as_bool() == Some(true));
        for on in ["has_sse3", "has_ssse3", "has_sse41"] {
            assert!(has(on), "{on} should be on");
        }
        for off in ["has_sse42", "has_popcnt", "has_avx", "has_avx2", "has_fma", "has_bmi1", "has_bmi2", "has_lzcnt"] {
            assert!(!has(off), "{off} should be off");
        }
    }
}
