use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings::{self, Configurable};

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
