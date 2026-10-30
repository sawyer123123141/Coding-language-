//! Messages for what other languages write differently: `0..n`, `|a| a + 1`, `func`, a
//! variant's data in parentheses, `=>` and `Enemy.Boss` in a `match`, `Enemy::Boss`, `for var`,
//! `n++`, `elif` and `let mut` (and `list<Item>` in `angles`). Each says how Kestrel writes it, at the token where the parse fails; none of them
//! changes what the language accepts.
//!
//! These are kept out of line and only report (the caller fails): some are reached from
//! functions that are on the stack once per level of nesting (see `depth`), whose frames must
//! stay small.

use super::{PResult, Parser};
use crate::ast::Expr;
use crate::span::Span;
use crate::token::Tok;

/// Where a `..` would be, for its message. Small: a copy is on the stack once per bracket.
#[derive(Clone, Copy, Default)]
pub(super) enum RangeAt {
    #[default]
    Plain,
    /// The head of a `for` loop, whose first token is at `first`: after `in` or `from`, which
    /// come right after the loop's variable.
    ForHead { first: u32, par: bool },
    /// Inside an index's brackets: `xs[0..2]`.
    Index,
}

impl Parser {
    /// Runs `f` with `at` as the place a `..` would be.
    pub(super) fn with_range_at<T>(&mut self, at: RangeAt, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = std::mem::replace(&mut self.range_at, at);
        let result = f(self);
        self.range_at = saved;
        result
    }

    /// The source of `span` as written; empty when the parser has no source.
    fn span_text(&self, span: Span) -> String {
        let at = |o: u32| o.saturating_sub(self.base) as usize;
        self.src.get(at(span.start)..at(span.end)).unwrap_or_default().to_string()
    }

    /// `0..n` or `0..=n`, at the first `.`.
    #[inline(never)]
    pub(super) fn range_spelling(&mut self) {
        let at = self.span();
        let msg = match self.range_at {
            RangeAt::Index => "Kestrel has no `..` slices; take part of a string with `s.slice(from, to)`, or copy part of a list in a `for i from a to b` loop".to_string(),
            RangeAt::Plain => "Kestrel has no `..` ranges; loop over numbers with `for i from 0 to n` (`to` leaves `n` out)".to_string(),
            RangeAt::ForHead { par: true, .. } => {
                "`par for` needs items to work on, not a range; loop over an array or list: `par for p in ps { ... }`".to_string()
            }
            RangeAt::ForHead { first, par: false } => {
                let first = first as usize;
                let var = match &self.toks[first.saturating_sub(2)].tok {
                    Tok::Ident(v) => v.clone(),
                    _ => "i".to_string(),
                };
                let lo = self.tokens_text(first, self.pos);
                self.bump();
                self.bump();
                let inclusive = self.eat(&Tok::Assign);
                let hi_first = self.pos;
                let hi = match self.attempt(|p| p.with_range_at(RangeAt::Plain, |p| p.in_head(Self::expr))) {
                    Some(_) => self.tokens_text(hi_first, self.pos),
                    None => String::new(),
                };
                let (lo, hi) = if lo.is_empty() || hi.is_empty() { ("0".to_string(), "n".to_string()) } else { (lo, hi) };
                if inclusive {
                    format!("Kestrel writes ranges as `for {var} from {lo} to {hi} + 1` (`to` leaves its end out, so `+ 1` takes `{hi}` too)")
                } else {
                    format!("Kestrel writes ranges as `for {var} from {lo} to {hi}` (`to` leaves `{hi}` out)")
                }
            }
        };
        self.error(at, msg);
    }

    /// `|a, b| a + b`, at the first `|`.
    #[inline(never)]
    pub(super) fn pipe_closure(&mut self) {
        let at = self.span();
        let saved = self.snapshot();
        let open = self.pos;
        let close = (open + 1..self.toks.len()).find(|&i| {
            !matches!(self.toks[i].tok, Tok::Ident(_) | Tok::Comma | Tok::Colon | Tok::Var | Tok::LBracket | Tok::RBracket | Tok::Question)
        });
        let mut example = None;
        if let Some(c) = close.filter(|&c| self.toks[c].tok == Tok::Pipe && !self.src.is_empty()) {
            let params = self.tokens_text(open + 1, c);
            self.pos = c + 1;
            let first = self.pos;
            let body = if self.at(&Tok::LBrace) {
                None
            } else {
                self.attempt(|p| p.inside_brackets(Self::expr)).map(|_| self.tokens_text(first, self.pos))
            };
            example = Some(format!("({params}) {{ {} }}", body.unwrap_or_else(|| "...".to_string())));
        }
        self.restore(saved);
        let example = example.unwrap_or_else(|| "(a, b) { ... }".to_string());
        self.error(at, format!("Kestrel writes closures as `{example}`"));
    }

    /// `Slime(size: int)` in an enum's body, at the variant's name: whether it is one (then it
    /// was reported, and the parser is past the `)`).
    #[inline(never)]
    pub(super) fn variant_in_parens(&mut self) -> bool {
        let Tok::Ident(name) = self.peek().clone() else { return false };
        if self.peek_at(1) != &Tok::LParen {
            return false;
        }
        let Some(close) = self.close_of(self.pos + 1) else { return false };
        if !matches!(self.toks.get(close + 1).map(|t| &t.tok), Some(Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof)) {
            return false;
        }
        let named = self.toks[self.pos + 2..close].iter().any(|t| t.tok == Tok::Colon);
        if let (true, Tok::Ident(field)) = (named, self.peek_at(2).clone()) {
            self.first_fields.insert(name.clone(), field);
        }
        let inner = self.tokens_text(self.pos + 2, close);
        let msg = if named && !inner.is_empty() {
            format!("enum variants declare their data in braces: `{name} {{ {inner} }}`")
        } else {
            format!("enum variants declare their data in braces, each field with a name: `{name} {{ value: int }}`")
        };
        self.error_here(msg);
        self.pos = close + 1;
        true
    }

    /// The other languages' forms of a `match` arm, after its variant's name (`name`, at
    /// `name_span`) and binding: `Slime(size, hp)` and `Slime s =>`. Reports one and fails.
    #[inline(never)]
    pub(super) fn arm_spelling(&mut self, name: &str, binding: Option<&str>) -> PResult<()> {
        if binding.is_none() && self.at(&Tok::LParen) {
            let short: String = name.chars().next().map(|c| c.to_lowercase().collect()).unwrap_or_default();
            // The variant's first field, when its enum came earlier in the file.
            let used = match self.first_fields.get(name) {
                Some(field) => format!(" and use `{short}.{field}`"),
                None => format!(" and read its fields as `{short}.<field>`"),
            };
            self.error_here(format!("a `match` arm takes the variant's data as one name: write `{name} {short} {{ ... }}`{used}"));
            return Err(());
        }
        // `Enemy.Boss b` or `Enemy::Boss b`: the arm names the variant alone.
        let path = match (self.peek(), self.peek_at(1), self.peek_at(2)) {
            (Tok::Dot, Tok::Ident(v), _) => Some((v.clone(), 2)),
            (Tok::Colon, Tok::Colon, Tok::Ident(v)) => Some((v.clone(), 3)),
            _ => None,
        };
        if let (None, Some((variant, len))) = (binding, path) {
            let arm = match self.peek_at(len) {
                Tok::Ident(b) => format!("{variant} {b}"),
                _ => variant,
            };
            self.error_here(format!("`match` arms name the variant alone: write `{arm} {{ ... }}`"));
            return Err(());
        }
        if self.at(&Tok::Assign) && self.peek_at(1) == &Tok::Gt {
            let arm = binding.map_or_else(|| name.to_string(), |b| format!("{name} {b}"));
            self.error_here(format!("`match` arms don't use `=>`: write `{arm} {{ ... }}`"));
            return Err(());
        }
        Ok(())
    }

    /// `for var x in xs`, at the `var`: reports it and moves past it, so the loop still parses.
    #[inline(never)]
    pub(super) fn for_var(&mut self, par: bool) {
        let from = self.pos + 1;
        let mut depth = 0i32;
        let mut end = from;
        let mut items = false;
        while let Some(t) = self.toks.get(end) {
            match t.tok {
                Tok::LBrace if depth == 0 => break,
                Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RBrace => depth -= 1,
                Tok::In if depth == 0 => items = true,
                Tok::Newline | Tok::Eof => break,
                _ => {}
            }
            end += 1;
        }
        let head = self.tokens_text(from, end);
        // `for i, x in xs`: the item is the second name.
        let item_at = if self.peek_at(2) == &Tok::Comma { 3 } else { 1 };
        let var = match self.peek_at(item_at) {
            Tok::Ident(v) => v.clone(),
            _ => "x".to_string(),
        };
        let kw = if par { "par for" } else { "for" };
        let write = if head.is_empty() { String::new() } else { format!(" write `{kw} {head}`;") };
        let msg = if items {
            format!("a `for` loop's variable can't be marked `var`:{write} changing `{var}` changes the item")
        } else {
            format!("a `for` loop's counter can't be marked `var`:{write} to change it as you go, use a `while` loop")
        };
        self.error_here(msg);
        self.bump();
    }

    /// Whether the current token and the next are a `++` or `--` written together at the end of
    /// a statement: `n++` (not `2 ++ 3` or `a - -b`).
    #[inline(never)]
    pub(super) fn at_step(&self) -> bool {
        let [a, b] = [self.pos, self.pos + 1].map(|i| &self.toks[i.min(self.toks.len() - 1)]);
        let same = matches!((&a.tok, &b.tok), (Tok::Plus, Tok::Plus) | (Tok::Minus, Tok::Minus));
        if !(same && a.span.end == b.span.start) {
            return false;
        }
        let next = &self.toks[(self.pos + 2).min(self.toks.len() - 1)];
        matches!(next.tok, Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof) || self.line_breaks_between(b.span.end, next.span.start) > 0
    }

    /// `n++` or `n--`, at the first `+` or `-` after `target`.
    #[inline(never)]
    pub(super) fn step_spelling(&mut self, target: &Expr) {
        let (op, assign) = if self.at(&Tok::Plus) { ("++", "+=") } else { ("--", "-=") };
        let text = self.span_text(target.span);
        let text = if text.is_empty() { "x".to_string() } else { text };
        self.error_here(format!("Kestrel has no `{op}`: write `{text} {assign} 1`"));
    }

    /// Whether the current token is `elif` starting a condition (`elif n > 0 {`), not the name
    /// `elif` being assigned, called or read.
    #[inline(never)]
    pub(super) fn at_elif(&self) -> bool {
        matches!(self.peek(), Tok::Ident(k) if k == "elif")
            && matches!(
                self.peek_at(1),
                Tok::Ident(_) | Tok::Int(_) | Tok::Hex(_) | Tok::Float(_) | Tok::Str(_) | Tok::StrParts(_) | Tok::True | Tok::False | Tok::Not | Tok::SelfKw
            )
    }

    /// `elif`, which Kestrel writes `else if`.
    #[inline(never)]
    pub(super) fn elif(&mut self) {
        self.error_here("Kestrel writes `else if`, not `elif`");
    }

    /// `Enemy::Boss` after the name `e`, at the first `:`.
    #[inline(never)]
    pub(super) fn path_spelling(&mut self, e: &Expr) {
        let base = self.span_text(e.span);
        let msg = match (self.peek_at(2), base.is_empty()) {
            (Tok::Ident(v), false) => format!("Kestrel writes `{base}.{v}`, not `{base}::{v}`"),
            _ => "Kestrel writes `Enemy.Boss`, not `Enemy::Boss`".to_string(),
        };
        self.error_here(msg);
    }
}
