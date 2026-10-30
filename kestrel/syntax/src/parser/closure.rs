//! Closure literals: `(x: int) -> int { x + 1 }`, a function with no name.
//!
//! In expression position, a `(` whose matching `)` is followed by `{` or `->` starts a closure.
//! The one exception is the head of an `if`, `while`, `for`, `match` or `in` (`no_closure_head`):
//! there `(cond) {` stays a parenthesised expression and its block, so `if (ready) { go() }`
//! keeps working. Inside brackets, parentheses, a map's braces and a block (a closure's body
//! included) the exception is lifted again, so a closure in a head goes inside a call's
//! parentheses: `if xs.any((e) { e.dead }) { ... }`.

use super::{PResult, Parser};
use crate::ast::{ClosureLit, ClosureParam, Expr, ExprKind};
use crate::span::Span;
use crate::token::Tok;

impl Parser {
    /// Whether the `(` at the current token starts a closure literal.
    pub(super) fn starts_closure(&mut self) -> bool {
        if self.no_closure_head {
            return false;
        }
        let Some(close) = self.close_of(self.pos) else { return false };
        matches!(self.toks.get(close + 1).map(|t| &t.tok), Some(Tok::LBrace | Tok::Arrow))
    }

    /// The index of the bracket that closes the one at `open`. One scan records the match of
    /// every bracket it closes along the way, so a pile of nested parentheses is scanned once.
    pub(super) fn close_of(&mut self, open: usize) -> Option<usize> {
        if let Some(&close) = self.closes.get(&open) {
            return Some(close);
        }
        let mut stack = Vec::new();
        for i in open..self.toks.len() {
            match self.toks[i].tok {
                Tok::LParen | Tok::LBracket | Tok::LBrace => stack.push(i),
                Tok::RParen | Tok::RBracket | Tok::RBrace => {
                    let from = stack.pop()?;
                    self.closes.insert(from, i);
                    if from == open {
                        return Some(i);
                    }
                }
                Tok::Eof => return None,
                _ => {}
            }
        }
        None
    }

    /// Runs `f` for the head of a statement or expression that is followed by a block.
    pub(super) fn in_head<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.closure_head(true, f)
    }

    /// Runs `f` inside a bracket or block, where a closure may start again.
    pub(super) fn inside_brackets<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.closure_head(false, f)
    }

    fn closure_head<T>(&mut self, head: bool, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = std::mem::replace(&mut self.no_closure_head, head);
        // Inside brackets a `..` is no longer the loop's own range (see `habits`).
        let range_at = if head { None } else { Some(std::mem::take(&mut self.range_at)) };
        let result = f(self);
        self.no_closure_head = saved;
        if let Some(r) = range_at {
            self.range_at = r;
        }
        result
    }

    /// `(params) [-> Type] { body }`, at its `(`. Kept out of line: `primary` is on the stack once
    /// per level of nesting.
    #[inline(never)]
    pub(super) fn closure(&mut self) -> PResult<Expr> {
        let start = self.span();
        let (lit, inner, end) = self.nested(Self::closure_parts)?;
        Ok(self.expr_node_holding(ExprKind::Closure(Box::new(lit)), start.to(end), inner))
    }

    /// The closure, the height of its body, and the span of its last token.
    fn closure_parts(&mut self) -> PResult<(ClosureLit, u32, Span)> {
        self.expect(&Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        while !self.at(&Tok::RParen) {
            params.push(self.closure_param()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RParen, "`)` or `,`")?;
        let ret = if self.eat(&Tok::Arrow) { Some(self.type_expr()?) } else { None };
        let (body, height) = self.block_with_height()?;
        let end = body.span;
        Ok((ClosureLit { params, ret, body }, height, end))
    }

    /// `x`, `x: Type`, `var x` or `var x: Type`: the type may be left out where the context gives it.
    fn closure_param(&mut self) -> PResult<ClosureParam> {
        let is_var = self.eat(&Tok::Var);
        if self.at(&Tok::SelfKw) {
            self.error_here("a closure has no `self` parameter; it can use the `self` of the method around it");
            return Err(());
        }
        let (name, span) = self.expect_ident("a parameter name")?;
        let bind = self.binding(name, span);
        let ty = if self.eat(&Tok::Colon) { Some(self.type_expr()?) } else { None };
        Ok(ClosureParam { bind, is_var, ty })
    }
}
