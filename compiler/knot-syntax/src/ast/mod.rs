//! The Knot AST: nothing resolved (no name resolution, no operator-fixity lookup —
//! Knot's operator set is closed so precedence is climbed directly during parsing),
//! spans attached throughout. See `knot-ast-parser-plan.md` §4 for the full design
//! and the reasoning behind each shape.

pub mod decl;
pub mod expr;
pub mod pattern;
pub mod ty;

/// A possibly-qualified identifier, e.g. `map` or `List.map`, stored as its full
/// dotted spelling. Simplicity over performance: no separate qualifier type for now.
pub type Name = String;
