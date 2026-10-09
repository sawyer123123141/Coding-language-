use kestrel_syntax::{render, render_short, Diagnostic, LineIndex, Span};

const SRC: &str = "main() {\n    let x = 5 +\n}\n";

#[test]
fn line_col_is_one_based() {
    let idx = LineIndex::new(SRC);
    assert_eq!(idx.line_col(SRC, 0), (1, 1));
    assert_eq!(idx.line_col(SRC, 13), (2, 5));
    assert_eq!(idx.line(13), 2);
    assert_eq!(idx.line_text(SRC, 2), "    let x = 5 +");
}

#[test]
fn short_render() {
    let d = Diagnostic::error(Span::new(13, 16), "bad let");
    assert_eq!(render_short(SRC, &d), "2:5: error: bad let");
}

#[test]
fn full_render_with_caret_and_help() {
    let d = Diagnostic::error(Span::new(13, 16), "bad let").with_help("try harder");
    let out = render(SRC, "t.kes", &d);
    let expected = "error: bad let\n  --> t.kes:2:5\n   |\n 2 |     let x = 5 +\n   |     ^^^\n   = help: try harder\n";
    assert_eq!(out, expected);
}

#[test]
fn span_to_covers_both() {
    assert_eq!(Span::new(5, 7).to(Span::new(2, 3)), Span::new(2, 7));
}
