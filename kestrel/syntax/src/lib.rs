pub mod ast;
pub mod diag;
pub mod lexer;
pub mod link;
mod newlines;
pub mod parser;
mod sexpr;
pub mod span;
pub mod token;

pub use diag::{render, render_short, Diagnostic, Severity};
pub use span::{LineIndex, Span};

/// Lex and parse one source file. Diagnostics from both stages, in order.
pub fn parse(src: &str) -> (ast::Program, Vec<Diagnostic>) {
    parse_at(src, 0, 0, 0)
}

/// Parses one file of several: its spans start at byte `base` and its ids
/// at `next_expr` / `next_bind`, so files can be merged into one program.
pub fn parse_at(src: &str, base: u32, next_expr: u32, next_bind: u32) -> (ast::Program, Vec<Diagnostic>) {
    let (mut toks, mut diags) = lexer::lex(src);
    if base != 0 {
        let shift = |s: &mut Span| {
            s.start += base;
            s.end += base;
        };
        for t in &mut toks {
            shift(&mut t.span);
            if let token::Tok::StrParts(parts) = &mut t.tok {
                for p in parts {
                    if let token::StrPart::Code { start, .. } = p {
                        *start += base;
                    }
                }
            }
        }
        for d in &mut diags {
            shift(&mut d.span);
        }
    }
    let (program, parse_diags) = parser::Parser::with_ids(toks, next_expr, next_bind).parse_program();
    diags.extend(parse_diags);
    (program, diags)
}
