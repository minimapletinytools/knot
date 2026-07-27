//! A single flat `ParseError` (kind + span + context stack of enclosing rule names)
//! rather than a fully bespoke per-production error ADT like Elm's. Gets most of the
//! message-quality benefit with far less code — see `knot-ast-parser-plan.md` §2.

use crate::line_index::LineIndex;
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
    IndentViolation {
        expected_col: u32,
        found_col: u32,
    },
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ErrorKind,
    pub span: Span,
    /// Enclosing rule names, innermost first, e.g. `["if-expression", "function body"]` —
    /// each call to `with_context` appends the *next* rule out as the error
    /// unwinds back through the call stack, so the closest-to-the-error context
    /// ends up at index 0.
    pub context: Vec<&'static str>,
    /// Marks a genuine syntax error that must always propagate, as opposed to the
    /// ordinary "this alternative didn't match" failures that drive the crate's
    /// many "try X, fall back to Y on error" spots (application arguments,
    /// optional trailing forms, etc.). Every such fallback loop checks this flag
    /// before deciding to swallow an error and try something else — e.g. `f a _b c`
    /// (a named hole isn't a valid expression placeholder) or `f- 1` (ambiguously
    /// spaced `-`) must hard-fail rather than be silently reinterpreted as "`a` was
    /// the last argument after all."
    pub fatal: bool,
}

impl ParseError {
    pub fn new(kind: ErrorKind, span: Span) -> Self {
        ParseError {
            kind,
            span,
            context: Vec::new(),
            fatal: false,
        }
    }

    pub fn with_context(mut self, rule: &'static str) -> Self {
        self.context.push(rule);
        self
    }

    pub fn fatal(mut self) -> Self {
        self.fatal = true;
        self
    }

    /// Renders this error as a single-line, human-readable message with a
    /// 1-based line:column instead of a raw byte offset — e.g. for CLI or
    /// editor display. Takes the original source since `Span` only stores
    /// byte offsets (kept cheap and source-independent throughout parsing);
    /// the line/col conversion only happens here, when something actually
    /// needs to show a position to a human. Building a fresh `LineIndex` per
    /// call is fine for occasional error display; a caller rendering many
    /// errors against the same source should build one `LineIndex` once and
    /// do the offset-to-line/col lookup itself instead.
    ///
    /// `context` is pushed innermost-first (the closest enclosing rule calls
    /// `with_context` before the error propagates further up), so printing it
    /// in that same order reads as "specific problem, zooming out": e.g.
    /// "expected `then` (while parsing if-expression) (while parsing function
    /// body)", not the reverse.
    pub fn render(&self, source: &str) -> String {
        let (line, col) = LineIndex::new(source).line_col(self.span.start);
        let mut message = format!("{line}:{col}: {}", self.kind_message());
        for rule in &self.context {
            message.push_str(&format!(" (while parsing {rule})"));
        }
        message
    }

    fn kind_message(&self) -> String {
        match &self.kind {
            ErrorKind::Expected(what) => format!("expected {what}"),
            ErrorKind::UnexpectedEof => "unexpected end of file".to_string(),
            ErrorKind::UnclosedBlockComment => "unclosed block comment".to_string(),
            ErrorKind::IndentViolation {
                expected_col,
                found_col,
            } => {
                format!(
                    "indentation error: expected column {expected_col}, found column {found_col}"
                )
            }
            ErrorKind::Custom(message) => message.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_reports_line_and_column() {
        let source = "x = 1\ny = .5";
        let err = ParseError::new(ErrorKind::Expected("a number"), Span::new(6, 6));
        assert_eq!(err.render(source), "2:1: expected a number");
    }

    #[test]
    fn render_includes_context_stack_innermost_first() {
        // Simulates the real push order: the closest enclosing rule
        // (if-expression) calls with_context first, as the error propagates
        // up past it; the further-out rule (function body) adds its own
        // context after that, one level further up the call stack.
        let err = ParseError::new(ErrorKind::Expected("`then`"), Span::new(0, 0))
            .with_context("if-expression")
            .with_context("function body");
        assert_eq!(
            err.render("x"),
            "1:1: expected `then` (while parsing if-expression) (while parsing function body)"
        );
    }
}
