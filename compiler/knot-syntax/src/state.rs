//! Layout is handled inline here rather than via a separate tokenize-then-layout
//! pass: `ParseState` carries a reference indent column, and `check_indent` /
//! `check_aligned` calls at each block-forming construct (`let`, `case`, `do`,
//! top-level decls) enforce it directly. The concept is ported from Elm's
//! `Parse.Space` (not the code) — see `knot-ast-parser-plan.md` §2.

use crate::error::{ErrorKind, ParseError};
use crate::span::{Cursor, Span};

#[derive(Clone)]
pub struct ParseState<'a> {
    pub src: &'a [u8],
    pub pos: Cursor,
    /// Reference column for the innermost open layout block. `None` means no layout
    /// constraint is active yet (e.g. before the first top-level declaration).
    pub indent: Option<u32>,
}

impl<'a> ParseState<'a> {
    pub fn new(src: &'a str) -> Self {
        ParseState { src: src.as_bytes(), pos: Cursor::start(), indent: None }
    }

    pub fn is_eof(&self) -> bool {
        self.pos.offset as usize >= self.src.len()
    }

    pub fn peek(&self) -> Option<u8> {
        self.src.get(self.pos.offset as usize).copied()
    }

    pub fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.src.get(self.pos.offset as usize + ahead).copied()
    }

    /// Consume one byte unconditionally, advancing the cursor.
    pub fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos.advance(byte);
        Some(byte)
    }

    /// Skip whitespace and comments (`--` line comments, nestable `{- -}` block
    /// comments). Both are layout-transparent: they never affect indentation checks,
    /// since `check_indent` / `check_aligned` are always evaluated against the column
    /// of the next *real* token, after this has already run.
    pub fn skip_trivia(&mut self) -> Result<(), ParseError> {
        loop {
            match (self.peek(), self.peek_at(1)) {
                (Some(b' ' | b'\t' | b'\r' | b'\n'), _) => {
                    self.bump();
                }
                (Some(b'-'), Some(b'-')) => {
                    self.bump();
                    self.bump();
                    while let Some(byte) = self.peek() {
                        if byte == b'\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                (Some(b'{'), Some(b'-')) => {
                    self.skip_block_comment()?;
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn skip_block_comment(&mut self) -> Result<(), ParseError> {
        let start = self.pos;
        self.bump(); // '{'
        self.bump(); // '-'
        let mut depth = 1u32;
        while depth > 0 {
            match (self.peek(), self.peek_at(1)) {
                (Some(b'{'), Some(b'-')) => {
                    self.bump();
                    self.bump();
                    depth += 1;
                }
                (Some(b'-'), Some(b'}')) => {
                    self.bump();
                    self.bump();
                    depth -= 1;
                }
                (Some(_), _) => {
                    self.bump();
                }
                (None, _) => {
                    return Err(ParseError::new(
                        ErrorKind::UnclosedBlockComment,
                        Span::new(start.offset, self.pos.offset),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Require the current position to be strictly to the right of `col` — a
    /// continuation of whatever construct opened at `col` (e.g. a wrapped binary-op
    /// chain, or a multi-line expression inside a block).
    pub fn check_indent(&self, col: u32) -> Result<(), ParseError> {
        if self.pos.col > col {
            Ok(())
        } else {
            Err(ParseError::new(
                ErrorKind::IndentViolation { expected_col: col + 1, found_col: self.pos.col },
                Span::new(self.pos.offset, self.pos.offset),
            ))
        }
    }

    /// Require the current position to be exactly at `col` — the next item in a
    /// layout block (another `let` binding, another `case` arm, another top-level
    /// declaration).
    pub fn check_aligned(&self, col: u32) -> Result<(), ParseError> {
        if self.pos.col == col {
            Ok(())
        } else {
            Err(ParseError::new(
                ErrorKind::IndentViolation { expected_col: col, found_col: self.pos.col },
                Span::new(self.pos.offset, self.pos.offset),
            ))
        }
    }

    /// Run `f` with `self.indent` set to the column of the *current* token, restoring
    /// the previous reference indent afterward. This is how a `let`/`case`/`do`/
    /// module block establishes its own layout scope.
    pub fn with_indent<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        let saved = self.indent;
        self.indent = Some(self.pos.col);
        let result = f(self);
        self.indent = saved;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_line_comment() {
        let mut state = ParseState::new("-- a comment\nx");
        state.skip_trivia().unwrap();
        assert_eq!(state.peek(), Some(b'x'));
    }

    #[test]
    fn skips_nested_block_comment() {
        let mut state = ParseState::new("{- outer {- inner -} still outer -}x");
        state.skip_trivia().unwrap();
        assert_eq!(state.peek(), Some(b'x'));
    }

    #[test]
    fn unclosed_block_comment_errors() {
        let mut state = ParseState::new("{- never closed");
        assert!(state.skip_trivia().is_err());
    }

    #[test]
    fn skips_mixed_whitespace_and_comments() {
        let mut state = ParseState::new("   -- one\n\t{- two -}  x");
        state.skip_trivia().unwrap();
        assert_eq!(state.peek(), Some(b'x'));
    }

    #[test]
    fn check_aligned_accepts_exact_column_only() {
        let mut state = ParseState::new("  x");
        state.bump();
        state.bump(); // now at column 3, pointing at 'x'
        assert!(state.check_aligned(3).is_ok());
        assert!(state.check_aligned(2).is_err());
        assert!(state.check_aligned(4).is_err());
    }

    #[test]
    fn check_indent_requires_strictly_greater_column() {
        let mut state = ParseState::new("   x");
        state.bump();
        state.bump();
        state.bump(); // column 4
        assert!(state.check_indent(3).is_ok());
        assert!(state.check_indent(4).is_err());
    }

    #[test]
    fn with_indent_scopes_and_restores() {
        let mut state = ParseState::new("x");
        assert_eq!(state.indent, None);
        state
            .with_indent(|s| {
                assert_eq!(s.indent, Some(1));
                Ok(())
            })
            .unwrap();
        assert_eq!(state.indent, None);
    }

    #[test]
    fn cursor_tracks_line_and_column_across_newlines() {
        let mut state = ParseState::new("ab\ncd");
        state.bump(); // 'a' -> col 2
        state.bump(); // 'b' -> col 3
        state.bump(); // '\n' -> line 2, col 1
        assert_eq!((state.pos.line, state.pos.col), (2, 1));
        state.bump(); // 'c' -> col 2
        assert_eq!(state.pos.col, 2);
    }
}
