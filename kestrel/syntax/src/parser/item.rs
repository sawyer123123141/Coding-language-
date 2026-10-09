use super::{PResult, Parser};
use crate::ast::{ConstDecl, EnumDecl, FieldDecl, FnDecl, Item, Param, StructDecl, TypeExpr, TypeKind, VariantDecl};
use crate::span::Span;
use crate::token::Tok;

impl Parser {
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
            Tok::Struct => self.struct_decl(is_pub, start),
            Tok::Ident(name) if name == "fn" => {
                self.error_here("Kestrel functions don't use `fn`: write `name(params) { ... }`");
                Err(())
            }
            Tok::Ident(_) => self.fn_decl(is_pub, false, start),
            Tok::Var => {
                self.error_here("global variables aren't supported yet");
                Err(())
            }
            Tok::Enum => self.enum_decl(is_pub, start),
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
        Ok(Item::Fn(self.fn_rest(name, name_span, is_pub, is_pure, start, None)?))
    }

    /// Everything after a function's name. `owner` is the struct a method belongs to.
    fn fn_rest(
        &mut self,
        name: String,
        name_span: Span,
        is_pub: bool,
        is_pure: bool,
        start: Span,
        owner: Option<&str>,
    ) -> PResult<FnDecl> {
        self.expect(&Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        while !self.at(&Tok::RParen) {
            params.push(self.param(params.len(), owner)?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RParen, "`)` or `,`")?;
        let ret = if self.eat(&Tok::Arrow) { Some(self.type_expr()?) } else { None };
        let body = self.block()?;
        let span = start.to(body.span);
        Ok(FnDecl { name, name_span, is_pure, is_pub, params, ret, body, span })
    }

    fn param(&mut self, index: usize, owner: Option<&str>) -> PResult<Param> {
        let is_var = self.eat(&Tok::Var);
        if self.at(&Tok::SelfKw) {
            let span = self.bump().span;
            let Some(owner) = owner.filter(|_| index == 0) else {
                self.error(span, "`self` is only allowed as the first parameter of a method");
                return Err(());
            };
            let bind = self.binding("self".to_string(), span);
            let ty = TypeExpr { kind: TypeKind::Named { name: owner.to_string(), args: Vec::new() }, span };
            return Ok(Param { bind, is_var, is_self: true, ty });
        }
        let (name, span) = self.expect_ident("a parameter name")?;
        let bind = self.binding(name, span);
        self.expect(&Tok::Colon, "`:` and a type")?;
        let ty = self.type_expr()?;
        Ok(Param { bind, is_var, is_self: false, ty })
    }

    fn struct_decl(&mut self, is_pub: bool, start: Span) -> PResult<Item> {
        self.bump();
        let (name, name_span) = self.expect_ident("a struct name")?;
        self.expect(&Tok::LBrace, "`{`")?;
        let (mut fields, mut methods) = (Vec::new(), Vec::new());
        self.skip_newlines();
        while !self.at(&Tok::RBrace) && !self.at(&Tok::Eof) {
            match self.struct_member(&name, &mut fields, &mut methods) {
                Ok(()) => {
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
        Ok(Item::Struct(StructDecl { is_pub, name, name_span, fields, methods, span: start.to(end) }))
    }

    fn enum_decl(&mut self, is_pub: bool, start: Span) -> PResult<Item> {
        self.bump();
        let (name, name_span) = self.expect_ident("an enum name")?;
        self.expect(&Tok::LBrace, "`{`")?;
        let (mut variants, mut methods) = (Vec::new(), Vec::new());
        self.skip_newlines();
        while !self.at(&Tok::RBrace) && !self.at(&Tok::Eof) {
            match self.enum_member(&name, &mut variants, &mut methods) {
                Ok(()) => {
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
        Ok(Item::Enum(EnumDecl { is_pub, name, name_span, variants, methods, span: start.to(end) }))
    }

    /// One variant (`Name` or `Name { field: Type, ... }`) or one method.
    fn enum_member(&mut self, owner: &str, variants: &mut Vec<VariantDecl>, methods: &mut Vec<FnDecl>) -> PResult<()> {
        let start = self.span();
        let is_method = matches!(self.peek(), Tok::Pub | Tok::Pure) || self.peek_at(1) == &Tok::LParen;
        if is_method {
            let is_pub = self.eat(&Tok::Pub);
            let is_pure = self.eat(&Tok::Pure);
            let (name, name_span) = self.expect_ident("a method name")?;
            methods.push(self.fn_rest(name, name_span, is_pub, is_pure, start, Some(owner))?);
            return Ok(());
        }
        let (name, name_span) = self.expect_ident("a variant or method name")?;
        let fields = if self.at(&Tok::LBrace) { self.variant_fields()? } else { Vec::new() };
        variants.push(VariantDecl { name, name_span, fields });
        Ok(())
    }

    /// `{ field: Type, ... }`, fields separated by commas or newlines.
    fn variant_fields(&mut self) -> PResult<Vec<FieldDecl>> {
        self.expect(&Tok::LBrace, "`{`")?;
        let fields = self.variant_field_list();
        if fields.is_err() {
            // Resume after this variant's `}` so the rest of the enum still parses.
            while !matches!(self.peek(), Tok::RBrace | Tok::Eof) {
                self.bump();
            }
            self.eat(&Tok::RBrace);
        }
        fields
    }

    fn variant_field_list(&mut self) -> PResult<Vec<FieldDecl>> {
        let mut fields = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&Tok::RBrace) {
                return Ok(fields);
            }
            let (name, name_span) = self.expect_ident("a field name")?;
            self.expect(&Tok::Colon, "`:` and a type")?;
            let ty = self.type_expr()?;
            fields.push(FieldDecl { name, name_span, ty, default: None });
            if !self.eat(&Tok::Comma) && !self.at(&Tok::Newline) && !self.at(&Tok::RBrace) {
                let found = self.peek().describe();
                self.error_here(format!("expected `,` or `}}`, found {found}"));
                return Err(());
            }
        }
    }

    /// One `field: Type [= default]` or one method inside a struct body.
    fn struct_member(&mut self, owner: &str, fields: &mut Vec<FieldDecl>, methods: &mut Vec<FnDecl>) -> PResult<()> {
        let start = self.span();
        let is_pub = self.eat(&Tok::Pub);
        let is_pure = self.eat(&Tok::Pure);
        let (name, name_span) = self.expect_ident("a field or method name")?;
        if self.at(&Tok::LParen) {
            methods.push(self.fn_rest(name, name_span, is_pub, is_pure, start, Some(owner))?);
            return Ok(());
        }
        if is_pure {
            self.error(start, "only methods can be `pure`");
            return Err(());
        }
        if !self.eat(&Tok::Colon) {
            let found = self.peek().describe();
            self.error_here(format!("expected `:` and a type, or `(` for a method, found {found}"));
            return Err(());
        }
        let ty = self.type_expr()?;
        let default = if self.eat(&Tok::Assign) { Some(self.expr()?) } else { None };
        fields.push(FieldDecl { name, name_span, ty, default });
        Ok(())
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
