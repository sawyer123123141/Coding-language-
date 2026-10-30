use crate::token::{Tok, Token};

/// Drops newlines that don't end a statement (see the continuation rules above).
pub fn filter(raw: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(raw.len());
    let mut brackets: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let t = &raw[i];
        if t.tok == Tok::Newline {
            let mut j = i + 1;
            while j < raw.len() && raw[j].tok == Tok::Newline {
                j += 1;
            }
            let in_parens = matches!(brackets.last(), Some(Tok::LParen) | Some(Tok::LBracket));
            // `n++` and `n--` end their line, so their message doesn't swallow the next one.
            let prev_continues = out.last().is_none_or(|p| p.tok.continues_line()) && !ends_in_step(&out);
            let next_continues = matches!(
                raw.get(j).map(|t| &t.tok),
                None | Some(Tok::Else) | Some(Tok::Dot) | Some(Tok::RBrace) | Some(Tok::Eof)
            );
            if !(in_parens || prev_continues || next_continues) {
                out.push(t.clone());
            }
            i = j;
            continue;
        }
        match t.tok {
            Tok::LParen | Tok::LBracket | Tok::LBrace => brackets.push(t.tok.clone()),
            Tok::RParen | Tok::RBracket | Tok::RBrace => {
                brackets.pop();
            }
            _ => {}
        }
        out.push(t.clone());
        i += 1;
    }
    out
}

/// Whether `out` ends in `++` or `--` written together: another language's step, which ends a line.
fn ends_in_step(out: &[Token]) -> bool {
    matches!(out, [.., a, b] if a.span.end == b.span.start && matches!((&a.tok, &b.tok), (Tok::Plus, Tok::Plus) | (Tok::Minus, Tok::Minus)))
}
