use kestrel_syntax::{parse, render, render_short, Diagnostic, LineIndex};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DiagStyle {
    Full,
    Short,
}

/// Parses, checks and lowers `src`, printing every diagnostic to stderr.
/// Returns `None` when there are errors. Checking is skipped if parsing failed.
pub fn compile(path: &str, src: &str, style: DiagStyle) -> Option<kestrel_kir::Module> {
    let (program, mut diags) = parse(src);
    let tables = if diags.iter().any(Diagnostic::is_error) {
        None
    } else {
        let (tables, check_diags) = kestrel_check::check(&program);
        diags.extend(check_diags);
        tables
    };
    report(path, src, &diags, style);
    let tables = tables?;
    Some(kestrel_kir::lower(&program, &tables, &LineIndex::new(src)))
}

fn report(path: &str, src: &str, diags: &[Diagnostic], style: DiagStyle) {
    for d in diags {
        match style {
            DiagStyle::Full => eprint!("{}", render(src, path, d)),
            DiagStyle::Short => eprintln!("{}", render_short(src, d)),
        }
    }
    let errors = diags.iter().filter(|d| d.is_error()).count();
    if errors > 0 && style == DiagStyle::Full {
        eprintln!("{errors} error{}", if errors == 1 { "" } else { "s" });
    }
}
