//! Moving what lowering found (at the last open or save) past an edit, line by line: lines
//! before and after the edited ones keep their findings, moved by the lines the edit added or
//! removed; the edited lines lose theirs until the next save.

use lsp_types::Diagnostic;

use crate::lens::LensLine;

pub struct LineShift {
    /// The lines both texts start with.
    prefix: u32,
    /// The lines both texts end with (not overlapping `prefix`).
    suffix: u32,
    old_lines: u32,
    new_lines: u32,
}

impl LineShift {
    pub fn new(old: &str, new: &str) -> Self {
        let (a, b): (Vec<&str>, Vec<&str>) = (old.split('\n').collect(), new.split('\n').collect());
        let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let max = a.len().min(b.len()) - prefix;
        let suffix = a.iter().rev().zip(b.iter().rev()).take(max).take_while(|(x, y)| x == y).count();
        LineShift { prefix: prefix as u32, suffix: suffix as u32, old_lines: a.len() as u32, new_lines: b.len() as u32 }
    }

    /// Where old 0-based line `l` is now, unless the edit changed it.
    pub fn line(&self, l: u32) -> Option<u32> {
        if l < self.prefix {
            Some(l)
        } else if l >= self.old_lines - self.suffix {
            Some(l + self.new_lines - self.old_lines)
        } else {
            None
        }
    }

    pub fn lens(&self, lens: &mut Vec<LensLine>) {
        lens.retain_mut(|l| match self.line(l.line) {
            Some(n) => {
                l.line = n;
                true
            }
            None => false,
        });
    }

    pub fn diagnostics(&self, diags: &mut Vec<Diagnostic>) {
        diags.retain_mut(|d| match (self.line(d.range.start.line), self.line(d.range.end.line)) {
            (Some(s), Some(e)) => {
                d.range.start.line = s;
                d.range.end.line = e;
                true
            }
            _ => false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::LineShift;

    #[test]
    fn lines_after_an_insertion_move_down() {
        let s = LineShift::new("a\nb\nc\n", "a\nnew\nb\nc\n");
        assert_eq!((s.line(0), s.line(1), s.line(2)), (Some(0), Some(2), Some(3)));
    }

    #[test]
    fn an_edited_line_loses_its_place() {
        let s = LineShift::new("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!((s.line(0), s.line(1), s.line(2)), (Some(0), None, Some(2)));
        let s = LineShift::new("a\nb\nc\n", "a\nc\n");
        assert_eq!((s.line(0), s.line(1), s.line(2)), (Some(0), None, Some(1)));
    }
}
