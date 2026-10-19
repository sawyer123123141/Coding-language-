//! Method tables (`kir::Module::tables`): read-only data of 8-byte words, `[type_id, copy, print,
//! method_0, ...]`, every word after the type id a function's address. The addresses are
//! relocations (`write_function_addr`), which the JIT and object backends both resolve.

use cranelift_module::{DataDescription, DataId, FuncId, Module};
use kestrel_kir::{self as kir, TABLE_COPY, TABLE_METHODS, TABLE_PRINT};

/// Declares and defines every method table of `m`, given the functions' ids (by `FuncIdx`), and
/// returns the tables' ids, by `TableIdx`.
pub(crate) fn define_tables<M: Module>(module: &mut M, m: &kir::Module, funcs: &[FuncId]) -> Result<Vec<DataId>, String> {
    if module.target_config().pointer_bytes() != 8 {
        return Err("method tables need 8-byte pointers".into());
    }
    let mut ids = Vec::with_capacity(m.tables.len());
    for table in &m.tables {
        let words = TABLE_METHODS as usize + table.methods.len();
        let mut contents = vec![0u8; words * 8];
        contents[..8].copy_from_slice(&u64::from(table.type_id).to_le_bytes());
        let mut desc = DataDescription::new();
        desc.define(contents.into_boxed_slice());
        desc.set_align(8);
        let entries = [(TABLE_COPY, table.copy), (TABLE_PRINT, table.print)].into_iter();
        let methods = table.methods.iter().enumerate().map(|(n, &f)| (TABLE_METHODS + n as u32, f));
        for (word, f) in entries.chain(methods) {
            let fref = module.declare_func_in_data(funcs[f.0 as usize], &mut desc);
            desc.write_function_addr(word * 8, fref);
        }
        let id = module.declare_anonymous_data(false, false).map_err(|e| e.to_string())?;
        module.define_data(id, &desc).map_err(|e| e.to_string())?;
        ids.push(id);
    }
    Ok(ids)
}
