pub mod diag;
pub mod lexer;
mod newlines;
pub mod span;
pub mod token;

pub use diag::{render, render_short, Diagnostic, Severity};
pub use span::{LineIndex, Span};
