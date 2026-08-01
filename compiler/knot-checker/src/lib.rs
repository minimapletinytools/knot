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
//! TM1+TM2. `constrain::{expr, pattern}` (TM3) generate `Constraint`s over
//! everything but `Do` (needs the Context interface's `pure`/`bind`, spec
//! §6.4). `constrain::decl` (TM4) does SCC dependency splitting for
//! `let`/top-level binding groups, builds the resulting nested
//! `Constraint::Let` tree, and wires real rigid variables up for signed
//! bindings. `solve.rs` (TM5) walks a `Constraint` tree, actually calling
//! `unify`, generalizing top-level bindings into the `SchemeEnv`, and
//! instantiating fresh copies at each `Lookup`. `interface::table`/
//! `interface::instance` (TM6) hold the closed interface set and the
//! per-module instance table `solve::PendingInstance`s get checked
//! against; `annotation::table`/`annotation::sensitivity` derive an
//! annotation key's expected type (though nothing yet checks a real
//! annotation *value* against it — see `annotation::table`'s own docs).
//! `ast.rs`/`elaborate.rs` (TM7) give the target Elaborated-AST shape and a
//! fully-working dictionary-*resolution* primitive, but not yet a complete
//! `CExpr` -> `TExpr` tree walk — see `ast.rs`'s own doc comment for
//! exactly what that still needs. There is no public `check_module` entry
//! point until that walk exists.

pub mod annotation;
pub mod ast;
pub mod constrain;
pub mod elaborate;
pub mod error;
pub mod interface;
pub mod solve;
pub mod ty;
pub mod unify;
pub mod var;
