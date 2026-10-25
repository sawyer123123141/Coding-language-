//! The cost lens in the editor: an inlay hint at the end of each line the report has something
//! to say about, and the full list in the line's hover.

use lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip, MarkupContent, MarkupKind, Range};

use crate::docs::Doc;
use crate::pos::LineMap;

/// A hint's label is cut to this many characters; its tooltip has everything.
const LABEL_CHARS: usize = 90;

/// The hints for the lines of `doc` inside `range`.
pub fn inlay_hints(doc: &Doc, range: Range) -> Vec<InlayHint> {
    let map = LineMap::new(&doc.text);
    let mut out = Vec::new();
    for l in doc.lens.iter().filter(|l| range.start.line <= l.line && l.line <= range.end.line) {
        let Some((_, end)) = map.line_bounds(l.line as usize) else { continue };
        let all: Vec<String> = l.items.iter().map(|i| format!("{}: {}", i.label, i.text)).collect();
        let mut label = all.join("; ");
        if label.chars().count() > LABEL_CHARS {
            label = label.chars().take(LABEL_CHARS - 1).collect::<String>() + "…";
        }
        out.push(InlayHint {
            position: map.position(end),
            label: InlayHintLabel::String(label),
            kind: Some(InlayHintKind::PARAMETER),
            text_edits: None,
            tooltip: Some(InlayHintTooltip::MarkupContent(MarkupContent { kind: MarkupKind::Markdown, value: list(doc, l.line).unwrap_or_default() })),
            padding_left: Some(true),
            padding_right: None,
            data: None,
        });
    }
    out
}

/// The cost lens's items on 0-based line `line`, as a Markdown list, for hover.
pub fn hover_text(doc: &Doc, line: u32) -> Option<String> {
    list(doc, line).map(|l| format!("**Cost** (at the last save)\n\n{l}"))
}

fn list(doc: &Doc, line: u32) -> Option<String> {
    let l = doc.lens.iter().find(|l| l.line == line)?;
    let items: Vec<String> = l.items.iter().map(|i| format!("- {}", i.sentence())).collect();
    Some(items.join("\n"))
}
