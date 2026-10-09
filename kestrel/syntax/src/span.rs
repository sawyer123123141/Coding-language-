/// A byte range in one source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start: start as u32, end: end as u32 }
    }

    /// Smallest span covering both.
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

/// Maps byte offsets to 1-based line and column numbers.
pub struct LineIndex {
    line_starts: Vec<u32>,
    /// Per file: where its bytes start, and the index of its first line.
    files: Vec<(u32, usize)>,
}

impl LineIndex {
    pub fn new(src: &str) -> Self {
        Self::for_files(&[(0, src)])
    }

    /// For several files laid out at the given byte offsets (see
    /// `parse_at`); `line` then counts within each offset's own file.
    pub fn for_files(files: &[(u32, &str)]) -> Self {
        let mut line_starts = Vec::new();
        let mut firsts = Vec::new();
        for &(base, src) in files {
            firsts.push((base, line_starts.len()));
            line_starts.push(base);
            for (i, b) in src.bytes().enumerate() {
                if b == b'\n' {
                    line_starts.push(base + i as u32 + 1);
                }
            }
        }
        LineIndex { line_starts, files: firsts }
    }

    /// 1-based line containing `offset`, within its file.
    pub fn line(&self, offset: u32) -> u32 {
        let global = match self.line_starts.binary_search(&offset) {
            Ok(i) => i + 1,
            Err(i) => i,
        };
        let first = self.files.iter().rev().find(|(base, _)| *base <= offset).map_or(0, |&(_, first)| first);
        (global - first) as u32
    }

    /// 1-based (line, column); column counts characters, not bytes.
    pub fn line_col(&self, src: &str, offset: u32) -> (u32, u32) {
        let line = self.line(offset);
        let start = self.line_starts[(line - 1) as usize] as usize;
        let col = src[start..offset as usize].chars().count() as u32 + 1;
        (line, col)
    }

    /// Text of a 1-based line without its line ending.
    pub fn line_text<'a>(&self, src: &'a str, line: u32) -> &'a str {
        let start = self.line_starts[(line - 1) as usize] as usize;
        let end = self.line_starts.get(line as usize).map(|&e| e as usize).unwrap_or(src.len());
        src[start..end].trim_end_matches(['\n', '\r'])
    }
}
