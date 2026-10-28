//! `@prove` and `@assume`: reading `@name(...)`, and attaching it to what it is on.
//!
//! Two picks here are provisional (the user may change them): how far an annotation may sit from
//! what it applies to (`MAX_LINE_BREAKS`), and, in the checker, which expressions an `@assume`
//! condition may use. A `@prove` is recorded in `Program::proves`, keyed by its target's span.

use super::{PResult, Parser};
use crate::ast::{Contract, Expr, FnDecl, Item, ProveAnnot, Stmt};
use crate::span::Span;
use crate::token::Tok;

/// Syntax pick 1: the line breaks that may sit between an annotation and what it applies to (or
/// the next annotation). With none, the target follows on the same line.
const MAX_LINE_BREAKS: usize = 1;

const NAME_EXPECTED: &str = "expected an annotation name after `@`, like `@prove` or `@assume`";
const PROVE_EMPTY: &str = "`@prove` needs at least one contract, like `@prove(no_alloc)`";
const PROVE_TARGET: &str = "`@prove` goes before a function, a method or a loop (`for`, `par for` or `while`)";
const PROVE_LINE: &str = "`@prove` goes on the line right before what it applies to, or on the same line";
const PROVE_NATIVE: &str = "`@prove` can't go on a native function";
const ASSUME_EMPTY: &str = "`@assume` needs a condition, like `@assume(i < xs.len)`";
const ASSUME_PLACE: &str = "`@assume` goes inside a function, as a statement of its own";

/// One annotation, read.
pub(super) enum Annot {
    Prove { contracts: Vec<(Contract, Span)>, span: Span },
    Assume { cond: Expr, text: String, span: Span },
}

/// The `@prove`s in front of one target, merged.
pub(super) struct Pending {
    contracts: Vec<(Contract, Span)>,
    /// From the first `@` to the last `)`.
    span: Span,
}

impl Parser {
    /// One annotation, at its `@`: `@prove(a, b)` or `@assume(cond)`.
    pub(super) fn read_annot(&mut self) -> PResult<Annot> {
        let at = self.expect(&Tok::At, "`@`")?;
        // The name starts where the `@` ends: `@ prove` is an error.
        let name = match self.peek() {
            Tok::Ident(n) if self.span().start == at.end => n.clone(),
            _ => {
                self.error(at, NAME_EXPECTED);
                return Err(());
            }
        };
        let head = at.to(self.bump().span);
        match name.as_str() {
            "prove" => self.prove_args(head),
            "assume" => self.assume_args(head),
            _ => {
                self.error(head, format!("unknown annotation `@{name}`; Kestrel has `@prove` and `@assume`"));
                Err(())
            }
        }
    }

    /// Whether the parentheses after `@name` hold nothing: no `(`, or `()`. Skips an empty `()`.
    fn args_missing(&mut self) -> bool {
        match (self.at(&Tok::LParen), self.peek_at(1) == &Tok::RParen) {
            (true, true) => {
                self.bump();
                self.bump();
                true
            }
            (true, false) => false,
            (false, _) => true,
        }
    }

    fn prove_args(&mut self, head: Span) -> PResult<Annot> {
        if self.args_missing() {
            self.error(head, PROVE_EMPTY);
            return Err(());
        }
        self.bump();
        let mut contracts: Vec<(Contract, Span)> = Vec::new();
        loop {
            let (name, span) = self.expect_ident("a contract name")?;
            let Some(contract) = Contract::from_name(&name) else {
                let msg = format!("unknown contract `{name}`; `@prove` takes `no_alloc`, `no_checks`, `no_panic`, `inline` and `deterministic`");
                self.error(span, msg);
                return Err(());
            };
            // A name listed twice counts once.
            if !contracts.iter().any(|(c, _)| *c == contract) {
                contracts.push((contract, span));
            }
            if !self.eat(&Tok::Comma) || self.at(&Tok::RParen) {
                break;
            }
        }
        let end = self.expect(&Tok::RParen, "`)` or `,`")?;
        Ok(Annot::Prove { contracts, span: head.to(end) })
    }

    fn assume_args(&mut self, head: Span) -> PResult<Annot> {
        if self.args_missing() {
            self.error(head, ASSUME_EMPTY);
            return Err(());
        }
        self.bump();
        let first = self.pos;
        let cond = self.nested(|p| p.inside_brackets(Self::expr))?;
        let last = self.pos;
        let close = self.expect(&Tok::RParen, "`)`")?;
        let text = self.tokens_text(first, last);
        Ok(Annot::Assume { cond, text, span: head.to(close) })
    }

    /// The source of tokens `from..to` as written, with one space wherever white space or a
    /// comment separates two of them; empty when the parser has no source.
    fn tokens_text(&self, from: usize, to: usize) -> String {
        if self.src.is_empty() {
            return String::new();
        }
        let at = |o: u32| o.saturating_sub(self.base) as usize;
        let mut text = String::new();
        let mut end = None;
        for t in self.toks[from..to].iter().filter(|t| t.tok != Tok::Newline) {
            if end.is_some_and(|e| e != t.span.start) {
                text.push(' ');
            }
            text.push_str(self.src.get(at(t.span.start)..at(t.span.end)).unwrap_or_default());
            end = Some(t.span.end);
        }
        text
    }

    fn line_breaks_between(&self, from: u32, to: u32) -> usize {
        let at = |o: u32| o.saturating_sub(self.base) as usize;
        self.src.get(at(from)..at(to)).map_or(0, |t| t.bytes().filter(|&b| b == b'\n').count())
    }

    /// A run of annotations in front of a target, `first` being the one just read: the `@prove`s
    /// merged. `@assume` has no place in it: at the top level or in a type's body it is out of
    /// place; in a body, after a `@prove`, it is not something a `@prove` goes on. Leaves the
    /// parser on the target's first token.
    fn prove_run(&mut self, first: Annot, in_body: bool) -> PResult<Pending> {
        let mut pending: Option<Pending> = None;
        let mut annot = first;
        let mut too_far = false;
        loop {
            let (contracts, span) = match annot {
                Annot::Prove { contracts, span } => (contracts, span),
                Annot::Assume { span, .. } => {
                    match &pending {
                        Some(p) if in_body => self.error(p.span, PROVE_TARGET),
                        _ => self.error(span, ASSUME_PLACE),
                    }
                    return Err(());
                }
            };
            let run = pending.get_or_insert_with(|| Pending { contracts: Vec::new(), span });
            run.span = run.span.to(span);
            for (c, s) in contracts {
                if !run.contracts.iter().any(|(k, _)| *k == c) {
                    run.contracts.push((c, s));
                }
            }
            if self.at(&Tok::Newline) {
                let next = self.toks[(self.pos + 1).min(self.toks.len() - 1)].span.start;
                too_far |= self.line_breaks_between(span.end, next) > MAX_LINE_BREAKS;
                self.bump();
            }
            if !self.at(&Tok::At) {
                break;
            }
            annot = self.read_annot()?;
        }
        let run = pending.expect("a run holds the annotation it starts with");
        if too_far {
            self.error(run.span, PROVE_LINE);
        }
        Ok(run)
    }

    /// The annotations before a top-level item, a method or an interface method, if any.
    pub(super) fn annots_before_item(&mut self) -> PResult<Option<Pending>> {
        if !self.at(&Tok::At) {
            return Ok(None);
        }
        let first = self.read_annot()?;
        let pending = self.prove_run(first, false)?;
        if matches!(self.peek(), Tok::Eof | Tok::RBrace | Tok::Semi) {
            self.error(pending.span, PROVE_TARGET);
            return Err(());
        }
        Ok(Some(pending))
    }

    /// A statement that starts with `@`: an `@assume`, or `@prove`s and the loop they are on.
    pub(super) fn annotated_stmt(&mut self) -> PResult<Stmt> {
        let first = self.read_annot()?;
        if let Annot::Assume { cond, text, span } = first {
            return Ok(Stmt::Assume { cond, text, span });
        }
        let pending = self.prove_run(first, true)?;
        if !matches!(self.peek(), Tok::For | Tok::While | Tok::Par) {
            self.error(pending.span, PROVE_TARGET);
            return Err(());
        }
        let stmt = self.stmt()?;
        self.prove_on(pending, stmt.span());
        Ok(stmt)
    }

    /// An annotation where an expression should be (`let f = @prove(inline) (x) -> ...`).
    pub(super) fn annot_in_expr(&mut self) -> PResult<Expr> {
        match self.read_annot()? {
            Annot::Prove { span, .. } => self.error(span, PROVE_TARGET),
            Annot::Assume { span, .. } => self.error(span, ASSUME_PLACE),
        }
        Err(())
    }

    fn prove_on(&mut self, pending: Pending, target: Span) {
        self.proves.push(ProveAnnot { target, contracts: pending.contracts, span: pending.span });
    }

    /// The annotations before a function or method, now parsed as `f`.
    pub(super) fn attach_to_fn(&mut self, pending: Option<Pending>, f: &FnDecl) {
        match pending {
            Some(p) if f.is_native => self.error(p.span, PROVE_NATIVE),
            Some(p) => self.prove_on(p, f.span),
            None => {}
        }
    }

    /// The annotations before an item: only a function takes them.
    pub(super) fn attach_to_item(&mut self, pending: Option<Pending>, item: &Item) {
        match item {
            Item::Fn(f) => self.attach_to_fn(pending, f),
            _ => self.attach_to_nothing(pending),
        }
    }

    /// The annotations before a member that is not a method.
    pub(super) fn attach_to_nothing(&mut self, pending: Option<Pending>) {
        if let Some(p) = pending {
            self.error(p.span, PROVE_TARGET);
        }
    }
}
