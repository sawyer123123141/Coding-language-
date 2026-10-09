use std::collections::HashMap;
use std::path::Path;

use kestrel_syntax::ast::Item;
use kestrel_syntax::link::{link, Module};
use kestrel_syntax::{parse_at, render, render_short, Diagnostic, LineIndex, Span};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DiagStyle {
    Full,
    Short,
}

/// One source file of the program, laid out at `base` in a shared span space.
struct File {
    path: String,
    src: String,
    base: u32,
}

/// Parses, links, checks and lowers the program whose entry file is `path`
/// (with text `src`), loading the modules it `use`s from the same folder.
/// Prints every diagnostic to stderr and returns `None` when there are errors.
pub fn compile(path: &str, src: &str, style: DiagStyle) -> Option<kestrel_kir::Module> {
    let (files, modules, mut diags) = load(path, src);
    if diags.iter().any(Diagnostic::is_error) {
        report(&files, &diags, style);
        return None;
    }
    let (program, link_diags) = link(modules);
    diags.extend(link_diags);
    let tables = if diags.iter().any(Diagnostic::is_error) {
        None
    } else {
        let (tables, check_diags) = kestrel_check::check(&program);
        diags.extend(check_diags);
        tables
    };
    report(&files, &diags, style);
    let tables = tables?;
    let parts: Vec<(u32, &str)> = files.iter().map(|f| (f.base, f.src.as_str())).collect();
    let mut module = kestrel_kir::lower(&program, &tables, &LineIndex::for_files(&parts));
    kestrel_kir::inline::inline_small(&mut module);
    Some(module)
}

/// Parses the entry file and, transitively, every module it uses. A module
/// `m` is `m.kes` next to the file that uses it; each file is parsed once.
fn load(path: &str, src: &str) -> (Vec<File>, Vec<Module>, Vec<Diagnostic>) {
    let mut files = vec![File { path: path.to_string(), src: src.to_string(), base: 0 }];
    let mut by_name: HashMap<String, usize> = HashMap::from([(stem(path), 0)]);
    let (mut modules, mut diags) = (Vec::new(), Vec::new());
    let (mut next_expr, mut next_bind) = (0, 0);
    let mut i = 0;
    while i < files.len() {
        let (program, file_diags) = parse_at(&files[i].src, files[i].base, next_expr, next_bind);
        (next_expr, next_bind) = (program.expr_count, program.bind_count);
        diags.extend(file_diags);
        let dir = Path::new(&files[i].path).parent().map(Path::to_path_buf).unwrap_or_default();
        for item in &program.items {
            let Item::Use(u) = item else { continue };
            for c in &u.clauses {
                if by_name.contains_key(&c.module) {
                    continue;
                }
                // A missing file is reported by `link`, which knows where it was named.
                let file = dir.join(format!("{}.kes", c.module));
                if let Ok(text) = std::fs::read_to_string(&file) {
                    let last = files.last().expect("the entry file");
                    let base = last.base + last.src.len() as u32 + 1;
                    by_name.insert(c.module.clone(), files.len());
                    files.push(File { path: file.to_string_lossy().into_owned(), src: text, base });
                }
            }
        }
        modules.push(Module { name: stem(&files[i].path), program });
        i += 1;
    }
    (files, modules, diags)
}

fn stem(path: &str) -> String {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Prints each diagnostic against the file its span falls in.
fn report(files: &[File], diags: &[Diagnostic], style: DiagStyle) {
    for d in diags {
        let f = files.iter().rev().find(|f| f.base <= d.span.start).unwrap_or(&files[0]);
        let mut local = d.clone();
        local.span = Span { start: d.span.start - f.base, end: d.span.end.saturating_sub(f.base) };
        match style {
            DiagStyle::Full => eprint!("{}", render(&f.src, &f.path, &local)),
            DiagStyle::Short => eprintln!("{}", render_short(&f.src, &local)),
        }
    }
    let errors = diags.iter().filter(|d| d.is_error()).count();
    if errors > 0 && style == DiagStyle::Full {
        eprintln!("{errors} error{}", if errors == 1 { "" } else { "s" });
    }
}
