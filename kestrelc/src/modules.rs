// Module file resolution -- see docs/superpowers/specs/2026-07-25-
// modules-imports-design.md. `use module;` looks for `module.kes` in
// the same directory as the file containing the `use`, not the entry
// file's directory (so a transitive import resolves relative to
// whichever file actually wrote it). This is pure path arithmetic --
// no filesystem I/O beyond an existence check, and no reading, parsing,
// or merging of the resolved file happens here yet; that's separate
// follow-up work this function's callers don't exist yet.

use crate::ast::{Program, UseDecl};
use crate::error::{ErrorKind, KestrelcError};
use crate::span::Span;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parses `entry_path`, then transitively parses every module it (and
/// each module it pulls in) `use`s, resolving each by
/// `resolve_module_path` relative to whichever file wrote the `use`.
/// Returns every discovered file keyed by its resolved, canonicalized
/// path (entry file included) -- not yet merged into one `Program`
/// with qualified symbols; that's separate follow-up work.
///
/// Rejects an import cycle (A uses B uses A) as a compile error rather
/// than recursing forever, and a missing module file as a compile
/// error naming the expected path.
pub fn discover_modules(entry_path: &Path) -> Result<HashMap<PathBuf, Program>, KestrelcError> {
    let mut discovered = HashMap::new();
    let mut in_progress = Vec::new();
    discover_one(entry_path, &mut discovered, &mut in_progress)?;
    Ok(discovered)
}

fn discover_one(
    path: &Path,
    discovered: &mut HashMap<PathBuf, Program>,
    in_progress: &mut Vec<PathBuf>,
) -> Result<(), KestrelcError> {
    let canonical = path.canonicalize().map_err(|e| {
        KestrelcError::new(ErrorKind::Resolve, format!("kestrelc: can't read '{}': {e}", path.display()), Span::new(1, 1, 0))
    })?;

    if discovered.contains_key(&canonical) {
        return Ok(()); // already parsed (diamond import, or repeated use) -- idempotent
    }
    if in_progress.contains(&canonical) {
        let cycle = in_progress
            .iter()
            .skip_while(|p| **p != canonical)
            .chain(std::iter::once(&canonical))
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(KestrelcError::new(
            ErrorKind::Resolve,
            format!("kestrelc: import cycle detected: {cycle}"),
            Span::new(1, 1, 0),
        ));
    }

    let src = std::fs::read_to_string(&canonical).map_err(|e| {
        KestrelcError::new(ErrorKind::Resolve, format!("kestrelc: can't read '{}': {e}", canonical.display()), Span::new(1, 1, 0))
    })?;
    let program = crate::parser::parse(crate::lexer::lex(&src)?)?;

    in_progress.push(canonical.clone());
    for u in &program.uses {
        let module_name = match u {
            UseDecl::Module(name) => name,
            UseDecl::Names { module, .. } => module,
        };
        let module_name_text = module_name.resolve();
        match resolve_module_path(&canonical, &module_name_text) {
            Some(module_path) => discover_one(&module_path, discovered, in_progress)?,
            None => {
                return Err(KestrelcError::new(
                    ErrorKind::Resolve,
                    format!(
                        "kestrelc: module '{module_name_text}' not found -- expected '{}'",
                        canonical.parent().unwrap_or_else(|| Path::new(".")).join(format!("{module_name_text}.kes")).display()
                    ),
                    Span::new(1, 1, 0),
                ));
            }
        }
    }
    in_progress.pop();

    discovered.insert(canonical, program);
    Ok(())
}

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
    fn discovers_a_directly_used_module() {
        let dir = scratch_dir("discover_direct");
        fs::write(dir.join("math_utils.kes"), "fn noop() {}").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use math_utils;\nfn main() { print(1); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        assert_eq!(discovered.len(), 2);
    }

    #[test]
    fn discovers_transitively_used_modules() {
        let dir = scratch_dir("discover_transitive");
        fs::write(dir.join("c.kes"), "fn noop() {}").unwrap();
        fs::write(dir.join("b.kes"), "use c;\nfn noop() {}").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use b;\nfn main() { print(1); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        assert_eq!(discovered.len(), 3, "expected main, b, and c all discovered");
    }

    #[test]
    fn a_missing_module_is_a_clear_error_not_a_panic() {
        let dir = scratch_dir("discover_missing");
        let entry = dir.join("main.kes");
        fs::write(&entry, "use nope;\nfn main() { print(1); }").unwrap();
        let err = discover_modules(&entry).unwrap_err();
        assert!(err.message.contains("not found"), "got: {}", err.message);
    }

    #[test]
    fn an_import_cycle_is_a_compile_error_not_infinite_recursion() {
        let dir = scratch_dir("discover_cycle");
        fs::write(dir.join("a.kes"), "use b;\nfn noop() {}").unwrap();
        fs::write(dir.join("b.kes"), "use a;\nfn noop() {}").unwrap();
        let entry = dir.join("a.kes");
        let err = discover_modules(&entry).unwrap_err();
        assert!(err.message.contains("cycle"), "got: {}", err.message);
    }

    #[test]
    fn a_module_used_twice_is_discovered_once_not_an_error() {
        // main uses both b and c, and b also uses c (diamond) -- c must
        // be discovered exactly once, not treated as a cycle or error.
        let dir = scratch_dir("discover_diamond");
        fs::write(dir.join("c.kes"), "fn noop() {}").unwrap();
        fs::write(dir.join("b.kes"), "use c;\nfn noop() {}").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use b;\nuse c;\nfn main() { print(1); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        assert_eq!(discovered.len(), 3);
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
