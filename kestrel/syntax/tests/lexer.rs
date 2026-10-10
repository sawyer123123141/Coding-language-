use kestrel_syntax::lexer::lex;
use kestrel_syntax::token::{StrPart, Tok};

fn toks(src: &str) -> Vec<Tok> {
    let (t, d) = lex(src);
    assert!(d.is_empty(), "unexpected diagnostics: {d:?}");
    t.into_iter().map(|t| t.tok).collect()
}

fn errors(src: &str) -> Vec<String> {
    lex(src).1.into_iter().map(|d| d.message).collect()
}

fn id(s: &str) -> Tok {
    Tok::Ident(s.into())
}

#[test]
fn keywords_and_identifiers() {
    assert_eq!(
        toks("pure let var par square self"),
        vec![Tok::Pure, Tok::Let, Tok::Var, Tok::Par, id("square"), Tok::SelfKw, Tok::Eof]
    );
    assert_eq!(toks("pure native natives"), vec![Tok::Pure, Tok::Native, id("natives"), Tok::Eof]);
}

#[test]
fn numbers() {
    assert_eq!(
        toks("42 1_000 0xFF 2.5 5"),
        vec![Tok::Int(42), Tok::Int(1000), Tok::Int(255), Tok::Float(2.5), Tok::Int(5), Tok::Eof]
    );
}

#[test]
fn strings_with_escapes() {
    assert_eq!(toks(r#""a\n\"b\" \{x\}""#), vec![Tok::Str("a\n\"b\" {x}".into()), Tok::Eof]);
}

#[test]
fn operators() {
    assert_eq!(
        toks("-> == != <= >= += -= *= /= %= = < > ? ;"),
        vec![
            Tok::Arrow, Tok::EqEq, Tok::NotEq, Tok::Le, Tok::Ge, Tok::PlusEq, Tok::MinusEq,
            Tok::StarEq, Tok::SlashEq, Tok::PercentEq, Tok::Assign, Tok::Lt, Tok::Gt,
            Tok::Question, Tok::Semi, Tok::Eof,
        ]
    );
}

#[test]
fn comments_are_skipped() {
    assert_eq!(toks("a // hello\nb"), vec![id("a"), Tok::Newline, id("b"), Tok::Eof]);
}

#[test]
fn newline_after_operator_continues() {
    assert_eq!(
        toks("x = a +\n  b\ny"),
        vec![id("x"), Tok::Assign, id("a"), Tok::Plus, id("b"), Tok::Newline, id("y"), Tok::Eof]
    );
}

#[test]
fn operator_at_line_start_does_not_continue() {
    // This is the trap the no-effect check later catches.
    assert_eq!(
        toks("x = a\n  + b"),
        vec![id("x"), Tok::Assign, id("a"), Tok::Newline, Tok::Plus, id("b"), Tok::Eof]
    );
}

#[test]
fn else_on_next_line_continues() {
    assert_eq!(toks("}\nelse {"), vec![Tok::RBrace, Tok::Else, Tok::LBrace, Tok::Eof]);
}

#[test]
fn newlines_inside_parens_are_dropped_but_not_inside_braces_within_them() {
    assert_eq!(
        toks("f(a,\n b)\n"),
        vec![id("f"), Tok::LParen, id("a"), Tok::Comma, id("b"), Tok::RParen, Tok::Eof]
    );
    assert_eq!(
        toks("f(if c {\nx\ny\n})"),
        vec![
            id("f"), Tok::LParen, Tok::If, id("c"), Tok::LBrace, id("x"), Tok::Newline,
            id("y"), Tok::RBrace, Tok::RParen, Tok::Eof,
        ]
    );
}

#[test]
fn blank_lines_collapse() {
    assert_eq!(toks("\n\na\n\n\nb\n\n"), vec![id("a"), Tok::Newline, id("b"), Tok::Eof]);
}

#[test]
fn helpful_errors() {
    assert_eq!(errors("a && b"), vec!["Kestrel uses `and`, not `&&`"]);
    assert_eq!(errors("a || b"), vec!["Kestrel uses `or`, not `||`"]);
    assert_eq!(errors("!a"), vec!["Kestrel uses `not`, not `!`"]);
    assert_eq!(errors("\"hp {}\""), vec!["empty `{}` in a string; write `\\{` for a brace"]);
    assert_eq!(errors("\"hp {x\""), vec!["unterminated `{` in a string"]);
    assert_eq!(errors("\"a {\"b\"}\""), vec!["unterminated `{` in a string"]);
    assert_eq!(errors("\"abc"), vec!["unterminated string"]);
    assert_eq!(errors("a @ b"), vec!["unexpected character `@`"]);
    assert_eq!(errors("99999999999999999999"), vec!["number is too large"]);
}

#[test]
fn unknown_escape_skips_the_whole_character() {
    // A multi-byte escaped char must not leave the cursor mid-UTF-8 (it used to panic).
    assert_eq!(errors("\"\\é\""), vec!["unknown escape sequence"]);
    assert_eq!(errors("\"\\éx\""), vec!["unknown escape sequence"]);
    // A backslash before a newline or EOF leaves it for the unterminated-string check.
    assert_eq!(errors("\"a\\\nb"), vec!["unknown escape sequence", "unterminated string"]);
    assert_eq!(errors("\"abc\\"), vec!["unknown escape sequence", "unterminated string"]);
}

#[test]
fn diagnostic_spans_stay_inside_the_source() {
    for src in ["\"abc\\", "\"abc", "\"\\é", "@", "a &&", "99999999999999999999", "0x"] {
        for d in lex(src).1 {
            assert!(d.span.start < d.span.end, "empty span for {src:?}: {d:?}");
            assert!(d.span.end as usize <= src.len(), "span past end for {src:?}: {d:?}");
        }
    }
}

#[test]
fn interpolated_strings_keep_code_and_its_offset() {
    assert_eq!(
        toks("\"hp = {hp}!\""),
        vec![
            Tok::StrParts(vec![
                StrPart::Lit("hp = ".into()),
                StrPart::Code { text: "hp".into(), start: 7 },
                StrPart::Lit("!".into()),
            ]),
            Tok::Eof,
        ]
    );
    assert_eq!(
        toks("\"{a.b + 1}{c}\""),
        vec![
            Tok::StrParts(vec![
                StrPart::Code { text: "a.b + 1".into(), start: 2 },
                StrPart::Code { text: "c".into(), start: 11 },
            ]),
            Tok::Eof,
        ]
    );
}

#[test]
fn bit_operators() {
    assert_eq!(
        toks("& | ^ ~ << >> &= |= ^= <<= >>= < <="),
        vec![
            Tok::Amp, Tok::Pipe, Tok::Caret, Tok::Tilde, Tok::Shl, Tok::Shr, Tok::AmpEq, Tok::PipeEq,
            Tok::CaretEq, Tok::ShlEq, Tok::ShrEq, Tok::Lt, Tok::Le, Tok::Eof,
        ]
    );
    assert_eq!(errors("a && b"), vec!["Kestrel uses `and`, not `&&`"]);
}
