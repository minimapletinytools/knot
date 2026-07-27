//! Type-expression grammar: arrows, application, tuples, records (incl.
//! extensible), and constrained signatures (`Ord a => ...`).
//!
//! Scope note: unlike the expression grammar, this one doesn't thread layout
//! (`check_indent`) through multi-line type expressions yet — every fixture in the
//! corpus is a single-line signature, and adding indent-checking here before
//! anything actually needs it would be complexity with no test behind it. Trivia
//! (including newlines) is skipped freely between tokens for now; tightening this
//! later is a small, local change if a real multi-line signature ever needs it.

use crate::ast::ty::{Constraint, Type, TypeSignature};
use crate::error::{ErrorKind, ParseError};
use crate::span::Span;
use crate::state::ParseState;

impl<'a> ParseState<'a> {
    /// A full `::` signature: an optional constraint list, then the type itself.
    /// The constraint list is tried first and discarded on failure (see
    /// `ParseState::try_parse`) — `(Integral a, Num b)` looks exactly like the
    /// start of an ordinary tuple type until we know whether `=>` follows it.
    pub fn type_signature(&mut self) -> Result<TypeSignature, ParseError> {
        let constraints = self
            .try_parse(Self::constraint_list_arrow)
            .unwrap_or_default();
        let ty = self.type_arrow()?;
        Ok(TypeSignature { constraints, ty })
    }

    fn constraint_list_arrow(&mut self) -> Result<Vec<Constraint>, ParseError> {
        self.skip_trivia()?;
        let constraints = if self.peek() == Some(b'(') {
            self.bump(); // '('
            self.skip_trivia()?;
            let mut constraints = vec![self.single_constraint()?];
            self.skip_trivia()?;
            while self.peek() == Some(b',') {
                self.bump();
                self.skip_trivia()?;
                constraints.push(self.single_constraint()?);
                self.skip_trivia()?;
            }
            self.expect_byte(b')')?;
            constraints
        } else {
            vec![self.single_constraint()?]
        };
        self.skip_trivia()?;
        if !(self.peek() == Some(b'=') && self.peek_at(1) == Some(b'>')) {
            return Err(ParseError::new(
                ErrorKind::Expected("`=>`"),
                Span::new(self.pos.offset, self.pos.offset),
            ));
        }
        self.bump(); // '='
        self.bump(); // '>'
        Ok(constraints)
    }

    fn single_constraint(&mut self) -> Result<Constraint, ParseError> {
        self.skip_trivia()?;
        let interface = self.upper_ident_segment()?.node;
        self.skip_trivia()?;
        let type_var = self.lower_ident()?.node;
        Ok(Constraint {
            interface,
            type_var,
        })
    }

    /// `T -> T -> T`, right-associative.
    pub fn type_arrow(&mut self) -> Result<Type, ParseError> {
        let left = self.type_app()?;
        self.skip_trivia()?;
        if self.peek() == Some(b'-') && self.peek_at(1) == Some(b'>') {
            self.expect_arrow()?;
            let right = self.type_arrow()?;
            Ok(Type::Fn(Box::new(left), Box::new(right)))
        } else {
            Ok(left)
        }
    }

    /// One or more atoms in a row: `Int`, `List a`, `Map k v`. Only a bare `Named`
    /// head can take arguments this way — a type variable, tuple, record, or Unit
    /// is always an atom on its own (`a b` is not valid Knot), so nothing is even
    /// attempted, and no already-consumed input is ever silently dropped.
    fn type_app(&mut self) -> Result<Type, ParseError> {
        let head = self.type_atom()?;
        let Type::Named(name, _) = head else {
            return Ok(head);
        };
        let mut args = Vec::new();
        loop {
            let checkpoint = self.clone();
            match self.type_atom() {
                Ok(arg) => args.push(arg),
                Err(err) if err.fatal => return Err(err),
                Err(_) => {
                    *self = checkpoint;
                    break;
                }
            }
        }
        Ok(Type::Named(name, args))
    }

    fn type_atom(&mut self) -> Result<Type, ParseError> {
        self.skip_trivia()?;
        match self.peek() {
            Some(b'(') => self.type_paren_or_tuple(),
            Some(b'{') => self.type_record(),
            Some(b) if b.is_ascii_uppercase() => {
                let name = self.qualified_upper_start()?.node;
                Ok(Type::Named(name, Vec::new()))
            }
            Some(b) if b.is_ascii_lowercase() => {
                let name = self.lower_ident()?.node;
                Ok(Type::Var(name))
            }
            _ => Err(ParseError::new(
                ErrorKind::Expected("a type"),
                Span::new(self.pos.offset, self.pos.offset),
            )),
        }
    }

    /// `()` (Unit), `(T)` (parens are pure grouping, transparent), or `(T, T, ...)`
    /// (a tuple — arity isn't checked here, see `validate.rs`).
    fn type_paren_or_tuple(&mut self) -> Result<Type, ParseError> {
        self.bump(); // '('
        self.skip_trivia()?;
        if self.peek() == Some(b')') {
            self.bump();
            return Ok(Type::Unit);
        }
        let first = self.type_arrow()?;
        self.skip_trivia()?;
        if self.peek() == Some(b',') {
            let mut elements = vec![first];
            while self.peek() == Some(b',') {
                self.bump();
                self.skip_trivia()?;
                elements.push(self.type_arrow()?);
                self.skip_trivia()?;
            }
            self.expect_byte(b')')?;
            Ok(Type::Tuple(elements))
        } else {
            self.expect_byte(b')')?;
            Ok(first)
        }
    }

    /// `{ field : Type, ... }`, or extensible `{ r | field : Type, ... }`. Both
    /// start with a lowercase identifier, so the extension form is tried first and
    /// discarded on failure rather than special-cased with extra lookahead.
    fn type_record(&mut self) -> Result<Type, ParseError> {
        self.bump(); // '{'
        self.skip_trivia()?;

        let extension = self
            .try_parse(|state| {
                let var = state.lower_ident()?.node;
                state.skip_trivia()?;
                if state.peek() != Some(b'|') {
                    return Err(ParseError::new(
                        ErrorKind::Expected("`|`"),
                        Span::new(state.pos.offset, state.pos.offset),
                    ));
                }
                state.bump();
                Ok(var)
            })
            .ok();

        self.skip_trivia()?;
        let mut fields = Vec::new();
        if self.peek() != Some(b'}') {
            fields.push(self.record_field()?);
            self.skip_trivia()?;
            while self.peek() == Some(b',') {
                self.bump();
                self.skip_trivia()?;
                fields.push(self.record_field()?);
                self.skip_trivia()?;
            }
        }
        self.expect_byte(b'}')?;
        Ok(Type::Record(fields, extension))
    }

    fn record_field(&mut self) -> Result<(String, Type), ParseError> {
        let name = self.lower_ident()?.node;
        self.skip_trivia()?;
        if self.peek() != Some(b':') {
            return Err(ParseError::new(
                ErrorKind::Expected("`:`"),
                Span::new(self.pos.offset, self.pos.offset),
            ));
        }
        if self.peek_at(1) == Some(b':') {
            return Err(ParseError::new(
                ErrorKind::Custom(
                    "expected a single `:` for a record field type, not `::`".to_string(),
                ),
                Span::new(self.pos.offset, self.pos.offset),
            ));
        }
        self.bump();
        self.skip_trivia()?;
        let ty = self.type_arrow()?;
        Ok((name, ty))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ty(src: &str) -> Type {
        let mut s = ParseState::new(src);
        let result = s.type_arrow().unwrap();
        assert!(
            s.is_eof(),
            "leftover input: {:?}",
            s.text_since(s.pos.offset)
        );
        result
    }

    fn named(name: &str) -> Type {
        Type::Named(name.to_string(), Vec::new())
    }

    #[test]
    fn plain_named_type() {
        assert_eq!(ty("Int"), named("Int"));
    }

    #[test]
    fn type_variable() {
        assert_eq!(ty("a"), Type::Var("a".to_string()));
    }

    #[test]
    fn unit_type() {
        assert_eq!(ty("()"), Type::Unit);
    }

    #[test]
    fn parens_are_transparent_grouping() {
        assert_eq!(ty("(Int)"), named("Int"));
    }

    #[test]
    fn tuple_pair() {
        assert_eq!(
            ty("(Int, String)"),
            Type::Tuple(vec![named("Int"), named("String")])
        );
    }

    #[test]
    fn tuple_triple() {
        assert_eq!(
            ty("(Float, Float, Float)"),
            Type::Tuple(vec![named("Float"), named("Float"), named("Float")])
        );
    }

    #[test]
    fn arrow_is_right_associative() {
        assert_eq!(
            ty("Int -> Int -> Bool"),
            Type::Fn(
                Box::new(named("Int")),
                Box::new(Type::Fn(Box::new(named("Int")), Box::new(named("Bool"))))
            )
        );
    }

    #[test]
    fn single_argument_application() {
        assert_eq!(
            ty("List a"),
            Type::Named("List".to_string(), vec![Type::Var("a".to_string())])
        );
    }

    #[test]
    fn multi_argument_application() {
        assert_eq!(
            ty("Map k v"),
            Type::Named(
                "Map".to_string(),
                vec![Type::Var("k".to_string()), Type::Var("v".to_string())]
            )
        );
    }

    #[test]
    fn parenthesized_argument() {
        assert_eq!(
            ty("Map (List Int) String"),
            Type::Named(
                "Map".to_string(),
                vec![
                    Type::Named("List".to_string(), vec![named("Int")]),
                    named("String")
                ]
            )
        );
    }

    #[test]
    fn qualified_type_name() {
        assert_eq!(
            ty("List.List Int"),
            Type::Named("List.List".to_string(), vec![named("Int")])
        );
    }

    #[test]
    fn type_variable_takes_no_arguments() {
        // "a b" is not valid Knot at the type level -- `a` alone is a complete
        // type_arrow, and the leftover ` b` triggers the `is_eof` assertion in the
        // `ty()` test helper, proving `type_app` didn't silently swallow it.
        let mut s = ParseState::new("a b");
        let result = s.type_arrow().unwrap();
        assert_eq!(result, Type::Var("a".to_string()));
        assert!(!s.is_eof());
    }

    #[test]
    fn plain_record() {
        assert_eq!(
            ty("{ x : Float, y : Float }"),
            Type::Record(
                vec![
                    ("x".to_string(), named("Float")),
                    ("y".to_string(), named("Float"))
                ],
                None
            )
        );
    }

    #[test]
    fn extensible_record() {
        assert_eq!(
            ty("{ r | x : Float, y : Float }"),
            Type::Record(
                vec![
                    ("x".to_string(), named("Float")),
                    ("y".to_string(), named("Float"))
                ],
                Some("r".to_string())
            )
        );
    }

    #[test]
    fn record_field_rejects_double_colon() {
        let mut s = ParseState::new("{ x :: Float }");
        assert!(s.type_arrow().is_err());
    }

    #[test]
    fn signature_without_constraints() {
        let mut s = ParseState::new("a -> a -> Bool");
        let sig = s.type_signature().unwrap();
        assert!(sig.constraints.is_empty());
        assert_eq!(
            sig.ty,
            Type::Fn(
                Box::new(Type::Var("a".to_string())),
                Box::new(Type::Fn(
                    Box::new(Type::Var("a".to_string())),
                    Box::new(named("Bool"))
                ))
            )
        );
    }

    #[test]
    fn signature_with_single_unparenthesized_constraint() {
        let mut s = ParseState::new("Ord a => a -> a -> a");
        let sig = s.type_signature().unwrap();
        assert_eq!(
            sig.constraints,
            vec![Constraint {
                interface: "Ord".to_string(),
                type_var: "a".to_string()
            }]
        );
    }

    #[test]
    fn signature_with_multiple_parenthesized_constraints() {
        let mut s = ParseState::new("(Integral a, Num b) => a -> b");
        let sig = s.type_signature().unwrap();
        assert_eq!(
            sig.constraints,
            vec![
                Constraint {
                    interface: "Integral".to_string(),
                    type_var: "a".to_string()
                },
                Constraint {
                    interface: "Num".to_string(),
                    type_var: "b".to_string()
                },
            ]
        );
        assert_eq!(
            sig.ty,
            Type::Fn(
                Box::new(Type::Var("a".to_string())),
                Box::new(Type::Var("b".to_string()))
            )
        );
    }

    #[test]
    fn plain_tuple_signature_is_not_mistaken_for_constraints() {
        // The critical backtracking-correctness case: `(Int, String)` looks
        // exactly like the start of a constraint list until the parser discovers
        // there's no `=>` after the closing paren -- it must fall back to parsing
        // the whole thing as an ordinary tuple type, not fail or misparse.
        let mut s = ParseState::new("(Int, String)");
        let sig = s.type_signature().unwrap();
        assert!(sig.constraints.is_empty());
        assert_eq!(sig.ty, Type::Tuple(vec![named("Int"), named("String")]));
    }
}
