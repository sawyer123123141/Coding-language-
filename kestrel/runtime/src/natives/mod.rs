//! The built-in modules whose natives need only the runtime, not the window library: `file`.
//! They live here so a program that opens no window still finds them, in the runtime's
//! staticlib as in the window's (which carries the whole runtime).
//!
//! `abi` and `paths` are shared with the window crate's natives, which re-export them.

pub mod abi;
pub mod error;
pub mod file;
pub mod paths;

/// The runtime's native functions, as `(symbol, address)` pairs: what the JIT needs to link a
/// program that uses `file`. The CLI chains them after the window crate's table.
pub fn native_symbols() -> Vec<(&'static str, *const u8)> {
    use file::*;

    // Names come from the identifiers, so a symbol can't drift from its function.
    macro_rules! table {
        ($($native:ident),* $(,)?) => { vec![$((stringify!($native), $native as *const u8)),*] };
    }
    table![
        kestrel_file_read,
        kestrel_file_read_text,
        kestrel_file_write,
        kestrel_file_write_text,
        kestrel_file_exists,
        kestrel_file_list_dir,
        kestrel_file_make_dir,
        kestrel_file_remove,
        kestrel_file_error,
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::native_symbols;

    #[test]
    fn symbols_cover_every_native_the_modules_declare() {
        let mut declared = HashSet::new();
        for (module, source) in [("file", include_str!("../../../driver/std/file.kes"))] {
            for line in source.lines() {
                // `pub native f(...)` or `pub pure native f(...)`.
                if let Some(rest) = line.strip_prefix("pub native ").or_else(|| line.strip_prefix("pub pure native ")) {
                    let name = rest.split('(').next().unwrap();
                    declared.insert(format!("kestrel_{module}_{name}"));
                }
            }
        }
        let table = native_symbols();
        let provided: HashSet<String> = table.iter().map(|&(name, _)| name.to_string()).collect();
        assert_eq!(declared.len(), 9);
        assert_eq!(provided, declared);
        assert_eq!(table.len(), 9, "names are distinct");
        let addresses: HashSet<usize> = table.iter().map(|&(_, address)| address as usize).collect();
        assert_eq!(addresses.len(), 9, "each name has its own function");
    }
}
