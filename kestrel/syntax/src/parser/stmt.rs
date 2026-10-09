use super::{PResult, Parser};
use crate::ast::{AssignOp, Block, Stmt};
use crate::token::Tok;

impl<'t> Parser<'t> {
    pub(crate) fn block(&mut self) -> PResult<Block> {
        let start = self.expect(&Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.at(&Tok::RBrace) && !self.at(&Tok::Eof) {
            match self.stmt() {
                Ok(s) => {
                    stmts.push(s);
                    let ended = self.at(&Tok::RBrace) || self.eat(&Tok::Newline) || self.eat(&Tok::Semi);
                    if !ended {
                        let found = self.peek().describe();
                        self.error_here(format!("expected end of line, found {found}"));
                        self.recover_stmt();
                    }
                }
                Err(()) => self.recover_stmt(),
            }
            self.skip_newlines();
        }
        let end = self.expect(&Tok::RBrace, "`}`")?;
        Ok(Block { stmts, span: start.to(end) })
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        match self.peek() {
            Tok::Let | Tok::Var => self.let_stmt(),
            Tok::For => self.for_stmt(),
            Tok::While => {
                self.bump();
                let cond = self.expr()?;
                let body = self.block()?;
                Ok(Stmt::While { span: start.to(body.span), cond, body })
            }
            Tok::Par => {
                self.error_here("`par for` isn't supported yet");
                Err(())
            }
            Tok::Break => {
                self.bump();
                Ok(Stmt::Break(start))
            }
            Tok::Continue => {
                self.bump();
                Ok(Stmt::Continue(start))
            }
            Tok::Return => {
                self.bump();
                let ends = matches!(self.peek(), Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof);
                let value = if ends { None } else { Some(self.expr()?) };
                let span = value.as_ref().map_or(start, |v| start.to(v.span));
                Ok(Stmt::Return { value, span })
            }
            _ => self.expr_or_assign(),
        }
    }

    fn let_stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        let is_var = self.at(&Tok::Var);
        self.bump();
        let (name, name_span) = self.expect_ident("a variable name")?;
        let bind = self.binding(name, name_span);
        let ty = if self.eat(&Tok::Colon) { Some(self.type_expr()?) } else { None };
        self.expect(&Tok::Assign, "`=`")?;
        let value = self.expr()?;
        Ok(Stmt::Let { is_var, bind, ty, span: start.to(value.span), value })
    }

    fn for_stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        self.bump();
        let (name, name_span) = self.expect_ident("a loop variable name")?;
        if self.at(&Tok::In) || self.at(&Tok::Comma) {
            self.error_here("for-in loops aren't supported yet");
            return Err(());
        }
        let var = self.binding(name, name_span);
        self.expect(&Tok::From, "`from`")?;
        let lo = self.expr()?;
        self.expect(&Tok::To, "`to`")?;
        let hi = self.expr()?;
        let body = self.block()?;
        Ok(Stmt::ForRange { var, start: lo, end: hi, span: start.to(body.span), body })
    }

    fn expr_or_assign(&mut self) -> PResult<Stmt> {
        let target = self.expr()?;
        let op = match self.peek() {
            Tok::Assign => AssignOp::Set,
            Tok::PlusEq => AssignOp::Add,
            Tok::MinusEq => AssignOp::Sub,
            Tok::StarEq => AssignOp::Mul,
            Tok::SlashEq => AssignOp::Div,
            Tok::PercentEq => AssignOp::Rem,
            _ => return Ok(Stmt::Expr(target)),
        };
        self.bump();
        let value = self.expr()?;
        Ok(Stmt::Assign { span: target.span.to(value.span), target, op, value })
    }
}
