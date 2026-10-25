//! The compiler's diagnostics as LSP ones, file by file. A program's diagnostics can fall in any
//! of its files, and a file can be part of several open programs (a module used by two), so each
//! open document's findings are kept apart and a file shows them all, each once.

use std::collections::HashMap;

use kestrel_driver::Source;
use kestrel_syntax::{Diagnostic, Severity, Span};
use lsp_types::{Diagnostic as LspDiagnostic, DiagnosticSeverity, NumberOrString, Url};

use crate::docs::{key, url_of, Docs};
use crate::pos::LineMap;

/// What one open document's program found, by file (the path's `docs::key`).
pub type Found = HashMap<String, (Url, Vec<LspDiagnostic>)>;

/// Every diagnostic of `diags` (spans in the shared space of `sources`), by file. Built-in
/// modules' files are left out. `code` names what found it ("kestrel" for the checker).
pub fn by_file(docs: &Docs, sources: &[Source], diags: &[Diagnostic], out: &mut Found) {
    for d in diags {
        let Some(s) = sources.iter().rev().find(|s| s.base <= d.span.start) else { continue };
        if s.builtin {
            continue;
        }
        let path = std::path::Path::new(&s.path);
        let Some(url) = url_of(docs, path) else { continue };
        let local = Span { start: d.span.start - s.base, end: d.span.end.saturating_sub(s.base).max(d.span.start - s.base) };
        let entry = out.entry(key(path)).or_insert_with(|| (url, Vec::new()));
        entry.1.push(to_lsp(&s.src, local, d));
    }
}

/// One diagnostic, its span local to the file with text `src`. The help line follows the message.
pub fn to_lsp(src: &str, span: Span, d: &Diagnostic) -> LspDiagnostic {
    let lines = LineMap::new(src);
    let severity = match d.severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
    };
    let message = match &d.help {
        Some(help) => format!("{}\nhelp: {help}", d.message),
        None => d.message.clone(),
    };
    LspDiagnostic {
        range: lines.range(span.start as usize, span.end as usize),
        severity: Some(severity),
        source: Some("kestrel".into()),
        message,
        ..LspDiagnostic::default()
    }
}

/// Everything shown for one file: each finder's diagnostics, without repeats, in order.
pub fn merged<'a>(file: &str, found: impl Iterator<Item = &'a Found>) -> Vec<LspDiagnostic> {
    let mut out: Vec<LspDiagnostic> = Vec::new();
    for (_, diags) in found.filter_map(|f| f.get(file)) {
        for d in diags {
            if !out.iter().any(|o| same(o, d)) {
                out.push(d.clone());
            }
        }
    }
    out
}

fn same(a: &LspDiagnostic, b: &LspDiagnostic) -> bool {
    a.range == b.range && a.message == b.message && a.severity == b.severity && a.code == b.code
}

/// A diagnostic's code, telling the cost lens's apart from the checker's.
pub fn code(name: &str) -> Option<NumberOrString> {
    Some(NumberOrString::String(name.to_string()))
}

#[cfg(test)]
mod tests {
    use kestrel_syntax::{Diagnostic, Span};
    use lsp_types::{DiagnosticSeverity, Position};

    use super::to_lsp;

    #[test]
    fn a_diagnostic_keeps_its_range_and_help() {
        let src = "main() {\n    let é = true + 1\n}\n";
        let start = src.find("true").unwrap() as u32;
        let d = Diagnostic::error(Span { start, end: start + 8 }, "can't add").with_help("write a number");
        let l = to_lsp(src, d.span, &d);
        assert_eq!(l.range.start, Position::new(1, 12));
        assert_eq!(l.range.end, Position::new(1, 20));
        assert_eq!(l.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(l.message, "can't add\nhelp: write a number");
    }
}
