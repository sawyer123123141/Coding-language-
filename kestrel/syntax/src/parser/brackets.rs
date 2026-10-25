//! Brackets after a name: `best[Enemy](foes)` holds types, `handlers[i](3)` an index, and only
//! what the name is can tell them apart (see `settle`). After a bare name or a field the parser
//! reads both ways and keeps what parses.
//!
//! The types are read first and silently. When they don't parse the brackets are an index, read
//! for real, so malformed nested brackets (`a[b[c[`) cost linear work per level. Only when the
//! types parse is the index tried too, inside `attempt`.

use super::{PResult, Parser};
use crate::ast::{Expr, ExprKind, TypeExpr};
use crate::span::Span;
use crate::token::Tok;

impl Parser {
    /// `[type, type]` at the current `[`: the types and the span of the `]`.
    pub(super) fn type_list(&mut self) -> PResult<(Vec<TypeExpr>, Span)> {
        self.nested_type(|p| {
            p.expect(&Tok::LBracket, "`[`")?;
            let mut types = Vec::new();
            loop {
                types.push(p.type_expr()?);
                if !p.eat(&Tok::Comma) {
                    break;
                }
            }
            let end = p.expect(&Tok::RBracket, "`]` or `,`")?;
            Ok((types, end))
        })
    }

    /// The type list at the current `[` when it parses, with nothing consumed, reported or
    /// recorded either way.
    pub(super) fn type_list_silently(&mut self) -> Option<Vec<TypeExpr>> {
        let saved = self.snapshot();
        let found = self.attempt(Self::type_list).map(|(types, _)| types);
        self.restore(saved);
        found
    }

    /// `[index]` at the current `[`: the index and the span of the `]`.
    fn index_part(&mut self) -> PResult<(Expr, Span)> {
        self.nested(|p| {
            p.bump();
            let index = p.inside_brackets(Self::expr)?;
            Ok((index, p.expect(&Tok::RBracket, "`]`")?))
        })
    }

    /// `base[index]`, the brackets read as an index only.
    pub(super) fn index_after(&mut self, base: Expr) -> PResult<Expr> {
        let (index, end) = self.index_part()?;
        let span = base.span.to(end);
        Ok(self.expr_node(ExprKind::Index { base: Box::new(base), index: Box::new(index) }, span))
    }

    /// The brackets at the current `[`, after `base`. Kept out of line: `postfix` is on the
    /// stack once per level of nesting.
    #[inline(never)]
    pub(super) fn brackets_after(&mut self, base: Expr) -> PResult<Expr> {
        let Some(types) = self.type_list_silently() else { return self.index_after(base) };
        if let Some((index, end)) = self.attempt(Self::index_part) {
            let span = base.span.to(end);
            let kind = ExprKind::Brackets { base: Box::new(base), index: Some(Box::new(index)), types };
            return Ok(self.expr_node(kind, span));
        }
        // Only the types parse: read them for real.
        let (types, end) = self.type_list()?;
        let span = base.span.to(end);
        Ok(self.expr_node(ExprKind::Brackets { base: Box::new(base), index: None, types }, span))
    }
}
