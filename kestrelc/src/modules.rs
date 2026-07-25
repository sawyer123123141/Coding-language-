// Module file resolution -- see docs/superpowers/specs/2026-07-25-
// modules-imports-design.md. `use module;` looks for `module.kes` in
// the same directory as the file containing the `use`, not the entry
// file's directory (so a transitive import resolves relative to
// whichever file actually wrote it). This is pure path arithmetic --
// no filesystem I/O beyond an existence check, and no reading, parsing,
// or merging of the resolved file happens here yet; that's separate
// follow-up work this function's callers don't exist yet.

use crate::ast::{Expr, ExprKind, Fn, Program, Stmt, StructDecl, Type, UseDecl};
use crate::error::{ErrorKind, KestrelcError};
use crate::interner::{intern, Symbol};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A resolved file's module name is its filename stem
/// (`math_utils.kes` -> `math_utils`). No validation needed here: no
/// `use` spelling can ever name a non-identifier module (a hyphenated
/// filename, say), so that case already failed at discovery time with
/// "module not found" rather than reaching this far.
fn module_name_for_path(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("module").to_string()
}

fn rewrite_type(ty: &mut Type, rename: &HashMap<Symbol, Symbol>) {
    match ty {
        Type::Named(name) => {
            if let Some(&q) = rename.get(name) {
                *name = q;
            }
        }
        Type::Array { elem, .. } => rewrite_type(elem, rename),
    }
}

/// Renames only the Symbol positions that can ever denote a
/// function/struct *name* -- `Call.name`, `StructLit.name`, and (the
/// one place a bare function name appears without being called)
/// `parallel_map`'s first argument. A plain `ExprKind::Ident` anywhere
/// else is always a local variable/parameter read, never a function
/// reference, so it's deliberately left untouched here -- renaming it
/// too would incorrectly rewrite a local variable that happens to
/// share a name with an imported/declared function.
///
/// `denied` holds synthesized "module.name" symbols (see parser.rs's
/// qualified-call desugar) for names that exist in their source module
/// but aren't `pub` -- encountering one as a Call/StructLit name is a
/// compile error naming the module, distinct from the generic "unknown
/// function" error a genuinely nonexistent name gets instead (that one
/// is never in `rename` or `denied`, so it just falls through unchanged
/// to resolve.rs's existing check).
fn rewrite_expr(e: &mut Expr, rename: &HashMap<Symbol, Symbol>, denied: &HashMap<Symbol, String>) -> Result<(), KestrelcError> {
    match &mut e.kind {
        ExprKind::Num(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Ident(_) => {}
        ExprKind::ArrayLit(elems) => {
            for el in elems {
                rewrite_expr(el, rename, denied)?;
            }
        }
        ExprKind::Unary { expr, .. } => rewrite_expr(expr, rename, denied)?,
        ExprKind::Binop { left, right, .. } => {
            rewrite_expr(left, rename, denied)?;
            rewrite_expr(right, rename, denied)?;
        }
        ExprKind::Index { target, index } => {
            rewrite_expr(target, rename, denied)?;
            rewrite_expr(index, rename, denied)?;
        }
        ExprKind::Call { name, args } => {
            let is_parallel_map = *name == crate::interner::well_known::parallel_map();
            if let Some(reason) = denied.get(name) {
                return Err(KestrelcError::internal(ErrorKind::Resolve, reason.clone()));
            }
            if let Some(&q) = rename.get(name) {
                *name = q;
            }
            if is_parallel_map {
                if let Some(first) = args.first_mut() {
                    if let ExprKind::Ident(fn_name) = &mut first.kind {
                        if let Some(reason) = denied.get(fn_name) {
                            return Err(KestrelcError::internal(ErrorKind::Resolve, reason.clone()));
                        }
                        if let Some(&q) = rename.get(fn_name) {
                            *fn_name = q;
                        }
                    }
                }
            }
            for a in args {
                rewrite_expr(a, rename, denied)?;
            }
        }
        ExprKind::StructLit { name, fields } => {
            if let Some(reason) = denied.get(name) {
                return Err(KestrelcError::internal(ErrorKind::Resolve, reason.clone()));
            }
            if let Some(&q) = rename.get(name) {
                *name = q;
            }
            for (_, v) in fields {
                rewrite_expr(v, rename, denied)?;
            }
        }
        ExprKind::Field { target, .. } => rewrite_expr(target, rename, denied)?,
    }
    Ok(())
}

fn rewrite_stmt(s: &mut Stmt, rename: &HashMap<Symbol, Symbol>, denied: &HashMap<Symbol, String>) -> Result<(), KestrelcError> {
    match s {
        Stmt::Let { value, .. } | Stmt::Assign { value, .. } | Stmt::FieldAssign { value, .. } => {
            rewrite_expr(value, rename, denied)?
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => {}
        Stmt::If { cond, then_block, else_block, .. } => {
            rewrite_expr(cond, rename, denied)?;
            for st in then_block {
                rewrite_stmt(st, rename, denied)?;
            }
            if let Some(eb) = else_block {
                for st in eb {
                    rewrite_stmt(st, rename, denied)?;
                }
            }
        }
        Stmt::While { cond, body, .. } => {
            rewrite_expr(cond, rename, denied)?;
            for st in body {
                rewrite_stmt(st, rename, denied)?;
            }
        }
        Stmt::RangeFor { start, end, body, .. } => {
            rewrite_expr(start, rename, denied)?;
            rewrite_expr(end, rename, denied)?;
            for st in body {
                rewrite_stmt(st, rename, denied)?;
            }
        }
        Stmt::Print { args, .. } => {
            for a in args {
                rewrite_expr(a, rename, denied)?;
            }
        }
        Stmt::Return { value, .. } => {
            if let Some(v) = value {
                rewrite_expr(v, rename, denied)?;
            }
        }
        Stmt::ExprStmt { expr, .. } => rewrite_expr(expr, rename, denied)?,
    }
    Ok(())
}

fn rewrite_fn_signature_and_body(f: &mut Fn, rename: &HashMap<Symbol, Symbol>, denied: &HashMap<Symbol, String>) -> Result<(), KestrelcError> {
    for p in &mut f.params {
        rewrite_type(&mut p.ty, rename);
    }
    if let Some(rt) = &mut f.return_type {
        rewrite_type(rt, rename);
    }
    if let Some(wc) = &mut f.where_clause {
        rewrite_expr(wc, rename, denied)?;
    }
    for s in &mut f.body {
        rewrite_stmt(s, rename, denied)?;
    }
    Ok(())
}

fn rewrite_struct(s: &mut StructDecl, rename: &HashMap<Symbol, Symbol>) {
    for f in &mut s.fields {
        rewrite_type(&mut f.ty, rename);
    }
}

/// Qualifies and merges every module `discover_modules` found into one
/// `Program`, ready for the existing resolve/typecheck/purity/codegen
/// pipeline exactly as it runs today. Every function/struct (including
/// the entry file's own, except its `main`) is renamed to
/// `{module}${name}` -- this qualified symbol is also what codegen
/// will use as the object-file/linker export name (a later, separate
/// codegen change, not this function's job), which is what actually
/// prevents two unrelated modules' same-named functions from clashing
/// at the object-file level, not just a Kestrel-level naming trick.
///
/// Handles `use a, b from module;` (unqualified) fully: existence is
/// validated against the target module's real declarations, and a
/// name colliding with a local declaration or another `from` import in
/// the same module is a compile error. A bare `use module;`
/// (whole-module qualified `module.fn()` access) is recorded by
/// `discover_modules` but has no resolvable call syntax yet -- see
/// docs/superpowers/specs/2026-07-25-modules-imports-design.md.
pub fn merge_modules(entry_path: &Path, mut discovered: HashMap<PathBuf, (String, Program)>) -> Result<Program, KestrelcError> {
    // A file with zero `use` items is the only way `discover_modules`
    // ever returns exactly one entry (any successful `use` discovers at
    // least one more file, or fails outright) -- skip qualification
    // entirely for it, so a plain single-file program compiles with
    // identical function/struct names and identical error messages to
    // before this feature existed. Qualifying every name unconditionally
    // (even `helper` -> `myfile$helper` with no imports involved at all)
    // would be pure noise for the overwhelmingly common no-imports case,
    // and would change user-visible error message text for no reason.
    if discovered.len() == 1 {
        return Ok(discovered.drain().next().expect("len == 1").1 .1);
    }

    let entry_canonical = entry_path.canonicalize().map_err(|e| {
        KestrelcError::internal(ErrorKind::Resolve, format!("kestrelc: can't read '{}': {e}", entry_path.display()))
    })?;

    // module name -> its own declared fn/struct names paired with
    // whether each is `pub` -- used to validate every from-import and
    // bare-use qualified call actually names something real AND
    // visible (see docs/superpowers/specs/2026-07-25-visibility-
    // design.md).
    let declared: HashMap<String, (Vec<(String, bool)>, Vec<(String, bool)>)> = discovered
        .iter()
        .map(|(path, (_src, prog))| {
            let name = module_name_for_path(path);
            let fns = prog.fns.iter().map(|f| (f.name.resolve().to_string(), f.pub_)).collect();
            let structs = prog.structs.iter().map(|s| (s.name.resolve().to_string(), s.pub_)).collect();
            (name, (fns, structs))
        })
        .collect();

    let mut merged_fns = Vec::new();
    let mut merged_structs = Vec::new();

    for (path, (_src, program)) in discovered {
        let module_name = module_name_for_path(&path);
        let is_entry = path == entry_canonical;

        let mut rename: HashMap<Symbol, Symbol> = HashMap::new();
        let mut denied: HashMap<Symbol, String> = HashMap::new();
        for f in &program.fns {
            let is_entry_main = is_entry && f.name == crate::interner::well_known::main();
            if !is_entry_main {
                rename.insert(f.name, intern(&format!("{module_name}${}", f.name.resolve())));
            }
        }
        for s in &program.structs {
            rename.insert(s.name, intern(&format!("{module_name}${}", s.name.resolve())));
        }

        let mut seen_from_names: HashMap<String, String> = HashMap::new();
        for u in &program.uses {
            if let UseDecl::Module(module) = u {
                // Bare `use module;` -- every declared *public* name
                // becomes callable/constructible as `module.name(...)`,
                // which the parser already desugars to a plain `Call`/
                // `StructLit` under the synthesized symbol
                // `"module.name"` (see parser.rs's qualified-call
                // parsing). Map that synthesized symbol straight to
                // the real qualified `module$name` here -- no new
                // rewrite logic needed for the public case, this reuses
                // the exact same rename table `rewrite_expr`'s Call/
                // StructLit arms already consult. A private name gets a
                // `denied` entry instead, so an actual attempt to call
                // it (not just the `use module;` line itself) is what
                // raises the "is private" error, right at the call
                // site -- there's no way to know at this point whether
                // the importing module will ever try to call any
                // particular one of the source module's names.
                let source_module = module.resolve().to_string();
                let (source_fns, source_structs) = declared.get(&source_module).ok_or_else(|| {
                    KestrelcError::internal(
                        ErrorKind::Resolve,
                        format!("kestrelc: module '{source_module}' not found"),
                    )
                })?;
                for (fn_name, is_pub) in source_fns {
                    let qualified_call_symbol = intern(&format!("{source_module}.{fn_name}"));
                    if *is_pub {
                        rename.insert(qualified_call_symbol, intern(&format!("{source_module}${fn_name}")));
                    } else {
                        denied.insert(
                            qualified_call_symbol,
                            format!("kestrelc: '{fn_name}' is private to module '{source_module}'"),
                        );
                    }
                }
                for (struct_name, is_pub) in source_structs {
                    let qualified_call_symbol = intern(&format!("{source_module}.{struct_name}"));
                    if *is_pub {
                        rename.insert(qualified_call_symbol, intern(&format!("{source_module}${struct_name}")));
                    } else {
                        denied.insert(
                            qualified_call_symbol,
                            format!("kestrelc: '{struct_name}' is private to module '{source_module}'"),
                        );
                    }
                }
            }
            if let UseDecl::Names { names, module } = u {
                let source_module = module.resolve().to_string();
                let (source_fns, source_structs) = declared.get(&source_module).ok_or_else(|| {
                    KestrelcError::internal(
                        ErrorKind::Resolve,
                        format!("kestrelc: module '{source_module}' not found"),
                    )
                })?;
                for name in names {
                    let name_text = name.resolve().to_string();
                    let is_pub = source_fns
                        .iter()
                        .find(|(n, _)| n == &name_text)
                        .or_else(|| source_structs.iter().find(|(n, _)| n == &name_text))
                        .map(|(_, p)| *p);
                    match is_pub {
                        None => {
                            return Err(KestrelcError::internal(
                                ErrorKind::Resolve,
                                format!("kestrelc: '{name_text}' not found in module '{source_module}'"),
                            ));
                        }
                        Some(false) => {
                            return Err(KestrelcError::internal(
                                ErrorKind::Resolve,
                                format!("kestrelc: '{name_text}' is private to module '{source_module}'"),
                            ));
                        }
                        Some(true) => {}
                    }
                    let collides_locally = program.fns.iter().any(|f| f.name.resolve().as_ref() == name_text)
                        || program.structs.iter().any(|s| s.name.resolve().as_ref() == name_text);
                    if collides_locally {
                        return Err(KestrelcError::internal(
                            ErrorKind::Resolve,
                            format!(
                                "kestrelc: '{name_text}' imported from '{source_module}' collides with a local declaration in '{module_name}'"
                            ),
                        ));
                    }
                    if let Some(prior_module) = seen_from_names.get(&name_text) {
                        return Err(KestrelcError::internal(
                            ErrorKind::Resolve,
                            format!(
                                "kestrelc: '{name_text}' imported from both '{prior_module}' and '{source_module}' in '{module_name}'"
                            ),
                        ));
                    }
                    seen_from_names.insert(name_text.clone(), source_module.clone());
                    rename.insert(*name, intern(&format!("{source_module}${name_text}")));
                }
            }
        }

        for mut f in program.fns {
            rewrite_fn_signature_and_body(&mut f, &rename, &denied)?;
            if let Some(&qualified) = rename.get(&f.name) {
                f.name = qualified;
            } // else: the entry file's own `main`, left unqualified.
            merged_fns.push(f);
        }
        for mut s in program.structs {
            rewrite_struct(&mut s, &rename);
            if let Some(&qualified) = rename.get(&s.name) {
                s.name = qualified;
            }
            merged_structs.push(s);
        }
    }

    Ok(Program { fns: merged_fns, structs: merged_structs, uses: Vec::new() })
}

/// Parses `entry_path`, then transitively parses every module it (and
/// each module it pulls in) `use`s, resolving each by
/// `resolve_module_path` relative to whichever file wrote the `use`.
/// Returns every discovered file keyed by its resolved, canonicalized
/// path (entry file included), each paired with its own raw source text
/// -- kept alongside the parsed `Program` specifically so a caller (see
/// `cache_key_material`) can fold every transitively-used file's actual
/// content into a compile-cache key, not just the entry file's. Without
/// this, editing an imported module without touching the entry file
/// would let a cache keyed only on the entry's text silently keep
/// serving a stale binary compiled against the old module content.
///
/// Rejects an import cycle (A uses B uses A) as a compile error rather
/// than recursing forever, and a missing module file as a compile
/// error naming the expected path.
pub fn discover_modules(entry_path: &Path) -> Result<HashMap<PathBuf, (String, Program)>, KestrelcError> {
    let mut discovered = HashMap::new();
    let mut in_progress = Vec::new();
    discover_one(entry_path, &mut discovered, &mut in_progress)?;
    Ok(discovered)
}

/// Deterministic (path-sorted, so hashing order never depends on
/// `HashMap` iteration order) concatenation of every discovered file's
/// path and content -- the actual string a caller should hash/fold into
/// a compile-cache key so the key reflects every file that could affect
/// the compiled output, not just the entry file's own text.
pub fn cache_key_material(discovered: &HashMap<PathBuf, (String, Program)>) -> String {
    let mut paths: Vec<&PathBuf> = discovered.keys().collect();
    paths.sort();
    let mut out = String::new();
    for p in paths {
        out.push_str(&p.to_string_lossy());
        out.push('\0');
        out.push_str(&discovered[p].0);
        out.push('\0');
    }
    out
}

fn discover_one(
    path: &Path,
    discovered: &mut HashMap<PathBuf, (String, Program)>,
    in_progress: &mut Vec<PathBuf>,
) -> Result<(), KestrelcError> {
    let canonical = path.canonicalize().map_err(|e| {
        KestrelcError::internal(ErrorKind::Resolve, format!("kestrelc: can't read '{}': {e}", path.display()))
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
        return Err(KestrelcError::internal(
            ErrorKind::Resolve,
            format!("kestrelc: import cycle detected: {cycle}"),
        ));
    }

    let src = std::fs::read_to_string(&canonical).map_err(|e| {
        KestrelcError::internal(ErrorKind::Resolve, format!("kestrelc: can't read '{}': {e}", canonical.display()))
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
                return Err(KestrelcError::internal(
                    ErrorKind::Resolve,
                    format!(
                        "kestrelc: module '{module_name_text}' not found -- expected '{}'",
                        canonical.parent().unwrap_or_else(|| Path::new(".")).join(format!("{module_name_text}.kes")).display()
                    ),
                ));
            }
        }
    }
    in_progress.pop();

    discovered.insert(canonical, (src, program));
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

    fn find_fn<'a>(program: &'a Program, name: &str) -> &'a crate::ast::Fn {
        program.fns.iter().find(|f| f.name.resolve().as_ref() == name).unwrap_or_else(|| panic!("no fn named '{name}' in merged program: {:?}", program.fns.iter().map(|f| f.name.resolve().to_string()).collect::<Vec<_>>()))
    }

    #[test]
    fn a_program_with_no_use_statements_is_left_completely_unqualified() {
        let dir = scratch_dir("merge_no_imports_no_op");
        let entry = dir.join("main.kes");
        fs::write(&entry, "fn helper() { print(1); } fn main() { helper(); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();
        // Names must stay exactly as written -- no "main$helper"
        // qualification for a program that never imports anything.
        find_fn(&merged, "helper");
        find_fn(&merged, "main");
        assert_eq!(merged.fns.len(), 2);
    }

    #[test]
    fn same_module_functions_get_qualified_names_and_self_calls_follow() {
        // Qualification only kicks in once a real second module is
        // involved (see a_program_with_no_use_statements_is_left_
        // completely_unqualified for the no-imports case) -- this test
        // pulls in an unrelated dummy module just to trigger it, then
        // checks that the entry file's own self-call (helper(), nothing
        // to do with the import) still gets qualified and rewritten
        // consistently.
        let dir = scratch_dir("merge_self_call");
        fs::write(dir.join("dummy.kes"), "fn noop() {}").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use dummy;\nfn helper() { print(1); } fn main() { helper(); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();

        // main itself stays unqualified (entry-file main exclusivity).
        find_fn(&merged, "main");
        // helper (a non-main entry-file function) is qualified by its
        // own module name.
        let helper = find_fn(&merged, "main$helper");
        // main's call to helper() must have been rewritten to call the
        // qualified name, not the original.
        let main_fn = find_fn(&merged, "main");
        let Stmt::ExprStmt { expr, .. } = &main_fn.body[0] else { panic!("expected ExprStmt") };
        let ExprKind::Call { name, .. } = &expr.kind else { panic!("expected Call") };
        assert_eq!(*name, helper.name);
    }

    #[test]
    fn a_from_import_resolves_to_the_source_modules_qualified_name() {
        let dir = scratch_dir("merge_from_import");
        fs::write(dir.join("geometry.kes"), "pub pure fn sqrt(x: i64) -> i64 { return x; }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use sqrt from geometry;\nfn main() { print(sqrt(4)); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();

        let sqrt_fn = find_fn(&merged, "geometry$sqrt");
        let main_fn = find_fn(&merged, "main");
        let Stmt::Print { args, .. } = &main_fn.body[0] else { panic!("expected Print") };
        let ExprKind::Call { name, .. } = &args[0].kind else { panic!("expected Call") };
        assert_eq!(*name, sqrt_fn.name);
    }

    #[test]
    fn a_from_import_naming_a_nonexistent_function_is_a_compile_error() {
        let dir = scratch_dir("merge_from_missing_name");
        fs::write(dir.join("geometry.kes"), "fn noop() {}").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use nope from geometry;\nfn main() { print(1); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let err = merge_modules(&entry, discovered).unwrap_err();
        assert!(err.message.contains("not found in module"), "got: {}", err.message);
    }

    #[test]
    fn a_from_import_colliding_with_a_local_declaration_is_a_compile_error() {
        let dir = scratch_dir("merge_from_collides_local");
        fs::write(dir.join("geometry.kes"), "pub fn helper() {}").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use helper from geometry;\nfn helper() {}\nfn main() { print(1); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let err = merge_modules(&entry, discovered).unwrap_err();
        assert!(err.message.contains("collides"), "got: {}", err.message);
    }

    #[test]
    fn two_from_imports_of_the_same_name_from_different_modules_is_a_compile_error() {
        let dir = scratch_dir("merge_from_collides_two_imports");
        fs::write(dir.join("a.kes"), "pub fn helper() {}").unwrap();
        fs::write(dir.join("b.kes"), "pub fn helper() {}").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use helper from a;\nuse helper from b;\nfn main() { print(1); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let err = merge_modules(&entry, discovered).unwrap_err();
        assert!(err.message.contains("imported from both"), "got: {}", err.message);
    }

    #[test]
    fn same_named_functions_in_unrelated_modules_dont_collide_after_qualification() {
        let dir = scratch_dir("merge_unrelated_same_name");
        fs::write(dir.join("a.kes"), "fn helper() {}").unwrap();
        fs::write(dir.join("b.kes"), "fn helper() {}").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use a;\nuse b;\nfn main() { print(1); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();
        find_fn(&merged, "a$helper");
        find_fn(&merged, "b$helper");
        assert_eq!(merged.fns.len(), 3); // main, a$helper, b$helper
    }

    #[test]
    fn a_struct_from_import_is_qualified_and_struct_lit_follows() {
        let dir = scratch_dir("merge_struct_from_import");
        fs::write(dir.join("shapes.kes"), "pub struct Point { x: i64, y: i64 }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use Point from shapes;\nfn main() { let p = Point { x: 1, y: 2 }; print(p.x); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();

        assert_eq!(merged.structs.len(), 1);
        assert_eq!(merged.structs[0].name.resolve().as_ref(), "shapes$Point");
        let main_fn = find_fn(&merged, "main");
        let Stmt::Let { value, .. } = &main_fn.body[0] else { panic!("expected Let") };
        let ExprKind::StructLit { name, .. } = &value.kind else { panic!("expected StructLit") };
        assert_eq!(name.resolve().as_ref(), "shapes$Point");
    }

    #[test]
    fn a_bare_use_qualified_call_resolves_to_the_source_modules_qualified_name() {
        let dir = scratch_dir("merge_bare_use_qualified_call");
        fs::write(dir.join("geometry.kes"), "pub pure fn square(x: i64) -> i64 { return x * x; }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use geometry;\nfn main() { print(geometry.square(7)); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();

        let square_fn = find_fn(&merged, "geometry$square");
        let main_fn = find_fn(&merged, "main");
        let Stmt::Print { args, .. } = &main_fn.body[0] else { panic!("expected Print") };
        let ExprKind::Call { name, .. } = &args[0].kind else { panic!("expected Call") };
        assert_eq!(*name, square_fn.name);
    }

    #[test]
    fn a_from_import_of_a_private_function_is_a_distinct_compile_error() {
        let dir = scratch_dir("merge_from_private_fn");
        fs::write(dir.join("geometry.kes"), "fn square(x: i64) -> i64 { return x * x; }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use square from geometry;\nfn main() { print(square(7)); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let err = merge_modules(&entry, discovered).unwrap_err();
        assert!(err.message.contains("private"), "got: {}", err.message);
        assert!(!err.message.contains("not found"), "private access should not read as \"not found\", got: {}", err.message);
    }

    #[test]
    fn a_bare_use_qualified_call_to_a_private_function_is_a_distinct_compile_error() {
        let dir = scratch_dir("merge_bare_use_private_fn");
        fs::write(dir.join("geometry.kes"), "fn square(x: i64) -> i64 { return x * x; }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use geometry;\nfn main() { print(geometry.square(7)); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let err = merge_modules(&entry, discovered).unwrap_err();
        assert!(err.message.contains("private"), "got: {}", err.message);
    }

    #[test]
    fn a_from_import_of_a_private_struct_is_a_distinct_compile_error() {
        let dir = scratch_dir("merge_from_private_struct");
        fs::write(dir.join("shapes.kes"), "struct Point { x: i64, y: i64 }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use Point from shapes;\nfn main() { let p = Point { x: 1, y: 2 }; print(p.x); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let err = merge_modules(&entry, discovered).unwrap_err();
        assert!(err.message.contains("private"), "got: {}", err.message);
    }

    #[test]
    fn a_pub_function_is_importable_via_from_import() {
        let dir = scratch_dir("merge_pub_fn_from_import");
        fs::write(dir.join("geometry.kes"), "pub fn square(x: i64) -> i64 { return x * x; }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use square from geometry;\nfn main() { print(square(7)); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();
        find_fn(&merged, "geometry$square");
    }

    #[test]
    fn a_pub_function_is_importable_via_bare_use_qualified_call() {
        let dir = scratch_dir("merge_pub_fn_qualified_call");
        fs::write(dir.join("geometry.kes"), "pub fn square(x: i64) -> i64 { return x * x; }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use geometry;\nfn main() { print(geometry.square(7)); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();
        find_fn(&merged, "geometry$square");
    }

    #[test]
    fn a_private_function_is_still_fully_usable_within_its_own_module() {
        let dir = scratch_dir("merge_private_same_module_ok");
        fs::write(dir.join("dummy.kes"), "fn noop() {}").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use dummy;\nfn helper() { print(1); }\nfn main() { helper(); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();
        find_fn(&merged, "main$helper");
    }
}
