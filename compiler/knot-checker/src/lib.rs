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
//! `Constraint::Let` tree, wires real rigid variables up for signed
//! bindings, and (Fix #5) checks every `CDecl::Instance`'s own methods
//! against its interface too, not just ordinary `CDecl::Fn`s. `solve.rs`
//! (TM5) walks a `Constraint` tree, actually calling `unify`, generalizing
//! top-level bindings into the `SchemeEnv`, and instantiating fresh copies
//! at each `Lookup`. `interface::table`/`interface::instance` (TM6) hold
//! the closed interface set (plus, since Fix #5, each interface's own
//! method *shapes*) and the per-module instance table `solve::
//! PendingInstance`s get checked against; `annotation::table`/
//! `annotation::sensitivity` derive an
//! annotation key's expected type (though nothing yet checks a real
//! annotation *value* against it — see `annotation::table`'s own docs).
//! `ast.rs`/`elaborate.rs` (TM7, completed by Fix #3) give the target
//! Elaborated-AST shape, a fully-working dictionary-*resolution* primitive,
//! and now a complete `CExpr` -> `TExpr` tree walk too. `prelude.rs` (TM8)
//! seeds real `SchemeEnv`/`InstanceTable` entries for every built-in
//! value/instance in the spec, `map`/`foldl`/`foldr`/`filter`/`length`/
//! `pure`/`bind` included (Fix #2). There is still no public `check_module`
//! entry point tying generation+solving+elaboration together into one call:
//! `constrain::decl::constrain_module`, `solve::solve_with_obligations`, and
//! `elaborate::elaborate_module` (see each's own tests) are the pieces, not
//! yet wired into one. `exhaustiveness.rs` (TM9, a stretch goal per the
//! plan) is a fully self-contained pattern-match usefulness checker
//! (Maranget's algorithm) — a warning-only pass, never wired into
//! `check_module` at all since it doesn't need to be.
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
//!   `elaborate.rs`'s own doc comment on why that belongs with a future fix
//!   instead. Skipped one literal step from the gaps plan on inspection:
//!   `Structure::Tuple`/`Record`'s hardcoded rule is gated to `Eq`/`Ord`/
//!   `Show` specifically (matching `Unit`'s own case) rather than applying
//!   to every interface unconditionally — the plan's own code sketch
//!   omitted that gate, which would have wrongly let e.g. `(Int, Int)`
//!   inherit a `Num` instance it should never have.
//! - **Fix #3** (done): `constrain::expr`/`constrain::pattern` now return a
//!   `Typed<TExpr>`/`Typed<TPattern>` (`ast.rs`) per node instead of a bare
//!   `TypeVarId` — Stage A of real elaboration. `LetMember.elaborated_body`
//!   and `constrain_module`/`constrain_let_bindings`'s new return shapes
//!   thread it through `constrain::decl`. `solve::solve_with_obligations`
//!   (`solve::solve` itself unchanged, so its ~15 existing callers didn't
//!   need to) adds a side-table recovering a `Lookup`/`LookupLocal`
//!   reference's own instantiated obligations, since — unlike `BinOp`/
//!   `Negate`, which know theirs already at generation time — those aren't
//!   known until the referenced scheme is instantiated during solving.
//!   `elaborate::elaborate_module` is Stage B: walks every top-level
//!   binding's elaborated body and resolves each obligation it finds, via
//!   the existing `resolve_dictionary`. Found a real gap in the gaps plan's
//!   own Fix #3 sketch while implementing Stage B: it assumed every
//!   obligation resolves concretely, but a genuinely polymorphic binding's
//!   own body (`useEq x y = x == y`, not called anywhere in its own module)
//!   has obligations that are legitimately never concrete *in that module*
//!   — absorbed into the binding's own scheme by `generalize` instead of
//!   left dangling. Reporting that as `NoInstance` would be a real, if
//!   subtle, false positive on valid polymorphic code — `ast::
//!   ObligationResolution::StillAbstract` names this case honestly instead.
//!   Full dictionary-*parameter* codegen for polymorphic bindings (Wadler &
//!   Blott's transform in full) is real, separate, future work; telling the
//!   two cases apart correctly, without silently conflating them, is what
//!   this fix actually commits to.
//! - **Fix #5** (done, closing out the gaps-plan's original five): instance
//!   method bodies are real, type-checked function bodies now, not skipped
//!   entirely — `interface::table::METHODS` restates each of the eight
//!   ordinary interfaces' own methods symbolically (`MethodShape`, relative
//!   to `Self`), and `constrain::decl::constrain_instance` instantiates a
//!   method's shape against its own instance's target (reusing
//!   `instantiate_rigid`, exactly like a signed top-level binding's `ty`)
//!   to get a real expected type to check the body against. The instance's
//!   own declared context becomes `given` facts via a new `Constraint::
//!   Given` — deliberately *not* reusing `Constraint::Let`'s own
//!   `declared`/scheme-installation path, since an instance method's
//!   context is never "dangling" the way `check_ambiguous` cares about (a
//!   zero-arg `Monoid a => List a`-shaped method would otherwise be
//!   wrongly flagged ambiguous, even though the instance's own `given` `a`
//!   is a precondition the dispatch mechanism itself guarantees, not
//!   something a caller has to separately resolve). `Collection`/`Context`
//!   methods (`map`, `bind`, ...) aren't covered — their own shapes are
//!   polymorphic over the constructor itself, needing a `MethodShape`
//!   variant this fix doesn't add — and an instance method's own
//!   elaborated body isn't threaded anywhere yet either (no `LetMember`-
//!   shaped home fits it — never generalized, never scheme-installed,
//!   dispatched by `(interface, head)` rather than by name); both are
//!   documented, narrow follow-ups rather than gaps papered over.

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
