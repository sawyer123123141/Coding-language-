//! Primary expressions: literals, names, calls, brackets, and interpolated strings.

use super::{PResult, Parser};
use crate::ast::{Arg, Expr, ExprKind, InterpPart};
use crate::span::Span;
use crate::token::{StrPart, Tok, Token};

impl Parser {
    /// Parses the code inside `{...}` of an interpolated string. `start` is
    /// its byte offset in the file, so spans point into the string.
    fn interpolated(&mut self, text: &str, start: u32) -> PResult<Expr> {
        let shift = |s: Span| Span { start: s.start + start, end: s.end + start };
        let (mut toks, diags) = crate::lexer::lex(text);
        for d in diags {
            self.error(shift(d.span), d.message);
        }
        for t in &mut toks {
            t.span = shift(t.span);
        }
        // The expression must end at the `}`: give the token stream one to stop at.
        let close = Span::new((start as usize) + text.len(), (start as usize) + text.len() + 1);
        let eof = toks.pop().expect("lex always ends with Eof");
        toks.push(Token { tok: Tok::RBrace, span: close });
        toks.push(eof);
        let saved = (std::mem::replace(&mut self.toks, toks), std::mem::replace(&mut self.pos, 0));
        // The string's tokens are numbered from 0 again: the matches found in the file's don't apply.
        let closes = std::mem::take(&mut self.closes);
        let result = self.inside_brackets(|p| {
            p.nested(|p| {
                let e = p.expr()?;
                p.expect(&Tok::RBrace, "`}` after the expression")?;
                Ok(e)
            })
        });
        self.closes = closes;
        (self.toks, self.pos) = saved;
        if self.stopped() {
            // Too deep inside the string: the rest of the file is skipped too.
            self.pos = self.toks.len() - 1;
        }
        result
    }

    pub(super) fn primary(&mut self) -> PResult<Expr> {
        let span = self.span();
        let kind = match self.peek().clone() {
            Tok::Int(n) | Tok::Hex(n) => ExprKind::Int(n),
            Tok::Float(f) => ExprKind::Float(f),
            Tok::Str(s) => ExprKind::Str(s),
            Tok::StrParts(parts) => {
                self.bump();
                let mut out = Vec::with_capacity(parts.len());
                for part in parts {
                    out.push(match part {
                        StrPart::Lit(s) => InterpPart::Lit(s),
                        StrPart::Code { text, start } => InterpPart::Expr(self.interpolated(&text, start)?),
                    });
                }
                return Ok(self.expr_node(ExprKind::Interp(out), span));
            }
            Tok::True => ExprKind::Bool(true),
            Tok::False => ExprKind::Bool(false),
            Tok::NoneKw => ExprKind::None,
            Tok::Ident(name) => {
                self.bump();
                if self.at(&Tok::LParen) {
                    return self.call(name, span);
                }
                // The built-ins whose brackets hold types: `list[int]()`, `from_bytes[Game](b)`.
                if matches!(name.as_str(), "array" | "list" | "map" | "from_bytes") && self.at(&Tok::LBracket) {
                    return self.type_call(name, span);
                }
                return Ok(self.expr_node(ExprKind::Ident(name), span));
            }
            Tok::LParen => {
                if self.starts_closure() {
                    return self.closure();
                }
                return self.nested(|p| {
                    p.bump();
                    let mut inner = p.inside_brackets(Self::expr)?;
                    let end = p.expect(&Tok::RParen, "`)`")?;
                    inner.span = span.to(end);
                    Ok(inner)
                });
            }
            Tok::LBrace => {
                let (entries, end) = self.nested(Self::map_entries)?;
                return Ok(self.expr_node(ExprKind::MapLit(entries), span.to(end)));
            }
            Tok::LBracket => {
                let (items, end) = self.nested(|p| {
                    p.bump();
                    let mut items = Vec::new();
                    while !p.at(&Tok::RBracket) {
                        items.push(p.inside_brackets(Self::expr)?);
                        if !p.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    Ok((items, p.expect(&Tok::RBracket, "`]` or `,`")?))
                })?;
                return Ok(self.expr_node(ExprKind::ArrayLit(items), span.to(end)));
            }
            Tok::SelfKw => {
                self.bump();
                return Ok(self.expr_node(ExprKind::Ident("self".to_string()), span));
            }
            Tok::At => return self.annot_in_expr(),
            Tok::If => return self.if_expr(),
            Tok::Match => return self.match_expr(),
            Tok::Pipe => {
                self.pipe_closure();
                return Err(());
            }
            other => {
                self.error_here(format!("expected an expression, found {}", other.describe()));
                return Err(());
            }
        };
        let hex = matches!(self.peek(), Tok::Hex(_));
        self.bump();
        let e = self.expr_node(kind, span);
        if hex {
            self.hex_literals.push(e.id);
        }
        Ok(e)
    }

    fn call(&mut self, name: String, name_span: Span) -> PResult<Expr> {
        let (args, end) = self.call_args()?;
        Ok(self.expr_node(ExprKind::Call { name, name_span, args }, name_span.to(end)))
    }

    /// `{k: v, ...}`. A `{` never starts a block expression, so here it's a
    /// map. Newlines inside it don't end anything.
    /// Returns the entries and the span of the closing `}`.
    fn map_entries(&mut self) -> PResult<(Vec<(Expr, Expr)>, Span)> {
        self.bump();
        self.skip_newlines();
        let mut entries = Vec::new();
        while !self.at(&Tok::RBrace) {
            let key = self.inside_brackets(Self::expr)?;
            self.expect(&Tok::Colon, "`:`")?;
            let value = self.inside_brackets(Self::expr)?;
            entries.push((key, value));
            self.skip_newlines();
            if !self.eat(&Tok::Comma) {
                break;
            }
            self.skip_newlines();
        }
        let end = self.expect(&Tok::RBrace, "`}` or `,`")?;
        Ok((entries, end))
    }

    /// `array[int](n)`: type arguments in brackets, then call arguments.
    fn type_call(&mut self, name: String, name_span: Span) -> PResult<Expr> {
        self.expect(&Tok::LBracket, "`[`")?;
        let mut type_args = Vec::new();
        loop {
            type_args.push(self.type_expr()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RBracket, "`]`")?;
        if !self.at(&Tok::LParen) {
            if name == "from_bytes" {
                self.error_here("expected `(` with the bytes to read, like `from_bytes[Game](bytes)`");
                return Err(());
            }
            self.error_here(format!("expected `(` to create the {name}, like `{name}[...]()`"));
            return Err(());
        }
        let (args, end) = self.call_args()?;
        Ok(self.expr_node(ExprKind::TypeCall { name, name_span, type_args, args }, name_span.to(end)))
    }

    /// `(arg, ...)`; returns the arguments and the span of the closing `)`.
    pub(super) fn call_args(&mut self) -> PResult<(Vec<Arg>, Span)> {
        self.nested(Self::call_args_inside)
    }

    fn call_args_inside(&mut self) -> PResult<(Vec<Arg>, Span)> {
        self.expect(&Tok::LParen, "`(`")?;
        let mut args = Vec::new();
        while !self.at(&Tok::RParen) {
            args.push(self.arg()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        let end = self.expect(&Tok::RParen, "`)` or `,`")?;
        Ok((args, end))
    }

    fn arg(&mut self) -> PResult<Arg> {
        let start = self.span();
        let mut label = None;
        if self.peek_at(1) == &Tok::Colon {
            if let Tok::Ident(name) = self.peek().clone() {
                self.bump();
                self.bump();
                label = Some(name);
            }
        }
        let is_var = self.eat(&Tok::Var);
        let value = self.inside_brackets(Self::expr)?;
        Ok(Arg { label, is_var, span: start.to(value.span), value })
    }
}
