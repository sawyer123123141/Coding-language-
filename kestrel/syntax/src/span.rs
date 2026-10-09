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
}

impl LineIndex {
    pub fn new(src: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        LineIndex { line_starts }
    }

    /// 1-based line containing `offset`.
    pub fn line(&self, offset: u32) -> u32 {
        match self.line_starts.binary_search(&offset) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
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
