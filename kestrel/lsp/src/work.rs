//! Analysing one open document: checking its program (always), and lowering it for its
//! contracts and the cost lens (when it was opened or saved).

use std::any::Any;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

use kestrel_syntax::ast::Item;
use kestrel_syntax::Diagnostic;
use lsp_types::{Diagnostic as LspDiagnostic, DiagnosticSeverity, Url};

use crate::diagnostics::{by_file, code, Found};
use crate::docs::key;
use crate::lens::LensLine;
use crate::lower::lower;
use crate::pos::LineMap;
use crate::server::Server;

/// Analyses the document at `uri` (and, with `lower`, lowers it), keeping what it found in `s`.
/// True when it was lowered.
pub fn analyse(s: &mut Server, uri: &Url, lower_it: bool) -> bool {
    let Some(doc) = s.docs.get(uri) else { return false };
    let (path, text) = (doc.path.clone(), doc.text.clone());
    let k = key(&path);
    let needs_main = needs_main(&text);
    let read = |p: &std::path::Path| s.docs.read(p);
    let a = kestrel_driver::analyze(&path.to_string_lossy(), &text, needs_main, &read);
    let mut found = Found::new();
    by_file(&s.docs, &a.sources, &a.diags, &mut found);
    let clean = !a.diags.iter().any(Diagnostic::is_error);
    // The analysis is kept before lowering, so a panic there leaves it matching the text.
    let doc = s.docs.get_mut(uri).expect("still open");
    doc.dirty = false;
    // A file opened or saved with errors is lowered once they are fixed.
    doc.lower = lower_it && !clean;
    let has_tables = a.tables.is_some();
    let old = doc.analysis.replace(a);
    // Keep the last analysis with tables for while the text doesn't parse.
    if has_tables {
        doc.good = None;
    } else if let Some(old) = old.filter(|o| o.tables.is_some()) {
        doc.good = Some((old.sources[0].src.clone(), old));
    }
    s.checked.insert(k.clone(), found);
    if !clean {
        // What lowering found no longer holds.
        s.lowered.remove(&k);
        s.docs.get_mut(uri).expect("still open").lens.clear();
        return false;
    }
    if !lower_it {
        return false;
    }
    let read = |p: &std::path::Path| s.docs.read(p);
    let lowered = catch_unwind(AssertUnwindSafe(|| lower(&path, &text, needs_main, &read, s.config.checked, s.config.lens)));
    let mut found = Found::new();
    let mut lens = Vec::new();
    match lowered {
        Ok(Some(l)) => {
            by_file(&s.docs, &l.sources, &l.diags, &mut found);
            lens = l.lens;
        }
        Ok(None) => {}
        Err(panic) => {
            found.insert(k.clone(), (uri.clone(), vec![ice(&*panic)]));
        }
    }
    if s.config.diagnostics() {
        let lens_diags = lens_diagnostics(&text, &lens);
        if !lens_diags.is_empty() {
            found.entry(k.clone()).or_insert_with(|| (uri.clone(), Vec::new())).1.extend(lens_diags);
        }
    }
    s.lowered.insert(k, found);
    s.docs.get_mut(uri).expect("still open").lens = lens;
    true
}

/// The error for a compiler panic.
fn ice(panic: &(dyn Any + Send)) -> LspDiagnostic {
    let why = panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "a panic".to_string());
    LspDiagnostic {
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("kestrel".into()),
        message: format!("internal compiler error: {why}; please report it with this file"),
        ..LspDiagnostic::default()
    }
}

/// When checking panicked: the document gets one error saying so, and hover answers from the
/// last analysis that had tables (moved past the edits since).
pub fn crashed(s: &mut Server, uri: &Url, panic: Box<dyn Any + Send>) -> bool {
    let Some(doc) = s.docs.get_mut(uri) else { return false };
    doc.dirty = false;
    doc.lower = false;
    if let Some(old) = doc.analysis.take().filter(|o| o.tables.is_some()) {
        doc.good = Some((old.sources[0].src.clone(), old));
    }
    doc.lens.clear();
    let k = key(&doc.path);
    s.checked.insert(k.clone(), HashMap::from([(k.clone(), (uri.clone(), vec![ice(&*panic)]))]));
    s.lowered.remove(&k);
    false
}

/// Whether the file is checked as a program: unless it has no `main` and has a `pub` item,
/// which makes it a module (another file's `use` reads it).
pub fn needs_main(text: &str) -> bool {
    let (p, _) = kestrel_syntax::parse(text);
    let main = p.items.iter().any(|i| matches!(i, Item::Fn(f) if f.name == "main"));
    let public = p.items.iter().any(|i| match i {
        Item::Fn(f) => f.is_pub,
        Item::Const(c) => c.is_pub,
        Item::Struct(s) => s.is_pub,
        Item::Enum(e) => e.is_pub,
        Item::Interface(i) => i.is_pub,
        Item::Use(_) => false,
    });
    main || !public
}

/// The cost lens as Information diagnostics, one per item, over the line's code.
fn lens_diagnostics(text: &str, lens: &[LensLine]) -> Vec<LspDiagnostic> {
    let map = LineMap::new(text);
    let mut out = Vec::new();
    for l in lens {
        let Some((start, end)) = map.line_bounds(l.line as usize) else { continue };
        // From the first non-blank character, so the squiggle sits under the code.
        let indent = text[start..end].len() - text[start..end].trim_start().len();
        let range = map.range(start + indent, end);
        for item in &l.items {
            out.push(LspDiagnostic {
                range,
                severity: Some(DiagnosticSeverity::INFORMATION),
                source: Some("kestrel cost".into()),
                code: code("cost"),
                message: item.sentence(),
                ..LspDiagnostic::default()
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::needs_main;

    #[test]
    fn a_file_with_pub_items_and_no_main_is_a_module() {
        assert!(!needs_main("pub area(w: int) -> int {\n    w * w\n}\n"));
        assert!(needs_main("area(w: int) -> int {\n    w * w\n}\n"));
        assert!(needs_main("pub area(w: int) -> int {\n    w * w\n}\nmain() {}\n"));
    }
}
