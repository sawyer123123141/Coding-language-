// Module file resolution -- see docs/superpowers/specs/2026-07-25-
// modules-imports-design.md. `use module;` looks for `module.kes` in
// the same directory as the file containing the `use`, not the entry
// file's directory (so a transitive import resolves relative to
// whichever file actually wrote it). This is pure path arithmetic --
// no filesystem I/O beyond an existence check, and no reading, parsing,
// or merging of the resolved file happens here yet; that's separate
// follow-up work this function's callers don't exist yet.

use std::path::{Path, PathBuf};

/// Resolves `module_name` (e.g. `math_utils`, from `use math_utils;`)
/// to a `.kes` path in the same directory as `importer_path` (the file
/// containing that `use` statement). Returns `None` if no such file
/// exists -- callers turn that into a "module not found" compile error
/// naming the expected path, not a panic.
pub fn resolve_module_path(importer_path: &Path, module_name: &str) -> Option<PathBuf> {
    let dir = importer_path.parent().unwrap_or_else(|| Path::new("."));
    let candidate = dir.join(format!("{module_name}.kes"));
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kestrelc-modules-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_a_module_file_next_to_the_importer() {
        let dir = scratch_dir("found");
        fs::write(dir.join("math_utils.kes"), "fn noop() {}").unwrap();
        let importer = dir.join("main.kes");
        fs::write(&importer, "use math_utils;").unwrap();
        assert_eq!(resolve_module_path(&importer, "math_utils"), Some(dir.join("math_utils.kes")));
    }

    #[test]
    fn returns_none_when_no_such_file_exists() {
        let dir = scratch_dir("missing");
        let importer = dir.join("main.kes");
        fs::write(&importer, "use nope;").unwrap();
        assert_eq!(resolve_module_path(&importer, "nope"), None);
    }

    #[test]
    fn resolves_relative_to_the_importer_not_the_current_directory() {
        // A transitive import (this "importer" living in a subfolder)
        // must resolve next to itself, not next to wherever the
        // process happens to be running from.
        let dir = scratch_dir("nested");
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("geometry.kes"), "fn noop() {}").unwrap();
        let importer = sub.join("vectors.kes");
        fs::write(&importer, "use geometry;").unwrap();
        assert_eq!(resolve_module_path(&importer, "geometry"), Some(sub.join("geometry.kes")));
    }

    #[test]
    fn a_module_in_a_different_directory_is_not_found() {
        // Documents the design's stated v1 limitation: no path syntax,
        // same-directory-only resolution.
        let dir = scratch_dir("different_dir");
        let other = dir.join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("math_utils.kes"), "fn noop() {}").unwrap();
        let importer = dir.join("main.kes");
        fs::write(&importer, "use math_utils;").unwrap();
        assert_eq!(resolve_module_path(&importer, "math_utils"), None);
    }
}
