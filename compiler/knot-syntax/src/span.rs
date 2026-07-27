//! Source positions. AST nodes own their data and carry a `Span` rather than
//! borrowing a slice of the source — no lifetimes on AST types, at the cost of a
//! few more allocations. Simplicity over performance, per the project's stated
//! priorities.

/// A byte-offset range into the original source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        Span { start, end }
    }
}

/// Wraps a value with the span of source text it was parsed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spanned<T> {
    pub span: Span,
    pub node: T,
}

impl<T> Spanned<T> {
    pub fn new(span: Span, node: T) -> Self {
        Spanned { span, node }
    }
}

/// Tracks position within the source — byte offset plus 1-based line/column — updated
/// incrementally as bytes are consumed. `col` is what `ParseState::check_indent` and
/// `check_aligned` compare against a block's reference indent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub offset: u32,
    pub line: u32,
    pub col: u32,
}

impl Cursor {
    pub fn start() -> Self {
        Cursor {
            offset: 0,
            line: 1,
            col: 1,
        }
    }

    /// Advance past a single byte, updating line/col bookkeeping. Only valid to call
    /// with the byte actually at `self.offset` in the source.
    pub fn advance(&mut self, byte: u8) {
        self.offset += 1;
        if byte == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_starts_at_one_one() {
        let c = Cursor::start();
        assert_eq!((c.offset, c.line, c.col), (0, 1, 1));
    }

    #[test]
    fn cursor_advances_column_on_normal_byte() {
        let mut c = Cursor::start();
        c.advance(b'x');
        assert_eq!((c.offset, c.line, c.col), (1, 1, 2));
    }

    #[test]
    fn cursor_advances_line_and_resets_column_on_newline() {
        let mut c = Cursor::start();
        c.advance(b'x');
        c.advance(b'\n');
        assert_eq!((c.offset, c.line, c.col), (2, 2, 1));
    }
}
