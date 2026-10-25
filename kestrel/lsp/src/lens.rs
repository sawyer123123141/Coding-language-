//! The cost lens: what `kestrel check --report` says about each line (every check range analysis
//! left, every allocation, every other way to fail and every call that isn't inlined, with why),
//! shown on the line itself, with the change that would remove it when there is a simple one.

use kestrel_kir::range::render::label;
use kestrel_kir::range::report::{FuncReport, ItemKind, Report};
use kestrel_kir::range::sites::CheckKind;
use kestrel_kir::unpack_line;

/// What the lens found on one line.
#[derive(Clone, Debug, PartialEq)]
pub struct LensLine {
    /// 0-based, in the file.
    pub line: u32,
    pub items: Vec<LensItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LensItem {
    /// "bounds check", "allocation", "can fail", "call", or "range analysis".
    pub label: String,
    /// Why, as the report says it.
    pub text: String,
    /// The change that would remove it, when the lens knows one.
    pub fix: Option<String>,
}

impl LensItem {
    /// The item as one line of text.
    pub fn sentence(&self) -> String {
        match &self.fix {
            Some(fix) => format!("{}: {} ({fix})", self.label, self.text),
            None => format!("{}: {}", self.label, self.text),
        }
    }
}

/// The lines of file `file` (an index of `Module::files`) the report has something to say about,
/// in order; each item once on its line.
pub fn lines(r: &Report, file: u32) -> Vec<LensLine> {
    let mut out: Vec<LensLine> = Vec::new();
    for f in &r.funcs {
        collect(f, file, &mut out);
    }
    out.sort_by_key(|l| l.line);
    out
}

fn collect(f: &FuncReport, file: u32, out: &mut Vec<LensLine>) {
    if f.gave_up && !f.never_called {
        let (line, home) = unpack_line(f.lines.0);
        if home == file {
            let text = format!("range analysis stopped early in `{}` ({}), so every check in it stays", f.name, f.stopped);
            push(out, line, LensItem { label: "range analysis".into(), text, fix: None });
        }
    }
    for i in &f.items {
        let (line, home) = unpack_line(i.line);
        if home == file {
            push(out, line, LensItem { label: label(i.kind).into(), text: i.text.clone(), fix: fix(i.kind, &i.text) });
        }
    }
    for (_, body) in &f.par_bodies {
        collect(body, file, out);
    }
}

/// Adds `item` to 1-based line `line`, unless it is already there.
fn push(out: &mut Vec<LensLine>, line: u32, item: LensItem) {
    let Some(line) = line.checked_sub(1) else { return };
    match out.iter_mut().find(|l| l.line == line) {
        Some(l) if l.items.contains(&item) => {}
        Some(l) => l.items.push(item),
        None => out.push(LensLine { line, items: vec![item] }),
    }
}

/// The change that removes a check, for the shapes where it is plain: an index that is a name,
/// a divisor that is a name, an integer `+ - *` that may be meant to wrap.
fn fix(kind: ItemKind, text: &str) -> Option<String> {
    let what = quoted(text)?;
    match kind {
        ItemKind::Check(CheckKind::Bounds) => {
            let (list, index) = what.strip_suffix(']')?.split_once('[')?;
            (is_path(list) && is_name(index)).then(|| {
                format!("if `{index}` is always inside `{list}`, `@assume({index} >= 0 and {index} < {list}.len)` on the line before removes it")
            })
        }
        ItemKind::Check(CheckKind::Division) => {
            let (_, divisor) = top_level_op(what, &["/", "%"])?;
            is_name(divisor).then(|| format!("if `{divisor}` is never 0, `@assume({divisor} != 0)` on the line before removes it"))
        }
        ItemKind::Check(CheckKind::Overflow) => {
            let int = ["`int`", "`i64`", "`u8`", "`ivec4`"].iter().any(|t| text.contains(&format!("may exceed {t}")));
            let (op, _) = top_level_op(what, &["+", "-", "*"])?;
            int.then(|| format!("if wrapping around is what you want, `{op}%` wraps and is never checked"))
        }
        _ => None,
    }
}

/// The text between the first pair of backticks.
fn quoted(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('`')?;
    Some(&rest[..rest.find('`')?])
}

/// The operator (one of `ops`, written with a space on each side) that splits `e` outside any
/// brackets, when there is exactly one such operator, and the right-hand side.
fn top_level_op<'a>(e: &'a str, ops: &[&'static str]) -> Option<(&'static str, &'a str)> {
    let (mut depth, mut found) = (0i32, None);
    let bytes = e.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b' ' if depth == 0 && i + 2 < bytes.len() && bytes[i + 2] == b' ' => {
                let op = &e[i + 1..i + 2];
                if ["+", "-", "*", "/", "%", "<", ">", "&", "|", "^"].contains(&op) {
                    if found.is_some() {
                        return None;
                    }
                    found = Some((op, &e[i + 3..]));
                }
            }
            _ => {}
        }
    }
    let (op, rhs) = found?;
    ops.iter().find(|o| **o == op).map(|o| (*o, rhs))
}

fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_ascii_alphabetic()) && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn is_path(s: &str) -> bool {
    s.split('.').all(is_name)
}

#[cfg(test)]
mod tests {
    use kestrel_kir::range::report::ItemKind;
    use kestrel_kir::range::sites::CheckKind;

    use super::fix;

    #[test]
    fn a_bounds_check_on_a_named_index_suggests_an_assumption() {
        let f = fix(ItemKind::Check(CheckKind::Bounds), "`xs[k]` may be outside `xs` (`k` can be any `int`)").unwrap();
        assert!(f.contains("`@assume(k >= 0 and k < xs.len)`"), "{f}");
        let f = fix(ItemKind::Check(CheckKind::Bounds), "`w.cells[i]` may be outside `w.cells`").unwrap();
        assert!(f.contains("`@assume(i >= 0 and i < w.cells.len)`"), "{f}");
        assert_eq!(fix(ItemKind::Check(CheckKind::Bounds), "`xs[i + 1]` may be outside `xs`"), None);
    }

    #[test]
    fn a_division_by_a_name_suggests_an_assumption() {
        let f = fix(ItemKind::Check(CheckKind::Division), "`a / b` may divide by zero").unwrap();
        assert!(f.contains("`@assume(b != 0)`"), "{f}");
        assert_eq!(fix(ItemKind::Check(CheckKind::Division), "`a / (b - 1)` may divide by zero"), None);
    }

    #[test]
    fn an_integer_overflow_suggests_the_wrapping_operator() {
        let f = fix(ItemKind::Check(CheckKind::Overflow), "`s + xs[i]` may exceed `int` (`s` can be any `int`)").unwrap();
        assert!(f.contains("`+%` wraps"), "{f}");
        let f = fix(ItemKind::Check(CheckKind::Overflow), "`(a + b) * c` may exceed `i64`").unwrap();
        assert!(f.contains("`*%` wraps"), "{f}");
        // Two operators at the top: which one overflows isn't said.
        assert_eq!(fix(ItemKind::Check(CheckKind::Overflow), "`a + b + c` may exceed `int`"), None);
        assert_eq!(fix(ItemKind::Allocation, "`add` may grow `ys`"), None);
    }
}
