//! Post-parse structural checks that don't need type information: tuple arity ≤ 3,
//! and duplicate top-level bindings (which is how "no multi-clause functions" gets
//! enforced — it's inherently a whole-module check, not a single-production grammar
//! limit). Mirrors what Elm actually does for tuple arity. (M7 — not yet implemented.)
