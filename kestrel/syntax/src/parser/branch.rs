//! `if` and `match`: expressions that hold blocks.

use super::{PResult, Parser};
use crate::ast::{Block, Expr, ExprKind, MatchArm, Pattern, Stmt};
use crate::span::Span;
use crate::token::Tok;

impl Parser {
    /// `match value { Variant { ... } Variant name { ... } else { ... } }`
    pub(super) fn match_expr(&mut self) -> PResult<Expr> {
        let (kind, span, inner) = self.nested(Self::match_parts)?;
        Ok(self.expr_node_holding(kind, span, inner))
    }

    /// A `match`, and the height of its tallest arm.
    fn match_parts(&mut self) -> PResult<(ExprKind, Span, u32)> {
        let start = self.expect(&Tok::Match, "`match`")?;
        let scrutinee = self.in_head(Self::expr)?;
        self.expect(&Tok::LBrace, "`{`")?;
        let mut arms = Vec::new();
        let mut inner = 0;
        loop {
            self.skip_newlines();
            if self.at(&Tok::RBrace) || self.at(&Tok::Eof) {
                break;
            }
            match self.match_arm() {
                Ok((arm, h)) => {
                    arms.push(arm);
                    inner = inner.max(h);
                }
                Err(()) => self.recover_stmt(),
            }
        }
        let end = self.expect(&Tok::RBrace, "`}`")?;
        Ok((ExprKind::Match { scrutinee: Box::new(scrutinee), arms }, start.to(end), inner))
    }

    /// An arm, and the height of its block.
    fn match_arm(&mut self) -> PResult<(MatchArm, u32)> {
        let start = self.span();
        let pattern = if self.eat(&Tok::Else) {
            Pattern::Else(start)
        } else {
            if matches!(self.peek(), Tok::Ident(n) if n == "_") {
                self.error_here("Kestrel writes the catch-all arm as `else { ... }`");
                return Err(());
            }
            let (name, name_span) = self.expect_ident("a variant name or `else`")?;
            let binding = match self.peek().clone() {
                Tok::Ident(b) => {
                    let span = self.bump().span;
                    Some(self.binding(b, span))
                }
                _ => None,
            };
            self.arm_spelling(&name, binding.as_ref().map(|b| b.name.as_str()))?;
            Pattern::Variant { name, name_span, binding }
        };
        let (body, h) = self.block_with_height()?;
        Ok((MatchArm { span: start.to(body.span), pattern, body }, h))
    }

    pub(crate) fn if_expr(&mut self) -> PResult<Expr> {
        let (kind, span, inner) = self.nested(Self::if_parts)?;
        Ok(self.expr_node_holding(kind, span, inner))
    }

    /// An `if`, and the height of its tallest block.
    fn if_parts(&mut self) -> PResult<(ExprKind, Span, u32)> {
        let start = self.expect(&Tok::If, "`if`")?;
        let cond = self.in_head(Self::expr)?;
        let (then_block, mut inner) = self.block_with_height()?;
        if matches!(self.peek(), Tok::Ident(k) if k == "elif") {
            self.elif();
            return Err(());
        }
        let else_block = if self.eat(&Tok::Else) {
            if self.at(&Tok::If) {
                let nested = self.if_expr()?;
                // The `else if` sits in a block of its own.
                inner = inner.max(self.height(&nested) + 1);
                let span = nested.span;
                Some(Block { stmts: vec![Stmt::Expr(nested)], span })
            } else {
                let (block, h) = self.block_with_height()?;
                inner = inner.max(h);
                Some(block)
            }
        } else {
            None
        };
        let end = else_block.as_ref().map_or(then_block.span, |b| b.span);
        Ok((ExprKind::If { cond: Box::new(cond), then_block, else_block }, start.to(end), inner))
    }
}
