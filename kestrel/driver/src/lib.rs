//! Turns a program's entry file into KIR: loads the modules it uses, links,
//! checks, lowers and inlines. Shared by the `kestrel` command and the Godot
//! add-on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use kestrel_syntax::ast::Item;
use kestrel_syntax::link::{link, Module};
use kestrel_syntax::{parse_at, render, render_short, Diagnostic, LineIndex, Span};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DiagStyle {
    Full,
    Short,
}

/// A file the program was built from, and when it last changed (for hot reload).
#[derive(Clone, Debug, PartialEq)]
pub struct SourceFile {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
}

impl SourceFile {
    fn new(path: &Path) -> Self {
        SourceFile { path: path.to_path_buf(), modified: modified(path) }
    }

    /// Whether the file changed (or vanished) since it was read.
    pub fn changed(&self) -> bool {
        modified(&self.path) != self.modified
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub struct Compiled {
    pub module: kestrel_kir::Module,
    /// The entry file first, then each module it uses.
    pub files: Vec<SourceFile>,
}

/// One source file of the program, laid out at `base` in a shared span space.
struct File {
    path: String,
    src: String,
    base: u32,
}

/// Compiles the program whose entry file is `path` (with text `src`). The
/// second result is every diagnostic, rendered as text (empty when clean).
pub fn compile(path: &str, src: &str, style: DiagStyle) -> (Option<Compiled>, String) {
    let (files, modules, mut diags) = load(path, src);
    if diags.iter().any(Diagnostic::is_error) {
        return (None, report(&files, &diags, style));
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
    let text = report(&files, &diags, style);
    let Some(tables) = tables else { return (None, text) };
    let parts: Vec<(u32, &str)> = files.iter().map(|f| (f.base, f.src.as_str())).collect();
    let mut module = kestrel_kir::lower(&program, &tables, &LineIndex::for_files(&parts));
    kestrel_kir::inline::inline_small(&mut module);
    let files = files.iter().map(|f| SourceFile::new(Path::new(&f.path))).collect();
    (Some(Compiled { module, files }), text)
}

/// Reads `path` and compiles it.
pub fn compile_file(path: &Path, style: DiagStyle) -> (Option<Compiled>, String) {
    match std::fs::read_to_string(path) {
        Ok(src) => compile(&path.to_string_lossy(), src.strip_prefix('\u{feff}').unwrap_or(&src), style),
        Err(e) => (None, format!("error: can't read {}: {e}\n", path.display())),
    }
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

/// Renders each diagnostic against the file its span falls in.
fn report(files: &[File], diags: &[Diagnostic], style: DiagStyle) -> String {
    let mut out = String::new();
    for d in diags {
        let f = files.iter().rev().find(|f| f.base <= d.span.start).unwrap_or(&files[0]);
        let mut local = d.clone();
        local.span = Span { start: d.span.start - f.base, end: d.span.end.saturating_sub(f.base) };
        match style {
            DiagStyle::Full => out.push_str(&render(&f.src, &f.path, &local)),
            DiagStyle::Short => {
                out.push_str(&render_short(&f.src, &local));
                out.push('\n');
            }
        }
    }
    let errors = diags.iter().filter(|d| d.is_error()).count();
    if errors > 0 && style == DiagStyle::Full {
        out.push_str(&format!("{errors} error{}\n", if errors == 1 { "" } else { "s" }));
    }
    out
}
