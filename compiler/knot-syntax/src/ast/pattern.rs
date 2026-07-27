use crate::ast::Name;
use crate::span::Spanned;

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `_` (anonymous) or `_name` (named) — both valid here, unlike in expression
    /// position. A named discard is purely a reader-facing mnemonic; the compiler
    /// treats it identically to `_`.
    Wildcard(Option<String>),
    Var(String),
    Literal(PatternLiteral),
    /// Also covers `True`/`False`, which are just 0-arity constructors.
    Ctor(Name, Vec<Spanned<Pattern>>),
    Tuple(Vec<Spanned<Pattern>>),
    /// `x : xs`
    Cons(Box<Spanned<Pattern>>, Box<Spanned<Pattern>>),
    Nil,
    As(Box<Spanned<Pattern>>, String),
}

/// `Int` and `String` literal patterns are both fine; no `Float` variant — Float
/// literal patterns are forbidden (unsound equality, matches Elm).
#[derive(Debug, Clone, PartialEq)]
pub enum PatternLiteral {
    Int(i64),
    Str(String),
}
