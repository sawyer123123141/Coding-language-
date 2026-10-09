mod expr;
mod item;
mod stmt;

use crate::ast::{BindId, Binding, Expr, ExprId, ExprKind, Program};
use crate::diag::Diagnostic;
use crate::span::Span;
use crate::token::{Tok, Token};

pub(crate) type PResult<T> = Result<T, ()>;

pub struct Parser<'t> {
    toks: &'t [Token],
    pos: usize,
    diags: Vec<Diagnostic>,
    next_expr: u32,
    next_bind: u32,
}

impl<'t> Parser<'t> {
    /// `toks` must end with `Tok::Eof` (as `lex` guarantees).
    pub fn new(toks: &'t [Token]) -> Self {
        Parser { toks, pos: 0, diags: Vec::new(), next_expr: 0, next_bind: 0 }
    }

    pub fn parse_program(mut self) -> (Program, Vec<Diagnostic>) {
        let items = self.items();
        let program = Program { items, expr_count: self.next_expr, bind_count: self.next_bind };
        (program, self.diags)
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    /// Consumes the current token. Never moves past `Eof`.
    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at(&self, t: &Tok) -> bool {
        self.peek() == t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: &Tok, what: &str) -> PResult<Span> {
        if self.at(t) {
            Ok(self.bump().span)
        } else {
            let found = self.peek().describe();
            self.error_here(format!("expected {what}, found {found}"));
            Err(())
        }
    }

    fn expect_ident(&mut self, what: &str) -> PResult<(String, Span)> {
        if let Tok::Ident(name) = self.peek().clone() {
            let span = self.bump().span;
            Ok((name, span))
        } else {
            let found = self.peek().describe();
            self.error_here(format!("expected {what}, found {found}"));
            Err(())
        }
    }

    fn skip_newlines(&mut self) {
        while self.at(&Tok::Newline) || self.at(&Tok::Semi) {
            self.bump();
        }
    }

    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, msg));
    }

    fn error_here(&mut self, msg: impl Into<String>) {
        let span = self.span();
        self.error(span, msg);
    }

    fn expr_node(&mut self, kind: ExprKind, span: Span) -> Expr {
        let id = ExprId(self.next_expr);
        self.next_expr += 1;
        Expr { id, kind, span }
    }

    fn binding(&mut self, name: String, span: Span) -> Binding {
        let id = BindId(self.next_bind);
        self.next_bind += 1;
        Binding { id, name, span }
    }

    /// Skips to the end of the current statement: a newline or `;` at this
    /// nesting level, or the `}` that closes the enclosing block.
    fn recover_stmt(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::LBrace | Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RBrace if depth == 0 => return,
                Tok::RBrace | Tok::RParen | Tok::RBracket => depth = (depth - 1).max(0),
                Tok::Newline | Tok::Semi if depth == 0 => return,
                _ => {}
            }
            self.bump();
        }
    }

    /// Skips to the next top-level item: a newline outside any braces.
    fn recover_item(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::LBrace => depth += 1,
                Tok::RBrace => depth -= 1,
                Tok::Newline if depth <= 0 => return,
                _ => {}
            }
            self.bump();
        }
    }
}
