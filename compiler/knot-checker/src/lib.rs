//! `knot-checker` — type-checks and dictionary-elaborates a `knot-canonical`
//! `CModule`: "type analysis + dictionary passing stuff" from `TODO.txt`, the
//! step after `knot-canonical`. See `knot-type-checker-plan.md` at the repo
//! root for the full design; this crate is being built incrementally,
//! milestone by milestone (plan §7), rather than in one pass.
//!
//! **Current state**: `var.rs` (the arena-indexed union-find substitution)
//! and `ty.rs` (the `Structure`/`Scheme` shapes) are TM0; `unify.rs` — full
//! structural unification over `App`/`Fn`/`Tuple`/`Unit`/`Record` (the last
//! via field-gathering), the occurs check, and rigid-variable handling — is
//! TM1+TM2. Constraint generation over `CExpr`/`CPattern`/`CDecl` (TM3/TM4),
//! `solve.rs` (TM5), the interface/instance and annotation tables (TM6), and
//! dictionary elaboration (TM7) don't exist yet — there is no public
//! `check_module` entry point until enough of those exist to make one
//! meaningful.

pub mod error;
pub mod ty;
pub mod unify;
pub mod var;
