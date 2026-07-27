//! Converts byte offsets into 1-based (line, column) pairs for display — e.g.
//! in error messages. Kept entirely separate from `Span`/`Cursor`: spans stay
//! cheap (`Copy`, no source dependency) for use throughout parsing, and the
//! line/col conversion only happens when something actually needs to show a
//! position to a human. Same idea as rust-analyzer's `LineIndex`.

/// Built once per source string; lookups are O(log lines).
pub struct LineIndex {
    /// Byte offset of the start of each line. `line_starts[0] == 0` always.
    line_starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (i, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        LineIndex { line_starts }
    }

    /// 1-based (line, column) for a byte offset. Column is a byte offset
    /// within the line, not a Unicode-aware "character" column — consistent
    /// with how `Cursor` counts columns during parsing.
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,
            // `Err(idx)` is the insertion point; since `line_starts[0] == 0`
            // and `offset` is unsigned, `idx` is never 0 here.
            Err(idx) => idx - 1,
        };
        let line = (line_idx + 1) as u32;
        let col = offset - self.line_starts[line_idx] + 1;
        (line, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_no_newline() {
        let index = LineIndex::new("hello");
        assert_eq!(index.line_col(0), (1, 1));
        assert_eq!(index.line_col(3), (1, 4));
    }

    #[test]
    fn multiple_lines() {
        // offsets: a=0 b=1 \n=2 c=3 d=4 \n=5 e=6 f=7
        let index = LineIndex::new("ab\ncd\nef");
        assert_eq!(index.line_col(0), (1, 1)); // 'a'
        assert_eq!(index.line_col(1), (1, 2)); // 'b'
        assert_eq!(index.line_col(3), (2, 1)); // 'c'
        assert_eq!(index.line_col(4), (2, 2)); // 'd'
        assert_eq!(index.line_col(6), (3, 1)); // 'e'
        assert_eq!(index.line_col(7), (3, 2)); // 'f'
    }

    #[test]
    fn offset_exactly_at_line_start() {
        let index = LineIndex::new("ab\ncd");
        assert_eq!(index.line_col(3), (2, 1));
    }

    #[test]
    fn offset_at_end_of_file() {
        let index = LineIndex::new("ab\ncd");
        assert_eq!(index.line_col(5), (2, 3)); // one past the last 'd'
    }

    #[test]
    fn empty_source() {
        let index = LineIndex::new("");
        assert_eq!(index.line_col(0), (1, 1));
    }

    #[test]
    fn trailing_newline() {
        let index = LineIndex::new("ab\n");
        assert_eq!(index.line_col(2), (1, 3)); // the '\n' itself, still line 1
        assert_eq!(index.line_col(3), (2, 1)); // EOF, start of a (nonexistent) line 2
    }
}
