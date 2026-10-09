use crate::diag::Diagnostic;
use crate::newlines;
use crate::span::Span;
use crate::token::{keyword, Tok, Token};

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

    fn error(&mut self, start: usize, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(Span::new(start, self.pos.max(start + 1)), msg));
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
                Ok(n) => self.push(Tok::Int(n), start),
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
        loop {
            if self.pos >= self.bytes.len() || self.peek(0) == b'\n' {
                self.error(start, "unterminated string");
                return;
            }
            match self.peek(0) {
                b'"' => {
                    self.pos += 1;
                    self.push(Tok::Str(value), start);
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
                            self.pos += 2;
                            self.error(at, "unknown escape sequence");
                            continue;
                        }
                    };
                    value.push(ch);
                    self.pos += 2;
                }
                b'{' => {
                    let at = self.pos;
                    self.pos += 1;
                    self.error(at, "string interpolation isn't supported yet; escape the brace as `\\{`");
                }
                _ => {
                    let ch = self.src[self.pos..].chars().next().unwrap();
                    value.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn punct(&mut self) {
        let start = self.pos;
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
            _ => None,
        };
        if let Some(tok) = pair {
            self.pos += 2;
            self.push(tok, start);
            return;
        }
        if &two == b"&&" || &two == b"||" {
            self.pos += 2;
            let msg = if two[0] == b'&' { "Kestrel uses `and`, not `&&`" } else { "Kestrel uses `or`, not `||`" };
            self.error(start, msg);
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
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '*' => Tok::Star,
            '/' => Tok::Slash,
            '%' => Tok::Percent,
            '=' => Tok::Assign,
            '<' => Tok::Lt,
            '>' => Tok::Gt,
            '!' => return self.error(start, "Kestrel uses `not`, not `!`"),
            other => return self.error(start, format!("unexpected character `{other}`")),
        };
        self.push(tok, start);
    }
}
