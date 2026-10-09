use crate::span::{LineIndex, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Error, message: message.into(), span, help: None }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Warning, message: message.into(), span, help: None }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

fn label(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// One line, stable format used by tests: `LINE:COL: error: MESSAGE`.
pub fn render_short(src: &str, d: &Diagnostic) -> String {
    let (line, col) = LineIndex::new(src).line_col(src, d.span.start);
    format!("{line}:{col}: {}: {}", label(d.severity), d.message)
}

/// Full human-readable form with source line and caret underline.
pub fn render(src: &str, path: &str, d: &Diagnostic) -> String {
    let idx = LineIndex::new(src);
    let (line, col) = idx.line_col(src, d.span.start);
    let text = idx.line_text(src, line);
    let gutter = line.to_string().len() + 1;
    let pad = " ".repeat(gutter);
    // Underline to the span end, clipped to this line, at least one caret.
    let span_chars = src[d.span.start as usize..(d.span.end as usize).min(src.len())]
        .chars()
        .take_while(|&c| c != '\n')
        .count();
    let carets = "^".repeat(span_chars.max(1));
    let mut out = format!(
        "{}: {}\n{pad}--> {path}:{line}:{col}\n{pad} |\n{:>w$} | {text}\n{pad} | {}{carets}\n",
        label(d.severity),
        d.message,
        line,
        " ".repeat(col as usize - 1),
        w = gutter,
    );
    if let Some(help) = &d.help {
        out.push_str(&format!("{pad} = help: {help}\n"));
    }
    out
}
