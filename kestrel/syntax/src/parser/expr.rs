use super::{PResult, Parser};
use crate::ast::{Arg, BinaryOp, Block, Expr, ExprKind, InterpPart, MatchArm, Pattern, Stmt, UnaryOp};
use crate::span::Span;
use crate::token::{StrPart, Tok, Token};

fn cmp_op(t: &Tok) -> Option<BinaryOp> {
    Some(match t {
        Tok::EqEq => BinaryOp::Eq,
        Tok::NotEq => BinaryOp::Ne,
        Tok::Lt => BinaryOp::Lt,
        Tok::Gt => BinaryOp::Gt,
        Tok::Le => BinaryOp::Le,
        Tok::Ge => BinaryOp::Ge,
        _ => return None,
    })
}

impl Parser {
    pub(crate) fn expr(&mut self) -> PResult<Expr> {
        self.or_expr()
    }

    fn binary(&mut self, op: BinaryOp, lhs: Expr, rhs: Expr) -> Expr {
        let span = lhs.span.to(rhs.span);
        self.expr_node(ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span)
    }

    fn or_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.and_expr()?;
        while self.eat(&Tok::Or) {
            let rhs = self.and_expr()?;
            lhs = self.binary(BinaryOp::Or, lhs, rhs);
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.cmp_expr()?;
        while self.eat(&Tok::And) {
            let rhs = self.cmp_expr()?;
            lhs = self.binary(BinaryOp::And, lhs, rhs);
        }
        Ok(lhs)
    }

    fn cmp_expr(&mut self) -> PResult<Expr> {
        let lhs = self.add_expr()?;
        let Some(op) = cmp_op(self.peek()) else { return Ok(lhs) };
        self.bump();
        let rhs = self.add_expr()?;
        let e = self.binary(op, lhs, rhs);
        if cmp_op(self.peek()).is_some() {
            self.error_here("comparisons can't be chained; combine them with `and`");
            return Err(());
        }
        Ok(e)
    }

    fn add_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.mul_expr()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinaryOp::Add,
                Tok::Minus => BinaryOp::Sub,
                _ => return Ok(lhs),
            };
            self.bump();
            let rhs = self.mul_expr()?;
            lhs = self.binary(op, lhs, rhs);
        }
    }

    fn mul_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinaryOp::Mul,
                Tok::Slash => BinaryOp::Div,
                Tok::Percent => BinaryOp::Rem,
                _ => return Ok(lhs),
            };
            self.bump();
            let rhs = self.unary()?;
            lhs = self.binary(op, lhs, rhs);
        }
    }

    fn unary(&mut self) -> PResult<Expr> {
        let start = self.span();
        let op = match self.peek() {
            Tok::Minus => UnaryOp::Neg,
            Tok::Not => UnaryOp::Not,
            _ => return self.postfix(),
        };
        self.bump();
        let inner = self.unary()?;
        let span = start.to(inner.span);
        Ok(self.expr_node(ExprKind::Unary { op, expr: Box::new(inner) }, span))
    }

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
        let result = self.expr().and_then(|e| {
            self.expect(&Tok::RBrace, "`}` after the expression")?;
            Ok(e)
        });
        (self.toks, self.pos) = saved;
        result
    }

    /// A primary expression followed by any `.field` / `.method(args)`.
    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        while self.eat(&Tok::Dot) {
            let (name, name_span) = self.expect_ident("a field or method name")?;
            if self.at(&Tok::LParen) {
                let (args, end) = self.call_args()?;
                let span = e.span.to(end);
                let kind = ExprKind::MethodCall { receiver: Box::new(e), name, name_span, args };
                e = self.expr_node(kind, span);
            } else {
                let span = e.span.to(name_span);
                e = self.expr_node(ExprKind::Field { base: Box::new(e), name, name_span }, span);
            }
        }
        Ok(e)
    }

    fn primary(&mut self) -> PResult<Expr> {
        let span = self.span();
        let kind = match self.peek().clone() {
            Tok::Int(n) => ExprKind::Int(n),
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
            Tok::Ident(name) => {
                self.bump();
                if self.at(&Tok::LParen) {
                    return self.call(name, span);
                }
                return Ok(self.expr_node(ExprKind::Ident(name), span));
            }
            Tok::LParen => {
                self.bump();
                let mut inner = self.expr()?;
                let end = self.expect(&Tok::RParen, "`)`")?;
                inner.span = span.to(end);
                return Ok(inner);
            }
            Tok::SelfKw => {
                self.bump();
                return Ok(self.expr_node(ExprKind::Ident("self".to_string()), span));
            }
            Tok::If => return self.if_expr(),
            Tok::Match => return self.match_expr(),
            other => {
                self.error_here(format!("expected an expression, found {}", other.describe()));
                return Err(());
            }
        };
        self.bump();
        Ok(self.expr_node(kind, span))
    }

    fn call(&mut self, name: String, name_span: Span) -> PResult<Expr> {
        let (args, end) = self.call_args()?;
        Ok(self.expr_node(ExprKind::Call { name, name_span, args }, name_span.to(end)))
    }

    /// `(arg, ...)`; returns the arguments and the span of the closing `)`.
    fn call_args(&mut self) -> PResult<(Vec<Arg>, Span)> {
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
        let value = self.expr()?;
        Ok(Arg { label, is_var, span: start.to(value.span), value })
    }

    /// `match value { Variant { ... } Variant name { ... } else { ... } }`
    fn match_expr(&mut self) -> PResult<Expr> {
        let start = self.expect(&Tok::Match, "`match`")?;
        let scrutinee = self.expr()?;
        self.expect(&Tok::LBrace, "`{`")?;
        let mut arms = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&Tok::RBrace) || self.at(&Tok::Eof) {
                break;
            }
            match self.match_arm() {
                Ok(arm) => arms.push(arm),
                Err(()) => self.recover_stmt(),
            }
        }
        let end = self.expect(&Tok::RBrace, "`}`")?;
        Ok(self.expr_node(ExprKind::Match { scrutinee: Box::new(scrutinee), arms }, start.to(end)))
    }

    fn match_arm(&mut self) -> PResult<MatchArm> {
        let start = self.span();
        let pattern = if self.eat(&Tok::Else) {
            Pattern::Else(start)
        } else {
            let (name, name_span) = self.expect_ident("a variant name or `else`")?;
            let binding = match self.peek().clone() {
                Tok::Ident(b) => {
                    let span = self.bump().span;
                    Some(self.binding(b, span))
                }
                _ => None,
            };
            Pattern::Variant { name, name_span, binding }
        };
        let body = self.block()?;
        Ok(MatchArm { span: start.to(body.span), pattern, body })
    }

    pub(crate) fn if_expr(&mut self) -> PResult<Expr> {
        let start = self.expect(&Tok::If, "`if`")?;
        let cond = self.expr()?;
        let then_block = self.block()?;
        let else_block = if self.eat(&Tok::Else) {
            if self.at(&Tok::If) {
                let nested = self.if_expr()?;
                let span = nested.span;
                Some(Block { stmts: vec![Stmt::Expr(nested)], span })
            } else {
                Some(self.block()?)
            }
        } else {
            None
        };
        let end = else_block.as_ref().map_or(then_block.span, |b| b.span);
        let kind = ExprKind::If { cond: Box::new(cond), then_block, else_block };
        Ok(self.expr_node(kind, start.to(end)))
    }
}
