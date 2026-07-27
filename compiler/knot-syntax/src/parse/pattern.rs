//! Pattern grammar: wildcards/named holes, `Int`/`String` literals (no `Float` —
//! forbidden, unsound equality, matches Elm), constructors (including `True`/
//! `False` as 0-arity ctors), tuples, `()`, `[]`/`:` for lists, and `as` aliases.
//! No record-shorthand destructuring and no pattern guards — both are enforced
//! simply by not existing as a grammar production here (or, for guards, in the
//! `case`-arm grammar in M4).

use crate::ast::pattern::{Pattern, PatternLiteral};
use crate::error::{ErrorKind, ParseError};
use crate::lex::literal::NumberLiteral;
use crate::span::{Cursor, Span, Spanned};
use crate::state::ParseState;

impl<'a> ParseState<'a> {
    /// A full pattern: `as`-aliasing and cons chains resolve here; anything
    /// tighter (constructor application, tuples, literals, ...) is `pattern_as`
    /// and below.
    pub fn pattern(&mut self) -> Result<Spanned<Pattern>, ParseError> {
        self.pattern_cons()
    }

    /// `x : xs`, right-associative — mirrors `:`'s associativity in the
    /// expression grammar (spec §4.8). Rejects `::` here the same way the
    /// operator table does (that's the type-signature token, never cons).
    fn pattern_cons(&mut self) -> Result<Spanned<Pattern>, ParseError> {
        let head = self.pattern_as()?;
        self.skip_trivia()?;
        if self.peek() == Some(b':') && self.peek_at(1) != Some(b':') {
            self.bump();
            let rest = self.pattern_cons()?;
            let span = Span::new(head.span.start, rest.span.end);
            Ok(Spanned::new(
                span,
                Pattern::Cons(Box::new(head), Box::new(rest)),
            ))
        } else {
            Ok(head)
        }
    }

    /// A constructor application (or anything tighter) with an optional trailing
    /// `as name`. Aliasing anything wider than one atom/application needs parens
    /// — `(x : xs) as full`, never `x : xs as full` (which would alias just
    /// `xs`), since `as` binds at this level, one below cons.
    fn pattern_as(&mut self) -> Result<Spanned<Pattern>, ParseError> {
        let pat = self.pattern_app()?;
        self.skip_trivia()?;
        if self.peek_keyword("as") {
            self.expect_keyword("as")?;
            self.skip_trivia()?;
            let name = self.lower_ident()?.node;
            let span = Span::new(pat.span.start, self.pos.offset);
            Ok(Spanned::new(span, Pattern::As(Box::new(pat), name)))
        } else {
            Ok(pat)
        }
    }

    /// A bare constructor greedily applied to trailing atoms (`Circle r`,
    /// `Rectangle w h`), or any other single atom on its own. Mirrors
    /// `type_app`'s design exactly: only a bare `Ctor` head takes further
    /// arguments, and application stops — without consuming — the moment the
    /// next token doesn't start a pattern atom (e.g. `Just x : xs` correctly
    /// stops `Just`'s application at `x`, since `:` isn't a valid atom start).
    fn pattern_app(&mut self) -> Result<Spanned<Pattern>, ParseError> {
        let head = self.pattern_atom()?;
        let Pattern::Ctor(name, _) = &head.node else {
            return Ok(head);
        };
        let name = name.clone();
        let mut args = Vec::new();
        loop {
            let checkpoint = self.clone();
            match self.pattern_atom() {
                Ok(arg) => args.push(arg),
                Err(err) if err.fatal => return Err(err),
                Err(_) => {
                    *self = checkpoint;
                    break;
                }
            }
        }
        if args.is_empty() {
            Ok(head)
        } else {
            let span = Span::new(
                head.span.start,
                args.last().expect("just checked non-empty").span.end,
            );
            Ok(Spanned::new(span, Pattern::Ctor(name, args)))
        }
    }

    /// `pub` because M4's lambda/function-parameter parsing reuses this directly:
    /// a lambda parameter must be a single atom (`\x -> ...`, `\(Just x) -> ...`)
    /// so that multiple params separated by spaces aren't ambiguous with a
    /// constructor pattern's own arguments.
    pub fn pattern_atom(&mut self) -> Result<Spanned<Pattern>, ParseError> {
        self.skip_trivia()?;
        let start = self.pos;
        match self.peek() {
            Some(b'_') => {
                let hole = self.hole()?;
                Ok(Spanned::new(hole.span, Pattern::Wildcard(hole.node)))
            }
            Some(b'(') => self.pattern_paren_tuple_or_unit(),
            Some(b'[') => self.pattern_nil(),
            Some(b'"') => {
                let lit = self.string_literal()?;
                Ok(Spanned::new(
                    lit.span,
                    Pattern::Literal(PatternLiteral::Str(lit.node)),
                ))
            }
            // A leading `-` immediately before a digit, with nothing preceding it
            // in this atom, is a negative Int literal pattern (not covered by the
            // spec's §4.9 negation rule, which is expression-only — but the same
            // "no space between sign and digits" spelling is the natural fit here
            // too, and negative-literal patterns are too standard to leave out).
            Some(b'-') if matches!(self.peek_at(1), Some(b) if b.is_ascii_digit()) => {
                self.bump();
                self.pattern_number_literal(start, true)
            }
            Some(b) if b.is_ascii_digit() => self.pattern_number_literal(start, false),
            Some(b) if b.is_ascii_uppercase() => {
                let name = self.qualified_upper_start()?;
                Ok(Spanned::new(
                    name.span,
                    Pattern::Ctor(name.node, Vec::new()),
                ))
            }
            Some(b) if b.is_ascii_lowercase() => {
                let name = self.lower_ident()?;
                Ok(Spanned::new(name.span, Pattern::Var(name.node)))
            }
            _ => Err(ParseError::new(
                ErrorKind::Expected("a pattern"),
                Span::new(self.pos.offset, self.pos.offset),
            )),
        }
    }

    fn pattern_number_literal(
        &mut self,
        start: Cursor,
        negative: bool,
    ) -> Result<Spanned<Pattern>, ParseError> {
        let lit = self.number_literal()?;
        let span = Span::new(start.offset, self.pos.offset);
        match lit.node {
            NumberLiteral::Int(value) => {
                let value = if negative { -value } else { value };
                Ok(Spanned::new(
                    span,
                    Pattern::Literal(PatternLiteral::Int(value)),
                ))
            }
            NumberLiteral::Float(_) => Err(ParseError::new(
                ErrorKind::Custom(
                    "Float literal patterns are forbidden (unsound equality, matches Elm)"
                        .to_string(),
                ),
                span,
            )),
        }
    }

    /// `()` (Unit), `(pat)` (transparent grouping), or `(pat, pat, ...)` (tuple —
    /// arity isn't checked here, see `validate.rs`).
    fn pattern_paren_tuple_or_unit(&mut self) -> Result<Spanned<Pattern>, ParseError> {
        let start = self.pos;
        self.bump(); // '('
        self.skip_trivia()?;
        if self.peek() == Some(b')') {
            self.bump();
            return Ok(Spanned::new(
                Span::new(start.offset, self.pos.offset),
                Pattern::Unit,
            ));
        }
        let first = self.pattern()?;
        self.skip_trivia()?;
        if self.peek() == Some(b',') {
            let mut elements = vec![first];
            while self.peek() == Some(b',') {
                self.bump();
                self.skip_trivia()?;
                elements.push(self.pattern()?);
                self.skip_trivia()?;
            }
            self.expect_byte(b')')?;
            Ok(Spanned::new(
                Span::new(start.offset, self.pos.offset),
                Pattern::Tuple(elements),
            ))
        } else {
            self.expect_byte(b')')?;
            Ok(first)
        }
    }

    /// `[]` — the only bracketed list-pattern form; anything with elements is
    /// written as a `:` chain instead (`x : y : []`), matching what's actually in
    /// the spec and corpus rather than adding unspecified `[a, b, c]` sugar.
    fn pattern_nil(&mut self) -> Result<Spanned<Pattern>, ParseError> {
        let start = self.pos;
        self.bump(); // '['
        self.expect_byte(b']')?;
        Ok(Spanned::new(
            Span::new(start.offset, self.pos.offset),
            Pattern::Nil,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(src: &str) -> Pattern {
        let mut s = ParseState::new(src);
        let result = s.pattern().unwrap();
        assert!(
            s.is_eof(),
            "leftover input: {:?}",
            s.text_since(s.pos.offset)
        );
        result.node
    }

    #[test]
    fn wildcard() {
        assert_eq!(pat("_"), Pattern::Wildcard(None));
    }

    #[test]
    fn var_pattern() {
        assert_eq!(pat("x"), Pattern::Var("x".to_string()));
    }

    #[test]
    fn unit_pattern() {
        assert_eq!(pat("()"), Pattern::Unit);
    }

    #[test]
    fn nil_pattern() {
        assert_eq!(pat("[]"), Pattern::Nil);
    }

    #[test]
    fn int_literal_pattern() {
        assert_eq!(pat("0"), Pattern::Literal(PatternLiteral::Int(0)));
        assert_eq!(pat("42"), Pattern::Literal(PatternLiteral::Int(42)));
    }

    #[test]
    fn negative_int_literal_pattern() {
        assert_eq!(pat("-1"), Pattern::Literal(PatternLiteral::Int(-1)));
    }

    #[test]
    fn string_literal_pattern() {
        assert_eq!(
            pat("\"admin\""),
            Pattern::Literal(PatternLiteral::Str("admin".to_string()))
        );
    }

    #[test]
    fn float_literal_pattern_is_rejected() {
        let mut s = ParseState::new("3.14");
        assert!(s.pattern().is_err());
    }

    #[test]
    fn bool_ctor_patterns() {
        assert_eq!(pat("True"), Pattern::Ctor("True".to_string(), Vec::new()));
        assert_eq!(pat("False"), Pattern::Ctor("False".to_string(), Vec::new()));
    }

    #[test]
    fn ctor_with_one_argument() {
        assert_eq!(
            pat("Circle r"),
            Pattern::Ctor(
                "Circle".to_string(),
                vec![Spanned::new(Span::new(7, 8), Pattern::Var("r".to_string()))]
            )
        );
    }

    #[test]
    fn ctor_with_multiple_arguments() {
        let Pattern::Ctor(name, args) = pat("Rectangle w h") else {
            panic!("expected Ctor")
        };
        assert_eq!(name, "Rectangle");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0].node, Pattern::Var("w".to_string()));
        assert_eq!(args[1].node, Pattern::Var("h".to_string()));
    }

    #[test]
    fn tuple_pattern() {
        assert_eq!(
            pat("(x, y)"),
            Pattern::Tuple(vec![
                Spanned::new(Span::new(1, 2), Pattern::Var("x".to_string())),
                Spanned::new(Span::new(4, 5), Pattern::Var("y".to_string())),
            ])
        );
    }

    #[test]
    fn parens_are_transparent_grouping() {
        assert_eq!(pat("(x)"), Pattern::Var("x".to_string()));
    }

    #[test]
    fn cons_pattern() {
        let Pattern::Cons(head, tail) = pat("x : rest") else {
            panic!("expected Cons")
        };
        assert_eq!(head.node, Pattern::Var("x".to_string()));
        assert_eq!(tail.node, Pattern::Var("rest".to_string()));
    }

    #[test]
    fn cons_is_right_associative() {
        let Pattern::Cons(a, rest) = pat("a : b : []") else {
            panic!("expected Cons")
        };
        assert_eq!(a.node, Pattern::Var("a".to_string()));
        let Pattern::Cons(b, nil) = rest.node else {
            panic!("expected nested Cons")
        };
        assert_eq!(b.node, Pattern::Var("b".to_string()));
        assert_eq!(nil.node, Pattern::Nil);
    }

    #[test]
    fn double_colon_is_not_cons() {
        let mut s = ParseState::new("x :: xs");
        // `x` parses fine as a pattern on its own; `::` is left unconsumed since
        // it isn't cons, proving pattern_cons didn't misparse it.
        let result = s.pattern().unwrap();
        assert_eq!(result.node, Pattern::Var("x".to_string()));
        assert!(!s.is_eof());
    }

    #[test]
    fn ctor_application_stops_before_cons() {
        // "Just x : xs" -- `Just`'s application must stop at `x`, not try to
        // swallow `xs` too, since `:` doesn't start a pattern atom.
        let Pattern::Cons(head, tail) = pat("Just x : xs") else {
            panic!("expected top-level Cons")
        };
        assert_eq!(
            head.node,
            Pattern::Ctor(
                "Just".to_string(),
                vec![Spanned::new(Span::new(5, 6), Pattern::Var("x".to_string()))]
            )
        );
        assert_eq!(tail.node, Pattern::Var("xs".to_string()));
    }

    #[test]
    fn as_alias_on_parenthesized_cons() {
        let Pattern::As(inner, name) = pat("(x : rest) as full") else {
            panic!("expected As")
        };
        assert_eq!(name, "full");
        assert!(matches!(inner.node, Pattern::Cons(_, _)));
    }

    #[test]
    fn as_alias_without_parens_binds_to_the_tighter_pattern_only() {
        // "x : xs as full" -- `as` binds at the level below cons, so this aliases
        // just `xs`, not the whole cons pattern (matching the spec's rationale for
        // why the parenthesized form exists at all).
        let Pattern::Cons(head, tail) = pat("x : xs as full") else {
            panic!("expected top-level Cons")
        };
        assert_eq!(head.node, Pattern::Var("x".to_string()));
        let Pattern::As(aliased, name) = tail.node else {
            panic!("expected As on the tail")
        };
        assert_eq!(name, "full");
        assert_eq!(aliased.node, Pattern::Var("xs".to_string()));
    }

    #[test]
    fn record_shorthand_pattern_is_rejected() {
        let mut s = ParseState::new("{ x, y }");
        assert!(s.pattern().is_err());
    }
}
