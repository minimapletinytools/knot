//! A single flat `ParseError` (kind + span + context stack of enclosing rule names)
//! rather than a fully bespoke per-production error ADT like Elm's. Gets most of the
//! message-quality benefit with far less code — see `knot-ast-parser-plan.md` §2.

use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// A human-readable description of what was expected at this position, e.g.
    /// "an expression" or "`then`".
    Expected(&'static str),
    UnexpectedEof,
    UnclosedBlockComment,
    /// Raised by `ParseState::check_indent` / `check_aligned` when a token appears at
    /// the wrong column for its layout context.
    IndentViolation { expected_col: u32, found_col: u32 },
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ErrorKind,
    pub span: Span,
    /// Enclosing rule names, outermost first, e.g. `["module", "declaration", "expression"]` —
    /// pushed on by `with_context` as the error unwinds back through the call stack.
    pub context: Vec<&'static str>,
}

impl ParseError {
    pub fn new(kind: ErrorKind, span: Span) -> Self {
        ParseError { kind, span, context: Vec::new() }
    }

    pub fn with_context(mut self, rule: &'static str) -> Self {
        self.context.push(rule);
        self
    }
}
