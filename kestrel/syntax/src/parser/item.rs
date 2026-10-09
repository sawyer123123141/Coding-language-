use super::{PResult, Parser};
use crate::ast::{ConstDecl, FnDecl, Item, Param, TypeExpr, TypeKind};
use crate::span::Span;
use crate::token::Tok;

impl<'t> Parser<'t> {
    pub(crate) fn items(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        self.skip_newlines();
        while !self.at(&Tok::Eof) {
            match self.item() {
                Ok(item) => {
                    items.push(item);
                    let ended = self.at(&Tok::Eof) || self.eat(&Tok::Newline) || self.eat(&Tok::Semi);
                    if !ended {
                        let found = self.peek().describe();
                        self.error_here(format!("expected end of line, found {found}"));
                        self.recover_item();
                    }
                }
                Err(()) => self.recover_item(),
            }
            self.skip_newlines();
        }
        items
    }

    fn item(&mut self) -> PResult<Item> {
        let start = self.span();
        let is_pub = self.eat(&Tok::Pub);
        match self.peek().clone() {
            Tok::Let => self.const_decl(is_pub, start),
            Tok::Pure => {
                self.bump();
                self.fn_decl(is_pub, true, start)
            }
            Tok::Ident(name) if name == "fn" => {
                self.error_here("Kestrel functions don't use `fn`: write `name(params) { ... }`");
                Err(())
            }
            Tok::Ident(_) => self.fn_decl(is_pub, false, start),
            Tok::Var => {
                self.error_here("global variables aren't supported yet");
                Err(())
            }
            Tok::Struct | Tok::Enum => {
                self.error_here("`struct` and `enum` aren't supported yet");
                Err(())
            }
            Tok::Use => {
                self.error_here("`use` isn't supported yet");
                Err(())
            }
            other => {
                self.error_here(format!("expected a function or `let`, found {}", other.describe()));
                Err(())
            }
        }
    }

    fn fn_decl(&mut self, is_pub: bool, is_pure: bool, start: Span) -> PResult<Item> {
        let (name, name_span) = self.expect_ident("a function name")?;
        self.expect(&Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        while !self.at(&Tok::RParen) {
            params.push(self.param()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RParen, "`)` or `,`")?;
        let ret = if self.eat(&Tok::Arrow) { Some(self.type_expr()?) } else { None };
        let body = self.block()?;
        let span = start.to(body.span);
        Ok(Item::Fn(FnDecl { name, name_span, is_pure, is_pub, params, ret, body, span }))
    }

    fn param(&mut self) -> PResult<Param> {
        let is_var = self.eat(&Tok::Var);
        if self.at(&Tok::SelfKw) {
            self.error_here("`self` parameters aren't supported yet");
            return Err(());
        }
        let (name, span) = self.expect_ident("a parameter name")?;
        let bind = self.binding(name, span);
        self.expect(&Tok::Colon, "`:` and a type")?;
        let ty = self.type_expr()?;
        Ok(Param { bind, is_var, ty })
    }

    fn const_decl(&mut self, is_pub: bool, start: Span) -> PResult<Item> {
        self.bump();
        let (name, name_span) = self.expect_ident("a constant name")?;
        let bind = self.binding(name, name_span);
        let ty = if self.eat(&Tok::Colon) { Some(self.type_expr()?) } else { None };
        self.expect(&Tok::Assign, "`=`")?;
        let value = self.expr()?;
        let span = start.to(value.span);
        Ok(Item::Const(ConstDecl { is_pub, bind, ty, value, span }))
    }

    pub(crate) fn type_expr(&mut self) -> PResult<TypeExpr> {
        let start = self.span();
        let mut ty = if self.eat(&Tok::LBracket) {
            let inner = self.type_expr()?;
            let end = self.expect(&Tok::RBracket, "`]`")?;
            TypeExpr { kind: TypeKind::Array(Box::new(inner)), span: start.to(end) }
        } else {
            let (name, span) = self.expect_ident("a type")?;
            let mut args = Vec::new();
            let mut end = span;
            if self.eat(&Tok::LBracket) {
                loop {
                    args.push(self.type_expr()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                end = self.expect(&Tok::RBracket, "`]`")?;
            }
            TypeExpr { kind: TypeKind::Named { name, args }, span: start.to(end) }
        };
        if self.at(&Tok::Question) {
            let end = self.bump().span;
            ty = TypeExpr { span: ty.span.to(end), kind: TypeKind::Optional(Box::new(ty)) };
        }
        Ok(ty)
    }
}
