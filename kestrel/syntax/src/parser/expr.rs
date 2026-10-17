use super::{PResult, Parser};
use crate::ast::{Arg, BinaryOp, Expr, ExprKind, InterpPart, UnaryOp};
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
        let lhs = self.bit_or()?;
        let Some(op) = cmp_op(self.peek()) else { return Ok(lhs) };
        self.bump();
        let rhs = self.bit_or()?;
        let e = self.binary(op, lhs, rhs);
        if cmp_op(self.peek()).is_some() {
            self.error_here("comparisons can't be chained; combine them with `and`");
            return Err(());
        }
        Ok(e)
    }

    /// `|`, then `^`, then `&`, then shifts, each binding tighter (as in GDScript).
    fn bit_or(&mut self) -> PResult<Expr> {
        self.left_assoc(&[(Tok::Pipe, BinaryOp::BitOr)], Self::bit_xor)
    }

    fn bit_xor(&mut self) -> PResult<Expr> {
        self.left_assoc(&[(Tok::Caret, BinaryOp::BitXor)], Self::bit_and)
    }

    fn bit_and(&mut self) -> PResult<Expr> {
        self.left_assoc(&[(Tok::Amp, BinaryOp::BitAnd)], Self::shift)
    }

    fn shift(&mut self) -> PResult<Expr> {
        self.left_assoc(&[(Tok::Shl, BinaryOp::Shl), (Tok::Shr, BinaryOp::Shr)], Self::add_expr)
    }

    /// `operand (op operand)*` for the operators in `ops`.
    fn left_assoc(&mut self, ops: &[(Tok, BinaryOp)], operand: fn(&mut Self) -> PResult<Expr>) -> PResult<Expr> {
        let mut lhs = operand(self)?;
        while let Some(&(_, op)) = ops.iter().find(|(t, _)| self.at(t)) {
            self.bump();
            let rhs = operand(self)?;
            lhs = self.binary(op, lhs, rhs);
        }
        Ok(lhs)
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
            Tok::Tilde => UnaryOp::BitNot,
            _ => return self.postfix(),
        };
        self.bump();
        let inner = self.nested(Self::unary)?;
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
        let result = self.nested(|p| {
            let e = p.expr()?;
            p.expect(&Tok::RBrace, "`}` after the expression")?;
            Ok(e)
        });
        (self.toks, self.pos) = saved;
        if self.stopped() {
            // Too deep inside the string: the rest of the file is skipped too.
            self.pos = self.toks.len() - 1;
        }
        result
    }

    /// A primary expression followed by any `.field` / `.method(args)` / `[index]`.
    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.at(&Tok::LBracket) {
                let (index, end) = self.nested(|p| {
                    p.bump();
                    let index = p.expr()?;
                    Ok((index, p.expect(&Tok::RBracket, "`]`")?))
                })?;
                let span = e.span.to(end);
                e = self.expr_node(ExprKind::Index { base: Box::new(e), index: Box::new(index) }, span);
                continue;
            }
            if !self.eat(&Tok::Dot) {
                break;
            }
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
            Tok::NoneKw => ExprKind::None,
            Tok::Ident(name) => {
                self.bump();
                if self.at(&Tok::LParen) {
                    return self.call(name, span);
                }
                if matches!(name.as_str(), "array" | "list" | "map") && self.at(&Tok::LBracket) {
                    return self.type_call(name, span);
                }
                return Ok(self.expr_node(ExprKind::Ident(name), span));
            }
            Tok::LParen => {
                return self.nested(|p| {
                    p.bump();
                    let mut inner = p.expr()?;
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
                        items.push(p.expr()?);
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

    /// `{k: v, ...}`. A `{` never starts a block expression, so here it's a
    /// map. Newlines inside it don't end anything.
    /// Returns the entries and the span of the closing `}`.
    fn map_entries(&mut self) -> PResult<(Vec<(Expr, Expr)>, Span)> {
        self.bump();
        self.skip_newlines();
        let mut entries = Vec::new();
        while !self.at(&Tok::RBrace) {
            let key = self.expr()?;
            self.expect(&Tok::Colon, "`:`")?;
            let value = self.expr()?;
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
            self.error_here(format!("expected `(` to create the {name}, like `{name}[...]()`"));
            return Err(());
        }
        let (args, end) = self.call_args()?;
        Ok(self.expr_node(ExprKind::TypeCall { name, name_span, type_args, args }, name_span.to(end)))
    }

    /// `(arg, ...)`; returns the arguments and the span of the closing `)`.
    fn call_args(&mut self) -> PResult<(Vec<Arg>, Span)> {
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
        let value = self.expr()?;
        Ok(Arg { label, is_var, span: start.to(value.span), value })
    }
}
