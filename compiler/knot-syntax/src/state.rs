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
    /// True while parsing the value expression of an annotation (`@name(...)` or
    /// `@{...}`), at any depth — set only around those two call sites in
    /// `annotation.rs`, never inside the shared `expr_record_field`/
    /// `expr_record_field_list` helpers those reuse (which must stay unrestricted
    /// for plain record construction). `expr_atom` consults this to reject a
    /// nested `@` anywhere inside an annotation's own value, since an unravel
    /// function (or any other annotation value) has no node-graph representation
    /// of its own for a nested annotation to attach to.
    pub in_annotation_value: bool,
}

impl<'a> ParseState<'a> {
    pub fn new(src: &'a str) -> Self {
        ParseState {
            src: src.as_bytes(),
            pos: Cursor::start(),
            indent: None,
            in_annotation_value: false,
        }
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

    /// Skips trivia, then requires and consumes a single-byte punctuation token
    /// (`)`, `}`, `:`, ...). A shared primitive since every grammar layer needs
    /// "expect this exact punctuation or error" constantly.
    pub fn expect_byte(&mut self, byte: u8) -> Result<(), ParseError> {
        self.skip_trivia()?;
        if self.peek() != Some(byte) {
            return Err(ParseError::new(
                ErrorKind::Custom(format!("expected `{}`", byte as char)),
                Span::new(self.pos.offset, self.pos.offset),
            ));
        }
        self.bump();
        Ok(())
    }

    /// Skips trivia, then requires and consumes `->` as one atomic two-byte token
    /// — deliberately *not* two separate `expect_byte` calls, since those would
    /// each skip trivia independently and wrongly accept a stray space between
    /// `-` and `>` as if it were still a single arrow.
    pub fn expect_arrow(&mut self) -> Result<(), ParseError> {
        self.skip_trivia()?;
        if !(self.peek() == Some(b'-') && self.peek_at(1) == Some(b'>')) {
            return Err(ParseError::new(
                ErrorKind::Expected("`->`"),
                Span::new(self.pos.offset, self.pos.offset),
            ));
        }
        self.bump();
        self.bump();
        Ok(())
    }

    /// Skips trivia, then requires and consumes `..` atomically (same rationale
    /// as `expect_arrow`) — used by `exposing (..)` and `Type(..)`.
    pub fn expect_dotdot(&mut self) -> Result<(), ParseError> {
        self.skip_trivia()?;
        if !(self.peek() == Some(b'.') && self.peek_at(1) == Some(b'.')) {
            return Err(ParseError::new(
                ErrorKind::Expected("`..`"),
                Span::new(self.pos.offset, self.pos.offset),
            ));
        }
        self.bump();
        self.bump();
        Ok(())
    }

    /// True unless a reference indent is active and the cursor hasn't reached
    /// strictly past it — i.e. "is the current position still a valid
    /// continuation of whatever's being parsed inside the current layout block."
    /// Checked before trying to consume one more application argument, one more
    /// binop, or one more declaration parameter, so a sibling block item (which
    /// sits *at* the reference column, not past it) is never misread as part of
    /// the previous item.
    pub fn continues_layout(&self) -> bool {
        match self.indent {
            None => true,
            Some(col) => self.pos.col > col,
        }
    }

    /// The source text between byte offset `start` and the current position.
    /// Returns a slice borrowed from the *original* source (lifetime `'a`),
    /// independent of this method's own short-lived `&self` borrow, so callers can
    /// keep using it after continuing to mutate `self`. Only valid to call when both
    /// ends fall on UTF-8 char boundaries, which every lexer in this crate
    /// guarantees by construction (each one only ever stops mid-scan on a full
    /// UTF-8 char boundary, whether that's an ASCII byte or the end of a
    /// multi-byte sequence it copied in one step).
    pub fn text_since(&self, start: u32) -> &'a str {
        let src: &'a [u8] = self.src;
        std::str::from_utf8(&src[start as usize..self.pos.offset as usize])
            .expect("lexer only stops on UTF-8 char boundaries, so this range is always valid")
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
                ErrorKind::IndentViolation {
                    expected_col: col + 1,
                    found_col: self.pos.col,
                },
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
                ErrorKind::IndentViolation {
                    expected_col: col,
                    found_col: self.pos.col,
                },
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

    /// Run `f` with `in_annotation_value` set, restoring the previous value
    /// afterward. Used only around the two spots in `annotation.rs` that parse
    /// an annotation's own value expression, so `expr_atom` can reject a nested
    /// `@` anywhere within it (spec: annotation values, including pullback/
    /// unravel functions, cannot themselves carry annotations).
    pub fn with_no_nested_annotations<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        let saved = self.in_annotation_value;
        self.in_annotation_value = true;
        let result = f(self);
        self.in_annotation_value = saved;
        result
    }

    /// Attempts `f`; if it fails, restores the position from before the attempt
    /// (leaving the error itself untouched). This is the crate's one deliberate,
    /// narrow use of backtracking — for the handful of spots where a short prefix
    /// can't be told apart from its alternative without just trying it (e.g. is
    /// `(Integral a, Num b)` the start of a constraint list, or an ordinary tuple
    /// type?). Not a general "try anything, backtrack on any error" combinator —
    /// callers should keep its scope as narrow as the ambiguity requires.
    pub fn try_parse<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        let checkpoint = self.clone();
        match f(self) {
            Ok(value) => Ok(value),
            Err(err) => {
                *self = checkpoint;
                Err(err)
            }
        }
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
