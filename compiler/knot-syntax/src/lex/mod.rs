//! Lexical layer: lower/upper/qualified identifiers, reserved words, holes, and
//! Int/Float/String literals. All exposed as methods on `ParseState` rather than a
//! separate token stream — matching the "no separate lexer, no separate layout
//! pass" architecture (see `knot-ast-parser-plan.md` §2).

pub mod ident;
pub mod literal;

pub use ident::RESERVED_WORDS;
pub use literal::NumberLiteral;
