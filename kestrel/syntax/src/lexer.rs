use crate::diag::Diagnostic;
use crate::newlines;
use crate::span::Span;
use crate::token::{keyword, StrPart, Tok, Token};

/// Source text to tokens. Always ends with one `Eof`; statement-continuing
/// newlines are already removed.
pub fn lex(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lx = Lexer { src, bytes: src.as_bytes(), pos: 0, toks: Vec::new(), diags: Vec::new() };
    lx.run();
    let end = Span::new(src.len(), src.len());
    lx.toks.push(Token { tok: Tok::Eof, span: end });
    (newlines::filter(lx.toks), lx.diags)
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    toks: Vec<Token>,
    diags: Vec<Diagnostic>,
}

impl<'a> Lexer<'a> {
    fn peek(&self, ahead: usize) -> u8 {
        *self.bytes.get(self.pos + ahead).unwrap_or(&0)
    }

    fn push(&mut self, tok: Tok, start: usize) {
        self.toks.push(Token { tok, span: Span::new(start, self.pos) });
    }

    /// Whether the last token ends a value, so an operator may follow it.
    fn after_value(&self) -> bool {
        self.toks.last().is_some_and(|t| {
            matches!(
                t.tok,
                Tok::Ident(_) | Tok::Int(_) | Tok::Hex(_) | Tok::Float(_) | Tok::Str(_) | Tok::StrParts(_)
                    | Tok::True | Tok::False | Tok::NoneKw | Tok::SelfKw | Tok::RParen | Tok::RBracket | Tok::RBrace
            )
        })
    }

    fn error(&mut self, start: usize, msg: impl Into<String>) {
        // Clamp to the source so a span never points past the end of the file.
        let end = self.pos.max(start + 1).min(self.src.len());
        self.diags.push(Diagnostic::error(Span::new(start.min(end), end), msg));
    }

    fn run(&mut self) {
        while self.pos < self.bytes.len() {
            let start = self.pos;
            match self.peek(0) {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b'\n' => {
                    self.pos += 1;
                    self.push(Tok::Newline, start);
                }
                b'/' if self.peek(1) == b'/' => {
                    while self.pos < self.bytes.len() && self.peek(0) != b'\n' {
                        self.pos += 1;
                    }
                }
                b'"' => self.string(),
                b'0'..=b'9' => self.number(),
                c if c == b'_' || c.is_ascii_alphabetic() => self.word(),
                _ => self.punct(),
            }
        }
    }

    fn word(&mut self) {
        let start = self.pos;
        while self.peek(0) == b'_' || self.peek(0).is_ascii_alphanumeric() {
            self.pos += 1;
        }
        let text = &self.src[start..self.pos];
        let tok = keyword(text).unwrap_or_else(|| Tok::Ident(text.to_string()));
        self.push(tok, start);
    }

    fn number(&mut self) {
        let start = self.pos;
        if self.peek(0) == b'0' && (self.peek(1) == b'x' || self.peek(1) == b'X') {
            self.pos += 2;
            while self.peek(0).is_ascii_hexdigit() || self.peek(0) == b'_' {
                self.pos += 1;
            }
            let digits: String = self.src[start + 2..self.pos].chars().filter(|&c| c != '_').collect();
            match u64::from_str_radix(&digits, 16) {
                Ok(n) => self.push(Tok::Hex(n), start),
                Err(_) => self.error(start, "number is too large"),
            }
            return;
        }
        while self.peek(0).is_ascii_digit() || self.peek(0) == b'_' {
            self.pos += 1;
        }
        let is_float = self.peek(0) == b'.' && self.peek(1).is_ascii_digit();
        if is_float {
            self.pos += 1;
            while self.peek(0).is_ascii_digit() || self.peek(0) == b'_' {
                self.pos += 1;
            }
        }
        let text: String = self.src[start..self.pos].chars().filter(|&c| c != '_').collect();
        if is_float {
            self.push(Tok::Float(text.parse().unwrap()), start);
        } else {
            match text.parse::<u64>() {
                Ok(n) => self.push(Tok::Int(n), start),
                Err(_) => self.error(start, "number is too large"),
            }
        }
    }

    fn string(&mut self) {
        let start = self.pos;
        self.pos += 1;
        let mut value = String::new();
        let mut parts: Vec<StrPart> = Vec::new();
        loop {
            if self.pos >= self.bytes.len() || self.peek(0) == b'\n' {
                self.error(start, "unterminated string");
                return;
            }
            match self.peek(0) {
                b'"' => {
                    self.pos += 1;
                    if parts.is_empty() {
                        self.push(Tok::Str(value), start);
                    } else {
                        if !value.is_empty() {
                            parts.push(StrPart::Lit(value));
                        }
                        self.push(Tok::StrParts(parts), start);
                    }
                    return;
                }
                b'\\' => {
                    let ch = match self.peek(1) {
                        b'n' => '\n',
                        b't' => '\t',
                        b'"' => '"',
                        b'\\' => '\\',
                        b'{' => '{',
                        b'}' => '}',
                        _ => {
                            let at = self.pos;
                            self.pos += 1;
                            // Skip the whole escaped char, but leave a newline or EOF for the loop.
                            if let Some(c) = self.src[self.pos..].chars().next().filter(|&c| c != '\n') {
                                self.pos += c.len_utf8();
                            }
                            self.error(at, "unknown escape sequence");
                            continue;
                        }
                    };
                    value.push(ch);
                    self.pos += 2;
                }
                b'{' => {
                    if !value.is_empty() {
                        parts.push(StrPart::Lit(std::mem::take(&mut value)));
                    }
                    match self.interpolation() {
                        Some(part) => parts.push(part),
                        None => return,
                    }
                }
                _ => {
                    let ch = self.src[self.pos..].chars().next().unwrap();
                    value.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    /// `{code}` inside a string, with `pos` on the `{`. On error, reports it,
    /// skips the rest of the string, and returns `None`.
    fn interpolation(&mut self) -> Option<StrPart> {
        let open = self.pos;
        let code_start = self.pos + 1;
        let mut depth = 0;
        let mut i = code_start;
        let fail = |lx: &mut Self, msg: &str| {
            lx.pos = lx.pos.max(open + 1);
            lx.error(open, msg);
            while lx.pos < lx.bytes.len() && !matches!(lx.peek(0), b'"' | b'\n') {
                lx.pos += 1;
            }
            if lx.peek(0) == b'"' {
                lx.pos += 1;
            }
            None
        };
        loop {
            match self.bytes.get(i) {
                // A `"` ends the string (strings nested inside `{...}` aren't supported).
                None | Some(b'\n' | b'"') => return fail(self, "unterminated `{` in a string"),
                Some(b'{') => depth += 1,
                Some(b'}') if depth == 0 => break,
                Some(b'}') => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        let text = &self.src[code_start..i];
        if text.trim().is_empty() {
            return fail(self, "empty `{}` in a string; write `\\{` for a brace");
        }
        self.pos = i + 1;
        Some(StrPart::Code { text: text.to_string(), start: code_start as u32 })
    }

    fn punct(&mut self) {
        let start = self.pos;
        let three = [self.peek(0), self.peek(1), self.peek(2)];
        if &three == b"<<=" || &three == b">>=" {
            self.pos += 3;
            return self.push(if three[0] == b'<' { Tok::ShlEq } else { Tok::ShrEq }, start);
        }
        // `*%=`, `+%=` and `-%=` come before `*%`, `+%` and `-%`, and those before `*`, `+`, `-` and `%=`.
        let wrapping = match (three[0], three[1]) {
            (b'*', b'%') => Some((Tok::StarPercent, Tok::StarPercentEq)),
            (b'+', b'%') => Some((Tok::PlusPercent, Tok::PlusPercentEq)),
            (b'-', b'%') => Some((Tok::MinusPercent, Tok::MinusPercentEq)),
            _ => None,
        };
        if let Some((plain, assign)) = wrapping {
            let compound = three[2] == b'=';
            self.pos += if compound { 3 } else { 2 };
            return self.push(if compound { assign } else { plain }, start);
        }
        let two = [self.peek(0), self.peek(1)];
        let pair = match &two {
            b"->" => Some(Tok::Arrow),
            b"==" => Some(Tok::EqEq),
            b"!=" => Some(Tok::NotEq),
            b"<=" => Some(Tok::Le),
            b">=" => Some(Tok::Ge),
            b"+=" => Some(Tok::PlusEq),
            b"-=" => Some(Tok::MinusEq),
            b"*=" => Some(Tok::StarEq),
            b"/=" => Some(Tok::SlashEq),
            b"%=" => Some(Tok::PercentEq),
            b"<<" => Some(Tok::Shl),
            b">>" => Some(Tok::Shr),
            b"&=" => Some(Tok::AmpEq),
            b"|=" => Some(Tok::PipeEq),
            b"^=" => Some(Tok::CaretEq),
            _ => None,
        };
        if let Some(tok) = pair {
            self.pos += 2;
            self.push(tok, start);
            return;
        }
        if &two == b"&&" || &two == b"||" {
            self.pos += 2;
            let after_value = self.after_value();
            // Where a value starts, `||` is another language's closure with no parameters.
            if two[0] == b'|' && !after_value {
                return self.error(start, "Kestrel writes closures as `() { ... }`");
            }
            let (msg, tok) = if two[0] == b'&' { ("Kestrel uses `and`, not `&&`", Tok::And) } else { ("Kestrel uses `or`, not `||`", Tok::Or) };
            self.error(start, msg);
            // Between values it is lexed as the keyword, so the rest of the line still parses.
            if after_value {
                self.push(tok, start);
            }
            return;
        }
        let ch = self.src[self.pos..].chars().next().unwrap();
        self.pos += ch.len_utf8();
        let tok = match ch {
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '{' => Tok::LBrace,
            '}' => Tok::RBrace,
            '[' => Tok::LBracket,
            ']' => Tok::RBracket,
            ',' => Tok::Comma,
            ':' => Tok::Colon,
            '.' => Tok::Dot,
            '?' => Tok::Question,
            ';' => Tok::Semi,
            '@' => Tok::At,
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '*' => Tok::Star,
            '/' => Tok::Slash,
            '%' => Tok::Percent,
            '=' => Tok::Assign,
            '<' => Tok::Lt,
            '>' => Tok::Gt,
            '&' => Tok::Amp,
            '|' => Tok::Pipe,
            '^' => Tok::Caret,
            '~' => Tok::Tilde,
            '!' => return self.error(start, "Kestrel uses `not`, not `!`"),
            other => return self.error(start, format!("unexpected character `{other}`")),
        };
        self.push(tok, start);
    }
}
