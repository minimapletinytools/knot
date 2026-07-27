//! Expression grammar, in sub-stages: atoms -> application -> unary negation ->
//! precedence-climbing binary ops (spec §4.8 table) -> layout-heavy forms (`if`/`let`/
//! `case`/lambda/`do`). (M4 — not yet implemented.)
