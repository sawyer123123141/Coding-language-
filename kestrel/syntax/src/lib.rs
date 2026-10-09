pub mod ast;
pub mod diag;
pub mod lexer;
mod newlines;
pub mod parser;
mod sexpr;
pub mod span;
pub mod token;

pub use diag::{render, render_short, Diagnostic, Severity};
pub use span::{LineIndex, Span};

/// Lex and parse one source file. Diagnostics from both stages, in order.
pub fn parse(src: &str) -> (ast::Program, Vec<Diagnostic>) {
    let (toks, mut diags) = lexer::lex(src);
    let (program, parse_diags) = parser::Parser::new(toks).parse_program();
    diags.extend(parse_diags);
    (program, diags)
}
