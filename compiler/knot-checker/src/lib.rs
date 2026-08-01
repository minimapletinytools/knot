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
//! every `CExpr`/`CPattern` shape, `Do` included (desugared to `bind`/`pure`
//! calls, Fix #2). `constrain::decl` (TM4) does SCC dependency splitting for
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
//! `SchemeEnv`/`InstanceTable` entries for every built-in value/instance in
//! the spec, `map`/`foldl`/`foldr`/`filter`/`length`/`pure`/`bind` included
//! (Fix #2). There is still no public `check_module` entry point — that
//! needs the `ast.rs` tree-walk gap closed first. `exhaustiveness.rs` (TM9,
//! a stretch goal per the plan) is a fully self-contained pattern-match
//! usefulness checker (Maranget's algorithm) — a warning-only pass, never
//! wired into `check_module` at all since it doesn't need to be.
//!
//! **Post-TM9**: see `knot-checker-gaps-plan.md` at the repo root for a
//! full audit of what TM0-TM9 left unsound (beyond the two already-known
//! deferrals above) and the fix plan being worked through.
//!
//! - **Fix #1** (done): `let`-bound names now get real let-polymorphism too,
//!   not just top-level ones — `constrain::LocalBinding`/`Constraint::
//!   LookupLocal`/`solve.rs`'s `local_env` mirror the `Ref::TopLevel`/
//!   `Lookup`/`SchemeEnv` machinery for names `Ref::Local` can't otherwise
//!   tell apart from an ordinary monomorphic param.
//! - **Fix #2** (done): `map`/`foldl`/`foldr`/`filter`/`length`/`pure`/
//!   `bind` (spec §6.3/§6.4) now have real, polymorphic-over-a-type-
//!   constructor signatures — `ty::Structure::Ctor`/`VarApp`,
//!   `Substitution::fresh_ctor_unbound`, and two new `Collection`/`Context`
//!   interfaces (`interface::table`, seeded in `prelude.rs` for `List`/
//!   `Map`/`Option`/`Result`/`IO`) are the whole of it — deliberately *not*
//!   general kind polymorphism, just enough closed-set machinery for these
//!   seven built-ins (see `ty::Structure::VarApp`'s own doc comment). `Do`
//!   (`constrain::expr::desugar_do`) is real now too, since it falls
//!   straight out of `bind`/`pure` having real schemes.
//! - **Fix #4** (done): `interface::instance::check_instance` replaces the
//!   old flat `has_instance` lookup at both its call sites
//!   (`check_pending`, `elaborate::resolve_dictionary`) with a real
//!   recursive check — `InstanceEntry.requires` records a parametric
//!   instance's own positional obligations (`instance Eq a => Eq (List
//!   a)` -> "position 0 needs `Eq`"), populated for both declared and
//!   hand-written built-in instances, and `Tuple`/`Record`/`Unit` get
//!   hardcoded structural `Eq`/`Ord`/`Show` rules instead of a table
//!   lookup (no `Ref` to key them by). `List Weird`'s `Eq` and `(Weird,
//!   Int)`'s `Eq` are now correctly rejected instead of silently ignored.
//!   Dictionary *construction* (as opposed to existence-checking) for
//!   structural obligations is deliberately still out of scope — see
//!   `elaborate.rs`'s own doc comment on why that belongs with Fix #3
//!   instead. Skipped one literal step from the gaps plan on inspection:
//!   `Structure::Tuple`/`Record`'s hardcoded rule is gated to `Eq`/`Ord`/
//!   `Show` specifically (matching `Unit`'s own case) rather than applying
//!   to every interface unconditionally — the plan's own code sketch
//!   omitted that gate, which would have wrongly let e.g. `(Int, Int)`
//!   inherit a `Num` instance it should never have.

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
