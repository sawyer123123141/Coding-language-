//! Struct and enum bodies: fields, variants and methods.

use super::{PResult, Parser};
use crate::ast::{EnumDecl, FieldDecl, FnDecl, Item, StructDecl, VariantDecl};
use crate::span::Span;
use crate::token::Tok;

impl Parser {
    /// Natives are top-level only: inside a struct or enum, `native` is an error.
    fn reject_native_method(&mut self) -> PResult<()> {
        if self.at(&Tok::Native) {
            self.error_here("native functions can't be methods");
            return Err(());
        }
        Ok(())
    }

    /// The annotations before a member, now parsed: `method` is the method it came to, if any.
    fn attach_to_method(&mut self, pending: Option<super::annot::Pending>, method: Option<&FnDecl>) {
        match method {
            Some(f) => self.attach_to_fn(pending, f),
            None => self.attach_to_nothing(pending),
        }
    }

    pub(super) fn struct_decl(&mut self, is_pub: bool, start: Span) -> PResult<Item> {
        self.bump();
        let (name, name_span) = self.expect_ident("a struct name")?;
        let type_params = self.type_params()?;
        let ifaces = self.iface_list()?;
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
        Ok(Item::Struct(StructDecl { is_pub, name, name_span, type_params, ifaces, fields, methods, span: start.to(end) }))
    }

    pub(super) fn enum_decl(&mut self, is_pub: bool, start: Span) -> PResult<Item> {
        self.bump();
        let (name, name_span) = self.expect_ident("an enum name")?;
        let type_params = self.type_params()?;
        let ifaces = self.iface_list()?;
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
        Ok(Item::Enum(EnumDecl { is_pub, name, name_span, type_params, ifaces, variants, methods, span: start.to(end) }))
    }

    /// One variant (`Name` or `Name { field: Type, ... }`) or one method, after any annotations.
    fn enum_member(&mut self, owner: &str, variants: &mut Vec<VariantDecl>, methods: &mut Vec<FnDecl>) -> PResult<()> {
        let pending = self.annots_before_item()?;
        let before = methods.len();
        self.enum_member_after_annots(owner, variants, methods)?;
        self.attach_to_method(pending, methods.get(before));
        Ok(())
    }

    fn enum_member_after_annots(&mut self, owner: &str, variants: &mut Vec<VariantDecl>, methods: &mut Vec<FnDecl>) -> PResult<()> {
        let start = self.span();
        if self.variant_in_parens() {
            return Err(());
        }
        let is_method = matches!(self.peek(), Tok::Pub | Tok::Pure | Tok::Native) || self.peek_at(1) == &Tok::LParen;
        if is_method {
            let is_pub = self.eat(&Tok::Pub);
            let is_pure = self.eat(&Tok::Pure);
            self.reject_native_method()?;
            let (name, name_span) = self.expect_ident("a method name")?;
            self.refuse_method_params()?;
            methods.push(self.fn_rest(name, name_span, is_pub, is_pure, start, Some(owner))?);
            return Ok(());
        }
        let (name, name_span) = self.expect_ident("a variant or method name")?;
        if self.at(&Tok::LBracket) {
            // Not `is_method` above, whose test sees `[`: a method with placeholders of its own.
            self.refuse_method_params()?;
            if !self.at(&Tok::LParen) {
                return Err(());
            }
            methods.push(self.fn_rest(name, name_span, false, false, start, Some(owner))?);
            return Ok(());
        }
        let fields = if self.at(&Tok::LBrace) { self.variant_fields()? } else { Vec::new() };
        if let Some(f) = fields.first() {
            self.note_first_field(&name, f.name.clone());
        }
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

    /// One `field: Type [= default]` or one method inside a struct body, after any annotations.
    fn struct_member(&mut self, owner: &str, fields: &mut Vec<FieldDecl>, methods: &mut Vec<FnDecl>) -> PResult<()> {
        let pending = self.annots_before_item()?;
        let before = methods.len();
        self.struct_member_after_annots(owner, fields, methods)?;
        self.attach_to_method(pending, methods.get(before));
        Ok(())
    }

    fn struct_member_after_annots(&mut self, owner: &str, fields: &mut Vec<FieldDecl>, methods: &mut Vec<FnDecl>) -> PResult<()> {
        let start = self.span();
        let is_pub = self.eat(&Tok::Pub);
        let is_pure = self.eat(&Tok::Pure);
        self.reject_native_method()?;
        let (name, name_span) = self.expect_ident("a field or method name")?;
        if self.at(&Tok::LBracket) {
            self.refuse_method_params()?;
            if !self.at(&Tok::LParen) {
                return Err(());
            }
        }
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
        if self.at(&Tok::LParen) {
            return self.value_in_type_place(&name, &ty);
        }
        let default = if self.eat(&Tok::Assign) { Some(self.expr()?) } else { None };
        fields.push(FieldDecl { name, name_span, ty, default });
        Ok(())
    }
}
