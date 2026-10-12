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

/// One `FileName` record per source file (see `kestrel_runtime::panic::FileName`): the name's
/// bytes, then a pointer to them and the length. A runtime error gets its file's record.
pub(crate) fn define_file_names<M: Module>(module: &mut M, m: &kir::Module) -> Result<Vec<DataId>, String> {
    let ptr_bytes = module.target_config().pointer_bytes() as usize;
    let mut ids = Vec::with_capacity(m.files.len());
    for name in &m.files {
        let bytes_id = module.declare_anonymous_data(false, false).map_err(|e| e.to_string())?;
        let mut bytes = DataDescription::new();
        // Zero-sized data objects aren't allowed; the length stays 0.
        bytes.define(if name.is_empty() { vec![0] } else { name.as_bytes().to_vec() }.into_boxed_slice());
        module.define_data(bytes_id, &bytes).map_err(|e| e.to_string())?;
        let id = module.declare_anonymous_data(false, false).map_err(|e| e.to_string())?;
        let mut record = DataDescription::new();
        let mut contents = vec![0u8; ptr_bytes + 8];
        contents[ptr_bytes..].copy_from_slice(&(name.len() as i64).to_le_bytes());
        record.define(contents.into_boxed_slice());
        // The runtime reads the record through a `*const FileName`, which must be aligned.
        record.set_align(8);
        let gv = module.declare_data_in_data(bytes_id, &mut record);
        record.write_data_addr(0, gv, 0);
        module.define_data(id, &record).map_err(|e| e.to_string())?;
        ids.push(id);
    }
    Ok(ids)
}
