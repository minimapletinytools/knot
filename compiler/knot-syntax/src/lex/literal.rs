//! Int/Float/String literal lexing. Which numeric type a literal produces is decided
//! lexically (by the presence of `.`/exponent), not deferred to the type checker —
//! matching Elm. No octal/binary literal forms, no digit-group underscores, no
//! multi-line strings, in v0.

use crate::error::{ErrorKind, ParseError};
use crate::span::{Cursor, Span, Spanned};
use crate::state::ParseState;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberLiteral {
    Int(i64),
    Float(f64),
}

impl<'a> ParseState<'a> {
    /// An `Int` or `Float` literal: `42`, `0xFF`, `3.14`, `1.5e10`. A `.` is only
    /// ever consumed as part of the fractional part if a digit immediately follows
    /// it — `1.` leaves the `.` unconsumed (Floats need a digit on both sides of the
    /// point), which is exactly why `1.` fails to parse as a complete literal: the
    /// leftover `.` has no valid grammar production to be consumed by afterward.
    pub fn number_literal(&mut self) -> Result<Spanned<NumberLiteral>, ParseError> {
        let start = self.pos;
        if !matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
            return Err(ParseError::new(
                ErrorKind::Expected("a number"),
                Span::new(start.offset, start.offset),
            ));
        }

        if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'x' | b'X')) {
            return self.hex_int_literal(start);
        }

        while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
            self.bump();
        }

        let mut is_float = false;
        if self.peek() == Some(b'.') && matches!(self.peek_at(1), Some(b) if b.is_ascii_digit()) {
            is_float = true;
            self.bump(); // '.'
            while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                self.bump();
            }
        }

        if matches!(self.peek(), Some(b'e' | b'E')) {
            let checkpoint = self.pos;
            self.bump();
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.bump();
            }
            let exp_digits_start = self.pos;
            while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                self.bump();
            }
            if self.pos.offset == exp_digits_start.offset {
                // 'e'/'E' wasn't actually followed by an exponent -- not part of
                // this literal, so back out and leave it for whatever comes next.
                self.pos = checkpoint;
            } else {
                is_float = true;
            }
        }

        let text = self.text_since(start.offset);
        let span = Span::new(start.offset, self.pos.offset);
        if is_float {
            let value: f64 = text.parse().map_err(|_| {
                ParseError::new(
                    ErrorKind::Custom(format!("`{text}` is not a valid Float")),
                    span,
                )
            })?;
            Ok(Spanned::new(span, NumberLiteral::Float(value)))
        } else {
            let value: i64 = text.parse().map_err(|_| {
                ParseError::new(
                    ErrorKind::Custom(format!("`{text}` is not a valid Int")),
                    span,
                )
            })?;
            Ok(Spanned::new(span, NumberLiteral::Int(value)))
        }
    }

    fn hex_int_literal(&mut self, start: Cursor) -> Result<Spanned<NumberLiteral>, ParseError> {
        self.bump(); // '0'
        self.bump(); // 'x' / 'X'
        let digits_start = self.pos;
        while matches!(self.peek(), Some(b) if b.is_ascii_hexdigit()) {
            self.bump();
        }
        if self.pos.offset == digits_start.offset {
            return Err(ParseError::new(
                ErrorKind::Expected("hex digits after `0x`"),
                Span::new(self.pos.offset, self.pos.offset),
            ));
        }
        let span = Span::new(start.offset, self.pos.offset);
        let digits = self.text_since(digits_start.offset);
        let value = i64::from_str_radix(digits, 16).map_err(|_| {
            ParseError::new(
                ErrorKind::Custom(format!("`0x{digits}` is not a valid hex Int")),
                span,
            )
        })?;
        Ok(Spanned::new(span, NumberLiteral::Int(value)))
    }

    /// A double-quoted string literal with escapes `\\`, `\"`, `\n`, `\t`, `\r`, and
    /// `\u{XXXX}`. No multi-line strings in v0 — an unescaped newline before the
    /// closing quote is an error, same as running off the end of the file.
    pub fn string_literal(&mut self) -> Result<Spanned<String>, ParseError> {
        let start = self.pos;
        if self.peek() != Some(b'"') {
            return Err(ParseError::new(
                ErrorKind::Expected("a string literal"),
                Span::new(start.offset, start.offset),
            ));
        }
        self.bump();
        let mut value = String::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => {
                    return Err(ParseError::new(
                        ErrorKind::Custom("unterminated string literal".to_string()),
                        Span::new(start.offset, self.pos.offset),
                    ));
                }
                Some(b'"') => {
                    self.bump();
                    break;
                }
                Some(b'\\') => {
                    self.bump();
                    self.string_escape(start, &mut value)?;
                }
                Some(first_byte) => {
                    let byte_start = self.pos.offset;
                    self.bump();
                    for _ in 1..utf8_char_len(first_byte) {
                        self.bump();
                    }
                    value.push_str(self.text_since(byte_start));
                }
            }
        }
        Ok(Spanned::new(
            Span::new(start.offset, self.pos.offset),
            value,
        ))
    }

    fn string_escape(&mut self, lit_start: Cursor, value: &mut String) -> Result<(), ParseError> {
        match self.peek() {
            Some(b'n') => {
                value.push('\n');
                self.bump();
            }
            Some(b't') => {
                value.push('\t');
                self.bump();
            }
            Some(b'r') => {
                value.push('\r');
                self.bump();
            }
            Some(b'\\') => {
                value.push('\\');
                self.bump();
            }
            Some(b'"') => {
                value.push('"');
                self.bump();
            }
            Some(b'u') => {
                self.bump();
                value.push(self.unicode_escape(lit_start)?);
            }
            _ => {
                return Err(ParseError::new(
                    ErrorKind::Custom("unrecognized escape sequence".to_string()),
                    Span::new(self.pos.offset, self.pos.offset),
                ));
            }
        }
        Ok(())
    }

    fn unicode_escape(&mut self, lit_start: Cursor) -> Result<char, ParseError> {
        if self.peek() != Some(b'{') {
            return Err(ParseError::new(
                ErrorKind::Custom("expected `{` after `\\u`".to_string()),
                Span::new(self.pos.offset, self.pos.offset),
            ));
        }
        self.bump();
        let digits_start = self.pos;
        while matches!(self.peek(), Some(b) if b.is_ascii_hexdigit()) {
            self.bump();
        }
        let digits = self.text_since(digits_start.offset).to_string();
        if self.peek() != Some(b'}') {
            return Err(ParseError::new(
                ErrorKind::Custom("expected `}` to close `\\u{...}`".to_string()),
                Span::new(self.pos.offset, self.pos.offset),
            ));
        }
        self.bump();
        u32::from_str_radix(&digits, 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| {
                ParseError::new(
                    ErrorKind::Custom(format!(
                        "`\\u{{{digits}}}` is not a valid Unicode code point"
                    )),
                    Span::new(lit_start.offset, self.pos.offset),
                )
            })
    }
}

fn utf8_char_len(first_byte: u8) -> usize {
    if first_byte & 0x80 == 0 {
        1
    } else if first_byte & 0xE0 == 0xC0 {
        2
    } else if first_byte & 0xF0 == 0xE0 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_int() {
        let mut s = ParseState::new("42 rest");
        let lit = s.number_literal().unwrap();
        assert_eq!(lit.node, NumberLiteral::Int(42));
        assert_eq!(s.peek(), Some(b' '));
    }

    #[test]
    fn hex_int() {
        let mut s = ParseState::new("0xFF");
        let lit = s.number_literal().unwrap();
        assert_eq!(lit.node, NumberLiteral::Int(255));
        assert!(s.is_eof());
    }

    #[test]
    fn hex_int_requires_digits() {
        let mut s = ParseState::new("0x");
        assert!(s.number_literal().is_err());
    }

    #[test]
    fn plain_float() {
        // 2.5, not e.g. 3.14: exactly representable in binary, so the equality
        // check isn't at the mercy of decimal-to-binary rounding (and doesn't
        // read as a suspiciously-approximate math constant to clippy).
        let mut s = ParseState::new("2.5");
        let lit = s.number_literal().unwrap();
        assert_eq!(lit.node, NumberLiteral::Float(2.5));
    }

    #[test]
    fn float_with_exponent() {
        let mut s = ParseState::new("1.5e10");
        let lit = s.number_literal().unwrap();
        assert_eq!(lit.node, NumberLiteral::Float(1.5e10));
    }

    #[test]
    fn int_with_exponent_is_still_a_float() {
        let mut s = ParseState::new("2e3");
        let lit = s.number_literal().unwrap();
        assert_eq!(lit.node, NumberLiteral::Float(2e3));
    }

    #[test]
    fn trailing_dot_is_not_consumed_into_the_literal() {
        // "1." -- Floats require a digit after the point, so this scans as the
        // Int `1` and leaves the `.` for the surrounding grammar to (fail to) deal
        // with, which is exactly why `1.` is an invalid literal end-to-end.
        let mut s = ParseState::new("1.");
        let lit = s.number_literal().unwrap();
        assert_eq!(lit.node, NumberLiteral::Int(1));
        assert_eq!(s.peek(), Some(b'.'));
    }

    #[test]
    fn leading_dot_is_not_a_number_at_all() {
        let mut s = ParseState::new(".5");
        assert!(s.number_literal().is_err());
    }

    #[test]
    fn dangling_e_is_not_treated_as_an_exponent() {
        // "1e" with nothing after -- backs out, so this is just the Int `1`.
        let mut s = ParseState::new("1ex");
        let lit = s.number_literal().unwrap();
        assert_eq!(lit.node, NumberLiteral::Int(1));
        assert_eq!(s.peek(), Some(b'e'));
    }

    #[test]
    fn plain_string() {
        let mut s = ParseState::new(r#""hello" rest"#);
        let lit = s.string_literal().unwrap();
        assert_eq!(lit.node, "hello");
    }

    #[test]
    fn string_escapes() {
        let mut s = ParseState::new(r#""a\nb\tc\\d\"e""#);
        let lit = s.string_literal().unwrap();
        assert_eq!(lit.node, "a\nb\tc\\d\"e");
    }

    #[test]
    fn string_unicode_escape() {
        let mut s = ParseState::new(r#""\u{1F600}""#);
        let lit = s.string_literal().unwrap();
        assert_eq!(lit.node, "\u{1F600}");
    }

    #[test]
    fn string_with_literal_utf8_content() {
        let mut s = ParseState::new("\"caf\u{e9}\"");
        let lit = s.string_literal().unwrap();
        assert_eq!(lit.node, "caf\u{e9}");
    }

    #[test]
    fn unterminated_string_errors() {
        let mut s = ParseState::new(r#""never closed"#);
        assert!(s.string_literal().is_err());
    }

    #[test]
    fn newline_in_string_errors() {
        let mut s = ParseState::new("\"a\nb\"");
        assert!(s.string_literal().is_err());
    }
}
