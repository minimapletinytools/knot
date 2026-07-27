//! Lexical layer: lower/upper/qualified identifiers, reserved words (including
//! `instance` — `where` graduates from meta-syntax-only to genuine user-facing syntax
//! alongside it), and Int/Float/String literals. (M1 — not yet implemented.)

pub mod ident;
pub mod literal;
