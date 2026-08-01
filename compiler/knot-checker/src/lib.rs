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
//! exactly what that still needs. `prelude.rs` (TM8) seeds real
//! `SchemeEnv`/`InstanceTable` entries for every built-in value/instance
//! this crate can currently give a correct signature to — see its own doc
//! comment for the one real gap (`map`/`foldl`/... need higher-kinded
//! polymorphism `Structure` doesn't represent yet). There is still no
//! public `check_module` entry point — that needs the `ast.rs` tree-walk
//! gap closed first. `exhaustiveness.rs` (TM9, a stretch goal per the plan)
//! is a fully self-contained pattern-match usefulness checker (Maranget's
//! algorithm) — a warning-only pass, never wired into `check_module` at
//! all since it doesn't need to be.
//!
//! **Post-TM9**: see `knot-checker-gaps-plan.md` at the repo root for a
//! full audit of what TM0-TM9 left unsound (beyond the two already-known
//! deferrals above) and the fix plan being worked through. Fix #1 is done:
//! `let`-bound names now get real let-polymorphism too, not just top-level
//! ones — `constrain::LocalBinding`/`Constraint::LookupLocal`/`solve.rs`'s
//! `local_env` mirror the `Ref::TopLevel`/`Lookup`/`SchemeEnv` machinery
//! for names `Ref::Local` can't otherwise tell apart from an ordinary
//! monomorphic param.

pub mod annotation;
pub mod ast;
pub mod constrain;
pub mod elaborate;
pub mod error;
pub mod exhaustiveness;
pub mod interface;
pub mod prelude;
pub mod solve;
pub mod ty;
pub mod unify;
pub mod var;
