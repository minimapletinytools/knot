//! `knot-syntax` — parses Knot source text into an AST: spans attached, nothing
//! resolved (no name resolution, no type checking), operator precedence already
//! climbed. See `knot-ast-parser-plan.md` at the repo root for the full design and
//! `language-spec-notes.md` for the language being parsed.

pub mod ast;
pub mod error;
pub mod lex;
pub mod parse;
pub mod span;
pub mod state;
pub mod validate;

pub use error::{ErrorKind, ParseError};
pub use span::{Cursor, Span, Spanned};
pub use state::ParseState;
