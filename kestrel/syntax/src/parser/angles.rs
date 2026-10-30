//! Other languages' angle brackets for generics: `list<Item>`, `Map<string, List<int>>`,
//! `Option<int>`, `struct Stack<T>` and `list<int>()`. The type is read from the tokens alone,
//! only for the message, which says how Kestrel writes it.

use super::{PResult, Parser};
use crate::ast::{BinaryOp, Expr, ExprKind, TypeExpr};
use crate::token::{Tok, Token};

const GENERIC: &str = "Kestrel writes generic types with square brackets";
const OPTIONAL: &str = "Kestrel writes an optional type with `?`";

/// The most levels of angle brackets read for a message; past it there is no example.
const MAX_LEVELS: u32 = 64;

/// Other languages' names for Kestrel's built-in generic types.
fn kestrel_type_name(name: &str) -> &str {
    match name {
        "List" | "Vec" | "ArrayList" => "list",
        "Map" | "HashMap" | "Dictionary" | "Dict" => "map",
        _ => name,
    }
}

/// `name<args>` as Kestrel writes it: `list[int]`, `[int]` for an `Array`, `int?` for an `Option`.
fn generic_text(name: &str, args: &[String]) -> String {
    match (name, args) {
        ("Option" | "Optional" | "Nullable", [one]) => format!("{one}?"),
        ("Array", [one]) => format!("[{one}]"),
        _ => format!("{}[{}]", kestrel_type_name(name), args.join(", ")),
    }
}

/// Reads a type written with angle brackets. A `>>` closes two lists: `half` is set while the
/// second of its `>`s is still to be read.
struct Scan<'a> {
    toks: &'a [Token],
    pos: usize,
    half: bool,
    levels: u32,
}

impl Scan<'_> {
    fn cur(&self) -> &Tok {
        if self.half {
            return &Tok::Gt;
        }
        self.toks.get(self.pos).map_or(&Tok::Eof, |t| &t.tok)
    }

    /// `name`, `name<args>`, `[T]`, each with an optional `?`.
    fn ty(&mut self) -> Option<String> {
        self.levels += 1;
        if self.levels > MAX_LEVELS {
            return None;
        }
        let text = match self.cur().clone() {
            Tok::Ident(name) => {
                self.pos += 1;
                if self.cur() == &Tok::Lt {
                    self.pos += 1;
                    let args = self.args()?;
                    generic_text(&name, &args)
                } else {
                    name
                }
            }
            Tok::LBracket => {
                self.pos += 1;
                let inner = self.ty()?;
                if self.cur() != &Tok::RBracket {
                    return None;
                }
                self.pos += 1;
                format!("[{inner}]")
            }
            _ => return None,
        };
        self.levels -= 1;
        if self.cur() == &Tok::Question {
            self.pos += 1;
            return Some(format!("{text}?"));
        }
        Some(text)
    }

    /// `T, U>` after a `<`: the types, and past the closing `>`.
    fn args(&mut self) -> Option<Vec<String>> {
        let mut args = vec![self.ty()?];
        while self.cur() == &Tok::Comma {
            self.pos += 1;
            args.push(self.ty()?);
        }
        match self.cur() {
            Tok::Gt => {
                self.half = false;
                self.pos += 1;
            }
            Tok::Shr => self.half = true,
            _ => return None,
        }
        Some(args)
    }
}

impl Parser {
    /// The type written with angle brackets at token `at`, as Kestrel writes it, and the index
    /// of the token after it.
    fn angle_text(&self, at: usize) -> Option<(String, usize)> {
        let mut scan = Scan { toks: &self.toks, pos: at, half: false, levels: 0 };
        let text = scan.ty()?;
        (!scan.half).then_some((text, scan.pos))
    }

    /// A type whose next token is `<`: another language's `list<Item>` after a name, and
    /// otherwise the error `type_expr` gives for a `<` where a type goes.
    #[inline(never)]
    pub(super) fn angle_type(&mut self) -> PResult<TypeExpr> {
        let Tok::Ident(name) = self.peek().clone() else {
            if self.at(&Tok::LBracket) {
                self.bump();
            }
            self.expect_ident("a type")?;
            return Err(());
        };
        let scanned = self.angle_text(self.pos);
        let (lead, example) = match &scanned {
            // `Option<int>` is `int?`, which has no brackets.
            Some((text, _)) if matches!(name.as_str(), "Option" | "Optional" | "Nullable") => (OPTIONAL, text.clone()),
            Some((text, _)) => (GENERIC, text.clone()),
            None => (GENERIC, format!("{}[...]", kestrel_type_name(&name))),
        };
        // A field's `list<int>` at the end of its line hides the line break (a line ending in
        // `>` goes on): recovery stops after the type instead, so the next field is read.
        if let Some((_, after)) = scanned {
            let (end, next) = (self.toks[after - 1].span.end, self.toks[after.min(self.toks.len() - 1)].span.start);
            if self.line_breaks_between(end, next) > 0 {
                self.resume_at = Some(after);
            }
        }
        self.bump();
        self.error_here(format!("{lead}: `{example}`"));
        Err(())
    }

    /// `struct Stack<T>` or `first<T>(...)`, at the `<` after the name being declared.
    #[inline(never)]
    pub(super) fn angle_type_params(&mut self) {
        let open = self.pos;
        let name = match &self.toks[open.saturating_sub(1)].tok {
            Tok::Ident(n) => n.clone(),
            _ => "Stack".to_string(),
        };
        let close = (open + 1..self.toks.len()).find(|&i| !matches!(self.toks[i].tok, Tok::Ident(_) | Tok::Comma | Tok::Colon | Tok::Plus));
        let inner = match close {
            Some(c) if self.toks[c].tok == Tok::Gt => self.tokens_text(open + 1, c),
            _ => String::new(),
        };
        let inner = if inner.is_empty() { "T".to_string() } else { inner };
        self.error_here(format!("{GENERIC}: `{name}[{inner}]`"));
    }

    /// After the comparison `e`, whose operator was token `op`: reports `list<int>()` or
    /// `Map<string, int>()` read as comparisons, or a chain of comparisons. Returns whether it
    /// reported one.
    #[inline(never)]
    pub(super) fn after_comparison(&mut self, e: &Expr, op: usize) -> bool {
        let chained = matches!(self.peek(), Tok::EqEq | Tok::NotEq | Tok::Lt | Tok::Gt | Tok::Le | Tok::Ge);
        if !chained && !self.at(&Tok::Comma) {
            return false;
        }
        if let Some(example) = self.angle_construction(e, op) {
            self.error(e.span, format!("{GENERIC}: `{example}`"));
            return true;
        }
        if chained {
            self.error_here("comparisons can't be chained; combine them with `and`");
        }
        chained
    }

    /// `list<int>()` for `e`, a `name < ...` whose `<` is token `op`, when the tokens from the
    /// name on are a type and an empty `()` (which no comparison is followed by).
    fn angle_construction(&self, e: &Expr, op: usize) -> Option<String> {
        let ExprKind::Binary { op: BinaryOp::Lt, lhs, .. } = &e.kind else { return None };
        if !matches!(lhs.kind, ExprKind::Ident(_)) || op == 0 {
            return None;
        }
        let (text, after) = self.angle_text(op - 1)?;
        let tok = |i: usize| self.toks.get(i).map(|t| &t.tok);
        (tok(after) == Some(&Tok::LParen) && tok(after + 1) == Some(&Tok::RParen)).then(|| format!("{text}()"))
    }
}
