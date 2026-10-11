//! Globals: zero-filled data for the heap constants, their "built" flag and their arena
//! (`kir::Module::globals`). The same definitions serve the JIT and object files.

use cranelift_module::{DataDescription, DataId, Module};
use kestrel_kir as kir;

/// Declares and defines every global of `m` as writable data filled with zeros, and returns
/// their ids, indexed by `GlobalIdx`.
pub(crate) fn define_globals<M: Module>(module: &mut M, m: &kir::Module) -> Result<Vec<DataId>, String> {
    let mut ids = Vec::with_capacity(m.globals.len());
    for &ty in &m.globals {
        let (size, align) = m.size_align(ty);
        let id = module.declare_anonymous_data(true, false).map_err(|e| e.to_string())?;
        let mut desc = DataDescription::new();
        // Zero-sized data objects aren't allowed.
        desc.define_zeroinit(size.max(1) as usize);
        desc.set_align(align.max(1) as u64);
        module.define_data(id, &desc).map_err(|e| e.to_string())?;
        ids.push(id);
    }
    Ok(ids)
}
