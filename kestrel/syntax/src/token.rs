use crate::span::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(u64),
    Float(f64),
    Str(String),
    /// A string with `{...}` interpolation.
    StrParts(Vec<StrPart>),
    // keywords
    Pure, Let, Var, If, Else, While, For, From, To, In, Par, Match, Struct, Enum, Arena,
    Return, Break, Continue, And, Or, Not, True, False, NoneKw, Use, Pub, SelfKw,
    // punctuation
    LParen, RParen, LBrace, RBrace, LBracket, RBracket, Comma, Colon, Dot, Arrow, Question,
    Semi, Newline,
    // operators
    Plus, Minus, Star, Slash, Percent, Assign, EqEq, NotEq, Lt, Gt, Le, Ge,
    PlusEq, MinusEq, StarEq, SlashEq, PercentEq,
    Amp, Pipe, Caret, Tilde, Shl, Shr, AmpEq, PipeEq, CaretEq, ShlEq, ShrEq,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StrPart {
    Lit(String),
    /// Source text of an interpolated expression and its byte offset in the file.
    Code { text: String, start: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

pub fn keyword(s: &str) -> Option<Tok> {
    Some(match s {
        "pure" => Tok::Pure,
        "let" => Tok::Let,
        "var" => Tok::Var,
        "if" => Tok::If,
        "else" => Tok::Else,
        "while" => Tok::While,
        "for" => Tok::For,
        "from" => Tok::From,
        "to" => Tok::To,
        "in" => Tok::In,
        "par" => Tok::Par,
        "match" => Tok::Match,
        "struct" => Tok::Struct,
        "enum" => Tok::Enum,
        "arena" => Tok::Arena,
        "return" => Tok::Return,
        "break" => Tok::Break,
        "continue" => Tok::Continue,
        "and" => Tok::And,
        "or" => Tok::Or,
        "not" => Tok::Not,
        "true" => Tok::True,
        "false" => Tok::False,
        "none" => Tok::NoneKw,
        "use" => Tok::Use,
        "pub" => Tok::Pub,
        "self" => Tok::SelfKw,
        _ => return None,
    })
}

impl Tok {
    /// A line ending in this token continues onto the next line.
    pub fn continues_line(&self) -> bool {
        use Tok::*;
        matches!(
            self,
            Plus | Minus | Star | Slash | Percent | Assign | EqEq | NotEq | Lt | Gt | Le | Ge
                | PlusEq | MinusEq | StarEq | SlashEq | PercentEq | And | Or | Comma | Arrow
                | Dot | LBrace | Amp | Pipe | Caret | Tilde | Shl | Shr | AmpEq | PipeEq | CaretEq
                | ShlEq | ShrEq
        )
    }

    /// How the token reads in an error message.
    pub fn describe(&self) -> String {
        use Tok::*;
        match self {
            Ident(s) => format!("name `{s}`"),
            Int(n) => format!("number `{n}`"),
            Float(f) => format!("number `{f}`"),
            Str(_) | StrParts(_) => "a string".into(),
            Newline => "end of line".into(),
            Eof => "end of file".into(),
            other => format!("`{}`", other.symbol()),
        }
    }

    fn symbol(&self) -> &'static str {
        use Tok::*;
        match self {
            Pure => "pure", Let => "let", Var => "var", If => "if", Else => "else",
            While => "while", For => "for", From => "from", To => "to", In => "in",
            Par => "par", Match => "match", Struct => "struct", Enum => "enum",
            Arena => "arena", Return => "return", Break => "break", Continue => "continue",
            And => "and", Or => "or", Not => "not", True => "true", False => "false", NoneKw => "none",
            Use => "use", Pub => "pub", SelfKw => "self",
            LParen => "(", RParen => ")", LBrace => "{", RBrace => "}", LBracket => "[",
            RBracket => "]", Comma => ",", Colon => ":", Dot => ".", Arrow => "->",
            Question => "?", Semi => ";", Plus => "+", Minus => "-", Star => "*",
            Slash => "/", Percent => "%", Assign => "=", EqEq => "==", NotEq => "!=",
            Lt => "<", Gt => ">", Le => "<=", Ge => ">=", PlusEq => "+=", MinusEq => "-=",
            StarEq => "*=", SlashEq => "/=", PercentEq => "%=",
            Amp => "&", Pipe => "|", Caret => "^", Tilde => "~", Shl => "<<", Shr => ">>",
            AmpEq => "&=", PipeEq => "|=", CaretEq => "^=", ShlEq => "<<=", ShrEq => ">>=",
            Ident(_) | Int(_) | Float(_) | Str(_) | StrParts(_) | Newline | Eof => "",
        }
    }
}
