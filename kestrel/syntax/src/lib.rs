pub mod diag;
pub mod span;

pub use diag::{render, render_short, Diagnostic, Severity};
pub use span::{LineIndex, Span};
