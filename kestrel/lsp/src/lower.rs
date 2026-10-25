//! The step after checking, run when a document is opened or saved (it lowers the program, so
//! not on every keystroke): every `@prove` contract is decided and each `@assume` that removes
//! no check gets a warning, as `kestrel check` does, and the cost lens reads the report.

use std::path::Path;

use kestrel_driver::{compile_as_reading, prove, DiagStyle, Read, Source};
use kestrel_kir::range::{remove_checks, report, Arith};
use kestrel_syntax::Diagnostic;

use crate::lens::{self, LensLine};

pub struct Lowered {
    pub sources: Vec<Source>,
    /// The contracts' errors and the assumptions' warnings, spans in `sources`' shared space.
    pub diags: Vec<Diagnostic>,
    /// What the cost lens found in the entry file, when asked for.
    pub lens: Vec<LensLine>,
}

/// Lowers the program whose entry file is `path`, for a checked build (`checked`) or a
/// `--release` one. `None` when it doesn't compile: the check's errors say why.
pub fn lower(path: &Path, src: &str, needs_main: bool, read: Read, checked: bool, want_lens: bool) -> Option<Lowered> {
    let (compiled, _) = compile_as_reading(&path.to_string_lossy(), src, DiagStyle::Short, needs_main, read);
    let mut c = compiled?;
    let proved = prove(&mut c, checked, true, DiagStyle::Short);
    let lens = if want_lens {
        let analysis = proved.analysis.unwrap_or_else(|| remove_checks(&mut c.module, Arith::of(checked)));
        let r = report::build(&c.module, &analysis);
        lens::lines(&r, 0)
    } else {
        Vec::new()
    };
    Some(Lowered { sources: c.sources().to_vec(), diags: proved.diags, lens })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::lower;

    #[test]
    fn two_copies_of_a_generic_give_one_lens_entry_per_line() {
        // Both copies of `total` hold the same overflow check on line 4: one entry on that line.
        let src = "pure total[T](xs: list[T]) -> int {\n    var n = 0\n    for x in xs {\n        n += 1\n    }\n    n\n}\n\npub run(xs: list[int], ys: list[float]) -> int {\n    total(xs) + total(ys)\n}\n\nmain() {}\n";
        let read = |_: &Path| None;
        let lowered = lower(Path::new("lens_copies.kes"), src, true, &read, true, true).expect("compiles");
        let mut lines: Vec<u32> = lowered.lens.iter().map(|l| l.line).collect();
        let before = lines.len();
        lines.dedup();
        assert_eq!(lines.len(), before, "a line twice: {:?}", lowered.lens.iter().map(|l| l.line).collect::<Vec<_>>());
        let on_four = lowered.lens.iter().find(|l| l.line == 3).expect("an entry on line 4");
        assert_eq!(on_four.items.len(), 1, "{:?}", on_four.items);
    }
}
