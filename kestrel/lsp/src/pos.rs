//! Byte offsets (the compiler's spans) to LSP positions and back. An LSP position is a line and
//! a column counted in UTF-16 code units, so `é` counts 1 and `🦅` counts 2, whatever their
//! length in UTF-8.

use lsp_types::{Position, Range};

/// Where each line of one text starts.
pub struct LineMap<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineMap<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.bytes().enumerate().filter(|&(_, b)| b == b'\n').map(|(i, _)| i + 1));
        LineMap { text, starts }
    }

    /// The position of byte `offset` (clamped to the text, and moved back to a character boundary).
    pub fn position(&self, offset: usize) -> Position {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let col: usize = self.text[self.starts[line]..offset].chars().map(char::len_utf16).sum();
        Position::new(line as u32, col as u32)
    }

    pub fn range(&self, start: usize, end: usize) -> Range {
        Range::new(self.position(start), self.position(end))
    }

    /// The byte offset of `p`. A column past the end of its line is the line's end (before its
    /// line ending); a line past the end of the text is the text's end. A column inside a
    /// character's two UTF-16 units is that character's start.
    pub fn offset(&self, p: Position) -> usize {
        let Some(&start) = self.starts.get(p.line as usize) else { return self.text.len() };
        let end = self.starts.get(p.line as usize + 1).map_or(self.text.len(), |&e| e - 1);
        let line = self.text[start..end].strip_suffix('\r').unwrap_or(&self.text[start..end]);
        let mut units = 0;
        for (i, c) in line.char_indices() {
            units += c.len_utf16();
            if units > p.character as usize {
                return start + i;
            }
        }
        start + line.len()
    }

    /// The byte offset where 0-based line `line` starts, and where it ends (before its line ending).
    pub fn line_bounds(&self, line: usize) -> Option<(usize, usize)> {
        let start = *self.starts.get(line)?;
        let end = self.starts.get(line + 1).map_or(self.text.len(), |&e| e - 1);
        let text = &self.text[start..end];
        Some((start, start + text.strip_suffix('\r').map_or(text.len(), str::len)))
    }
}

#[cfg(test)]
mod tests {
    use lsp_types::Position;

    use super::LineMap;

    #[test]
    fn ascii_offsets_round_trip() {
        let text = "main() {\n    let x = 1\n}\n";
        let m = LineMap::new(text);
        assert_eq!(m.position(0), Position::new(0, 0));
        assert_eq!(m.position(13), Position::new(1, 4));
        assert_eq!(m.offset(Position::new(1, 4)), 13);
        assert_eq!(m.position(text.len()), Position::new(3, 0));
        for i in 0..=text.len() {
            assert_eq!(m.offset(m.position(i)), i, "offset {i}");
        }
    }

    #[test]
    fn columns_count_utf16_units() {
        // `é` is 2 bytes and 1 unit; `🦅` is 4 bytes and 2 units.
        let text = "let s = \"é🦅\" + x\n";
        let m = LineMap::new(text);
        let x = text.find('x').unwrap();
        assert_eq!(x, 19);
        assert_eq!(m.position(x), Position::new(0, 16));
        assert_eq!(m.offset(Position::new(0, 16)), x);
        // The quote after the eagle.
        let quote = text.rfind('"').unwrap();
        assert_eq!(m.position(quote), Position::new(0, 12));
        // The middle of the eagle's surrogate pair is its start.
        assert_eq!(m.offset(Position::new(0, 11)), text.find('🦅').unwrap());
    }

    #[test]
    fn a_position_inside_a_character_moves_to_its_start() {
        let text = "é";
        let m = LineMap::new(text);
        assert_eq!(m.position(1), Position::new(0, 0));
        assert_eq!(m.position(2), Position::new(0, 1));
    }

    #[test]
    fn positions_past_the_end_are_clamped() {
        let text = "ab\r\ncd";
        let m = LineMap::new(text);
        // Past the end of line 0 is before its `\r\n`.
        assert_eq!(m.offset(Position::new(0, 99)), 2);
        assert_eq!(m.offset(Position::new(1, 1)), 5);
        assert_eq!(m.offset(Position::new(7, 0)), text.len());
        assert_eq!(m.position(99), Position::new(1, 2));
        assert_eq!(m.line_bounds(0), Some((0, 2)));
        assert_eq!(m.line_bounds(1), Some((4, 6)));
        assert_eq!(m.line_bounds(2), None);
    }
}
