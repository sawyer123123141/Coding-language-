//! The modules that ship inside the compiler: `window`, `draw` and `input`.
//! They are ordinary Kestrel files, embedded here, whose bodyless `native`
//! functions Rust implements (see `kestrel-window`). A program loads one with
//! `use window`, exactly as it would a file next to it.

use std::path::Path;

/// The directory name of an embedded module's virtual path.
const DIR: &str = "<builtin>";

/// Each built-in module: its name, and its source.
const MODULES: [(&str, &str); 3] = [
    ("window", include_str!("../std/window.kes")),
    ("draw", include_str!("../std/draw.kes")),
    ("input", include_str!("../std/input.kes")),
];

/// The built-in module called `name`, as its name and source.
pub(crate) fn find(name: &str) -> Option<(&'static str, &'static str)> {
    MODULES.iter().copied().find(|(n, _)| *n == name)
}

/// The path shown for built-in module `name` in diagnostics: `<builtin>/draw.kes`.
pub(crate) fn path(name: &str) -> String {
    format!("{DIR}/{name}.kes")
}

/// Whether `path` is the virtual path of a built-in module (see `path`).
pub(crate) fn is_path(path: &Path) -> bool {
    path.starts_with(DIR)
}

/// The error for a file `name.kes` that sits where a `use name` would find it.
pub(crate) fn shadowed(name: &str) -> String {
    format!("`{name}` is a built-in module, so {name}.kes can't be used as a module; rename the file")
}

/// The error for a `native` declaration in a file the user wrote.
pub(crate) const USER_NATIVE: &str = "only Kestrel's built-in modules can declare native functions";

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{find, is_path, path};

    #[test]
    fn every_module_is_found_by_name() {
        for name in ["window", "draw", "input"] {
            let (found, src) = find(name).expect(name);
            assert_eq!(found, name);
            assert!(src.starts_with("//"), "{name} starts with a comment");
        }
        assert!(find("geom").is_none());
    }

    #[test]
    fn virtual_paths_are_recognised() {
        assert!(is_path(Path::new(&path("draw"))));
        assert!(!is_path(Path::new("examples/draw.kes")));
        assert!(!is_path(Path::new("builtin/draw.kes")));
    }
}
