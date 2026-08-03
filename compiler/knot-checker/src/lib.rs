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
//!   `Map`/`Maybe`/`Result`/`IO`) are the whole of it — deliberately *not*
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
//!
//! **Post-gaps-plan audit (2026-08-01)**: closing out the five fixes above
//! wasn't the same as this crate actually working end to end — two gaps
//! found via live testing (not by inspection) were more severe than
//! anything in that plan, in that they broke common-case, not edge-case,
//! programs:
//! - **User-defined ADT constructors had no schemes at all** — `type Shape
//!   = Circle Float` followed by using `Circle` as a value or in a pattern
//!   anywhere was an `UnboundValue` error, full stop, since nothing but
//!   `prelude.rs`'s hand-written built-ins (`Just`, `Ok`, ...) ever
//!   installed a constructor's scheme. Fixed by `constrain::decl::
//!   seed_user_constructors`, called alongside `constrain_module` (not
//!   folded into it — a constructor's type needs no unification or
//!   generalization at all, so it doesn't belong on the `Constraint`/
//!   `solve.rs` path the way an ordinary binding's does).
//! - **Type aliases never expanded** — `type alias Point = { x : Float, y :
//!   Float }` used in a signature built an opaque nominal type that could
//!   never unify with an actual record literal. Fixed in `knot-canonical`
//!   (not here): `resolve::alias::expand_aliases` runs as a whole-module
//!   post-pass after name resolution, substituting every local alias
//!   reference with its own (cycle-checked, recursively-expanded)
//!   definition — the right layer for this, since an alias reference is a
//!   *name* to resolve, not a type-checking judgment.
//!
//! Neither called for rearchitecting anything — both were straightforward
//! missing pieces once found. A third, smaller finding from the same audit
//! is also fixed now: `elaborate::elaborate_module`'s `resolve_one` used to
//! hand any non-`Structure::App` obligation straight to `resolve_dictionary`
//! and treat its `Err` as a real `NoInstance`, misreporting a valid `Tuple`/
//! `Record` obligation `interface::instance::check_instance` would actually
//! accept (it was dormant — nothing calls `elaborate_module` in a real
//! pipeline yet — but a real bug regardless). `resolve_one` now checks
//! `check_instance` first; a structural obligation it confirms is
//! `ObligationResolution::Structural`, not an error.
//! - **Fix #6** (done): closes Fix #5's own documented gap — `Collection`/
//!   `Context` instance methods (`map`, `bind`, ...) are now real,
//!   type-checked bodies too. `interface::table::CtorMethodShape`/
//!   `CTOR_METHODS` restate `map`/`foldl`/`foldr`/`filter`/`length`/`pure`/
//!   `bind`'s own shapes (spec §6.3/§6.4), as a genuinely separate type from
//!   `MethodShape` rather than an extension of it — `Self` here is a type
//!   *constructor*, only ever appearing applied (`SelfApp`), and a method
//!   can introduce its own extra type variables (`map`'s `a`/`b`) that
//!   `MethodShape` has no way to express. `constrain::decl::
//!   constrain_instance` now dispatches `Collection`/`Context` targets to a
//!   new `constrain_ctor_instance`, sharing the actual body-checking logic
//!   (`constrain_method_body_against`) with the ordinary-interface path.
//!   Found a second, unrelated gap while writing this fix's own tests (live
//!   testing again, not inspection): `knot-canonical::prelude::
//!   BUILTIN_INTERFACES` had never been updated when Fix #2 added
//!   `Collection`/`Context` to this crate's own interface table, so
//!   `instance Collection MyType where ...` was rejected at canonicalization
//!   with `UnknownInterface` before ever reaching this crate at all — fixed
//!   there, not here, since `BUILTIN_INTERFACES` is `knot-canonical`'s own
//!   closed list.
//! - **Fix #7** (done, entirely in `knot-canonical`): an extensible-record
//!   alias (spec §3.4's `{ r | field : Type }`) applied to a *concrete*
//!   argument in its own row-extension position — `type alias Selectable a
//!   = { a | isSelected : Bool }` used as `Selectable Foo` — never actually
//!   substituted anything into that slot. `CType::Record`'s extension is
//!   just a variable *name* (`Option<String>`), not a nested `CType`, so
//!   `resolve::alias::substitute_vars`'s old `CType::Record` case cloned it
//!   through unchanged regardless of what `a` had been mapped to. The
//!   practical effect: `Selectable Foo`'s own declared type stayed the
//!   generic, still-row-polymorphic `{ isSelected : Bool | a }` instead of
//!   becoming the concrete closed `{ name : String, isSelected : Bool }`,
//!   so a value's own literal definition failed to unify against its own
//!   signature — a valid program rejected for a reason that had nothing to
//!   do with what should actually be checked. Fixed by a new
//!   `substitute_record_ext`: when the extension name resolves (via
//!   substitution) to another still-free variable, the row just gets
//!   renamed and stays open; when it resolves to a concrete `CType::
//!   Record`, that record's own fields are merged in and its own extension
//!   adopted (closed if it had none); anything else (a nominal type,
//!   tuple, function, unit) is a new `RecordExtensionNotARecord` error, and
//!   a field declared on both sides is `RecordExtensionFieldConflict`
//!   rather than one silently shadowing the other. Confirmed against real
//!   Elm's own reported behavior for this exact pattern: once `Selectable
//!   Foo` resolves concretely, a *downstream* use with a closed, non-
//!   extensible signature demanding fewer fields (`Foo -> String` given a
//!   `Selectable Foo`) is still correctly rejected — for the right reason
//!   (an exact-match closed record short of `isSelected`), not the
//!   previous, unrelated one.
//!
//! **`corpus/semantic/` planning (2026-08-01)** — drafting an interaction
//! matrix for a real semantic test corpus (see that directory's own
//! `README.md`) surfaced three more findings by grounding each matrix row
//! in a live test rather than guessing:
//! - **Fix #8**: `interface::instance::build_instance_table`'s superclass
//!   coherence check used to depend on declaration order *within one
//!   module* — `instance Ord Shape` followed later by `instance Eq Shape`
//!   in the same file wrongly reported a missing `Eq` superclass, purely
//!   because the single forward pass checked `has_instance` against the
//!   table *as built so far*. Now two passes: every instance's own
//!   existence is registered first (duplicates rejected, first occurrence
//!   wins), then every accepted instance's superclasses are checked against
//!   the now-fully-populated table, so order no longer matters.
//! - **Fix #9**: an instance for any interface other than `Eq`/`Ord`/`Show`
//!   targeting a `Tuple`/`Record`/`Unit`/bare-variable/`Fn` shape (e.g.
//!   `instance Semigroup Point where ...` for a record type) used to just
//!   vanish from the table with **no diagnostic at all** (`head_ref`
//!   returns `None` for those shapes — no `Ref` to key an entry by), only
//!   surfacing later as a confusing `NoInstance` wherever a caller tried to
//!   use it. `build_instance_table` now reports a real
//!   `TypeErrorKind::InstanceTargetNotNominal` at the declaration site.
//! - **Fix #10**: added `check::check_module`, the entry point this file's
//!   own doc comment had been noting as missing since TM0. Confirmed live
//!   why this mattered before writing more than a couple of semantic
//!   fixtures: nothing anywhere merged `prelude::seed`'s builtin
//!   `InstanceTable` with a module's own declared one, so a scratch test
//!   for the utterly mundane `addX a b = a + b` on `Float`s spuriously
//!   reported `NoInstance("Num")` purely from a test harness forgetting
//!   that merge — exactly the trap a hand-wired-per-fixture corpus would
//!   keep hitting. `check_module` deliberately does *not* also call
//!   `elaborate::elaborate_module` — seeing both together would double-count
//!   `NoInstance` errors for ordinary bindings while still missing every
//!   obligation from inside an instance method's own body (elaboration only
//!   walks `LetMember`s; instance methods aren't threaded through it at all
//!   yet, per Fix #5's own note above) — `check_pending` alone is the
//!   complete, correct source of truth for "does every obligation in this
//!   module resolve." `check_module`'s own doc comment also names a narrow
//!   gap inherited rather than introduced here: a user re-declaring an
//!   instance a *builtin* type already has isn't flagged as a duplicate,
//!   since coherence-checking happens before the builtin table is merged in.
//!
//! **`corpus/programs/` (2026-08-02)** — a new corpus tier of realistic,
//! outcome-agnostic whole-program examples (see that directory's own
//! README.md) found several more real gaps by trying varied small programs
//! the way an actual user would write them, rather than checking feature
//! interactions systematically:
//! - **Fix #11**: `Semigroup`/`Monoid` had **zero** builtin instances
//!   anywhere — not `String`, not `List`, nothing. `<>` and `empty` failed
//!   on every builtin type, breaking the single most common string-
//!   building pattern outright, and any custom `Show` instance that tried
//!   to build its own output by concatenating strings. `prelude::
//!   seed_instances` now seeds both for `String` and `List` (plain
//!   concatenation needs nothing from the element type, so `requires`
//!   stays empty, unlike `Eq`/`Ord`/`Show`'s recursive requirement).
//! - **Fix #12**: a rigid variable's own `given` facts (from a signature's
//!   declared constraints, or an instance's own context) were never closed
//!   over the interface hierarchy's superclass relationships — `Ord a =>`
//!   only ever recorded `given Ord a`, never the implied `given Eq a`, even
//!   though superclass coherence (`interface::instance::build_instance_
//!   table`) *guarantees* every real `Ord` instance has a matching `Eq`
//!   one. `f :: Ord a => a -> a -> Bool; f x y = x == y` wrongly failed
//!   with `NoInstanceForRigid("Eq")`; same story for `Monoid a =>` never
//!   implying `given Semigroup a`. `solve::insert_given_with_superclasses`
//!   now inserts the full transitive superclass closure (reusing
//!   `interface::table::superclasses`, already used for coherence
//!   checking) at both of `given`'s own insertion points.
//!
//! Both found via real code in `corpus/programs/` (string-building examples
//! for Fix #11; a binary-search-tree's own `Ord`-constrained `contains`
//! using `==`, and a `Monoid`-constrained generic combiner using `<>`, for
//! Fix #12) — see that directory's own README.md for the rest of this
//! round's findings, several still open.
//!
//! **`corpus/programs/`, round 2 (2026-08-02)** — a deeper batch (19 more
//! fixtures: sorting algorithms, monoid-based reports, layered config via
//! record spread, a stack-language interpreter, nested generics, multi-
//! interface constraints) surfaced three more real gaps, two in this crate
//! and one in `knot-syntax`:
//! - **`knot-syntax` parser bug (not numbered here, see that crate's own
//!   `parse/expr.rs`)**: `classify_minus`'s whitespace-only heuristic
//!   answered `Subtraction` for a `-` with symmetric spacing (`(-40.0)`,
//!   `[-5, -6]`) even at the very start of an atom, where no left operand
//!   could possibly exist for it to subtract from — hard parse errors on
//!   `f (-5)`, `[-1, -2, -3]`, and any parenthesized/bracketed leading
//!   negative literal. Fixed by only ever treating that heuristic's
//!   `Subtraction` answer as a real binary operator inside `expr_app`'s own
//!   trailing-argument loop (the one place a preceding operand genuinely
//!   exists to back off to), and treating it as negation everywhere else.
//! - **Fix #13**: a signed binding's own header-type-vs-inferred-type
//!   `Constraint::Equal` (`constrain::decl::constrain_group_chain`) was
//!   solved *after* its body's own constraints, not before -- so a nested
//!   `let` inside the body (e.g. a hand-rolled quicksort's `smaller`/
//!   `larger`) generalized over a parameter-derived variable that hadn't
//!   been unified into the signature's rigid type yet, wrongly treating it
//!   as a fresh, ambient-free, quantifiable variable and dragging its
//!   interface obligation into the nested binding's own scheme --
//!   misfiring `AmbiguousConstraint` on perfectly ordinary code. Fixed by
//!   solving that `Equal` first, so the parameter variable is already
//!   unioned with the rigid one (and thus correctly `ambient`-visible and
//!   `given`-covered) by the time the body's own constraints run.
//! - **Fix #14**: `interface::instance::check_instance`'s recursive
//!   per-argument checks (a parametric instance's own `requires`, e.g.
//!   `instance Ord a => Semigroup (Max a)`'s `Ord` requirement on its own
//!   argument) had no way to resolve a bare *rigid* variable -- `sub::
//!   resolve_structure` returns `None` for a `Rigid` slot by design, so
//!   every such recursive step answered `false` unconditionally, no matter
//!   how thoroughly an enclosing signature/instance context already
//!   established the interface via `given`. A *concrete* pending obligation
//!   like `Semigroup (Max a)` (not itself rigid, so never diverted to
//!   `solve::solve_with_obligations`'s own separate rigid-vs-`given` check)
//!   still has a rigid `a` buried inside once `check_instance` recurses --
//!   this broke recursive/self-referential parametric instances too (e.g.
//!   `instance Show a => Show (Tree a)` calling `show` on child `Tree a`
//!   nodes). Fixed by threading `given` (now also returned by
//!   `solve::solve_with_obligations`, alongside `pending`/`errors`/
//!   `obligations`) through `check_instance`/`check_pending` and
//!   `elaborate`'s own dictionary-resolution functions, consulting it
//!   directly for a rigid variable (its values are already closed over
//!   superclasses by Fix #12, so a plain `contains` suffices).
//!
//! Found via real code in `corpus/programs/` (`numeric/clamp-and-abs.knot`'s
//! `clamp (-40.0) 50.0 raw` for the parser bug; `algorithms/quicksort.knot`
//! and `multi_interface/generic-function-multi-constraint.knot` for Fix #13;
//! `monoids/max-min-via-ord.knot` and `multi_interface/recursive-tree-
//! show.knot` for Fix #14) — see `corpus/programs/README.md` for the rest
//! of this round's findings.
//!
//! **`corpus/programs/`, rounds 3-4, plus a user-directed fix pass
//! (2026-08-02)** — round 3 (18 more fixtures: graph algorithms, an
//! error-handling interpreter, a hand-declared `Num` instance) surfaced one
//! new root cause (operator sections entirely missing as an expression) and
//! confirmed `InstanceTargetNotNominal` was broader than Fix #9 first
//! documented. Rather than stopping at "logged, not yet fixed," every
//! outstanding `corpus/programs` finding through that point was then
//! deliberately closed in one pass, followed by a fourth, monad-`do`-
//! notation-focused round that found one more:
//! - **Elm-style bare operator sections**: `(+)`/`(::)`/`(<>)`/... as
//!   first-class values, desugaring to `\x y -> x op y` at parse time in
//!   `knot-syntax::parse::expr::try_operator_section`. Deliberately no
//!   Haskell-style partial sections (`(+ 1)`/`(1 +)`) — a real design
//!   decision, not a shortcut; those now hard-fail with a message pointing
//!   at the lambda-form alternative instead of silently misparsing.
//! - **`let`-bound local functions can take parameters**: `let go acc rest
//!   = ... in ...` was a hard parse error; `knot-syntax::parse::expr::
//!   let_binding` now runs the identical params-loop a top-level `FnDef`
//!   already has, folding the result into a `Lambda` the same way the
//!   `\acc rest -> ...` workaround already did.
//! - **Custom instances on closed records**: `type alias` expansion erases
//!   every alias reference to its literal `CType` before instance-table
//!   building ever runs, so `InstanceTable` gained a second key space,
//!   `record_entries` (keyed by `RecordKey`, a closed record's own sorted
//!   field-name set) alongside its `Ref`-keyed one — letting a custom
//!   instance override the structural `Eq`/`Ord`/`Show` fallback, and,
//!   more importantly, giving records access to interfaces with no
//!   structural fallback at all (`Num`, `Semigroup`, anything user-
//!   defined). An *open* record target still correctly reports
//!   `InstanceTargetNotNominal` — there's no fixed shape to key by when a
//!   use site could still gain more fields. Two limitations accepted
//!   rather than solved (see `checker_impl_summary.md`'s own "Known gaps"):
//!   field-*name*-only keying, and a record instance's own declared
//!   context never being re-checked against a real argument type.
//! - **Numeric-literal polymorphism**: `constrain::expr`'s `CExpr::IntLit`
//!   now produces a fresh, `Num`-obligated variable instead of a hard-wired
//!   `Int`, unifying with `Float` or any user's own `Num` instance by
//!   ordinary unification. Defaulting to `Int` when nothing else pins it
//!   down needs two separate spots in `solve.rs`, both provably safe
//!   against ever misfiring on a real user-written constraint like `Ord a
//!   =>` (nothing else in this closed language can produce a bare,
//!   non-function, `Num`-polymorphic value): `generalize` defaults one
//!   right before it would become part of some binding's own generalized
//!   scheme (`x = 5`, no signature); `solve_with_obligations`'s own final
//!   sweep catches the shape that never becomes part of *any* scheme's own
//!   quantified variables at all (`f x y = x == y; result = f 1 2`).
//! - **The 2-parameter `Collection`/`Context` fix**: `ty::Structure::
//!   Ctor(Ref)` became `Ctor(Ref, Vec<TypeVarId>)`, carrying whichever
//!   leading arguments of a constructor are already fixed. `unify.rs`'s
//!   own `VarApp`-vs-`App` case now splits the concrete side's own argument
//!   list at `own_len - VarApp's own arg count` instead of demanding an
//!   exact match — the latter had made every 2-parameter `Collection`/
//!   `Context` instance (`Map k v`, `Result e a`) unconditionally fail to
//!   unify at all, breaking `Result`'s own do-notation entirely.
//! - **`Map`'s own key-value API**: `prelude::seed_map_module` seeds
//!   `Ref::Imported(["Map"], _)` schemes for `empty`/`get`/`insert`/
//!   `remove`/`member`/`size`/`isEmpty`/`keys`/`values`/`toList`/
//!   `fromList` — concrete, ordinarily-constrained-polymorphic (`Eq k =>`
//!   wherever a key comparison is needed), not `Collection`/`Context`-
//!   generic, since these are specific to `Map`'s own two-parameter shape.
//! - **Instance methods' own header-vs-body solve order**: `constrain::
//!   decl::constrain_method_body_against` had the identical bug Fix #13
//!   fixed in `constrain_group_chain`, independently present since
//!   instance methods build the same header-`Equal`-after-body shape via
//!   their own, separate code path. A local `let` inside an instance
//!   method's own body (e.g. a `Show` instance computing intermediate
//!   values via `div`/`mod` before formatting them) misfired
//!   `AmbiguousConstraint`. Fixed the same way: solve the header `Equal`
//!   first.
//!
//! `corpus/programs` stood at 98/98 passing here — every finding logged
//! across all four rounds was fixed. See `corpus/programs/README.md` for
//! the full, round-by-round account (including a couple of this session's
//! own authoring mistakes that turned out not to be bugs at all).
//!
//! **A user-directed pass closing four of the five gaps the previous
//! entry's "Known gaps" review had just catalogued, plus a fifth round of
//! `corpus/programs` (2026-08-03)**:
//! - **Exhaustiveness checking is now wired in**: `check::
//!   check_module_with_warnings`, a new sibling of `check_module` (which
//!   keeps its own signature unchanged for its ~30 existing callers),
//!   walks every binding's body and returns a `Vec<exhaustiveness::
//!   Warning>` alongside the usual `Vec<TypeError>` — a wholly separate
//!   channel, since the spec only ever wants a non-exhaustive `case`
//!   reported as a warning, never something that can reject an otherwise-
//!   valid program.
//! - **A record instance's own declared context is now enforced**: a real,
//!   demonstrated unsoundness, not just an incompleteness — `instance Eq a
//!   => Eq { value : a }` type-checked and was accepted into the table,
//!   but its own `Eq a` obligation was silently dropped, never re-checked
//!   against a real call site's field type. New `InstanceEntry.
//!   field_requires` (populated by `instance_field_requires`) closes this.
//! - **Record/tuple instance keys are now type-aware, and a `Tuple` target
//!   can now declare a custom instance too**: a new `CanonicalType` walk
//!   (alpha-equivalent shape, type variables replaced by first-occurrence-
//!   order indices) replaces the old field-name-only `RecordKey`, and
//!   gives the new `TupleKey` the same treatment — `{ value : Int }` and
//!   `{ value : String }` no longer collide as a false-positive
//!   `DuplicateInstance`, and `instance Num (Int, Int)` no longer
//!   incorrectly reports `InstanceTargetNotNominal`.
//! - **Re-declaring an instance a builtin type already has is now
//!   flagged**: `build_instance_table` takes the seeded builtin table as
//!   its own `builtins` parameter, consulted by its coherence pass
//!   alongside the module's own declared instances.
//!
//! (The fifth gap, type-checking annotation *values* against their own
//! derived expected type, was deliberately left open — the largest, least-
//! contained item, deserving separate discussion rather than folding into
//! this pass.) Full detail on all four fixes, including exactly which
//! `corpus/programs/known_gaps/` fixtures graduated into `corpus/semantic/`
//! and where, is in `checker_impl_summary.md`'s own "Known gaps" section,
//! not repeated here.
//!
//! **Round 5 of `corpus/programs`** (13 more fixtures, reaching deeper than
//! rounds 1-4: a user-declared `Collection` instance recursively calling
//! `map`/`foldl` on both its own wrapped `List` and, separately, on a
//! genuinely recursive ADT; the brand-new custom-`Tuple`-instance feature
//! above exercised in realistic sorting/vector-math/priority-queue code;
//! exhaustiveness warnings inside a realistic 4-constructor expression AST
//! and inside an instance method body specifically; annotations spanning a
//! mutually-recursive pair of bindings; `@unravel` on a real multi-argument
//! function) surfaced one new, genuine bug, fixed immediately:
//! - **Fix #15**: `Ordering` (`LT`/`EQ`/`GT`) had no seeded `Eq`/`Ord`/
//!   `Show` instance at all, unlike `Bool`'s identically-shaped `True`/
//!   `False` — so the entirely ordinary idiom `compare a b == LT` was a
//!   hard `NoInstance("Eq")`. `prelude::seed_instances`'s own Eq/Ord/Show
//!   loop now includes `"Ordering"` alongside `Int`/`Float`/`String`/
//!   `Bool`.
//!
//! After that fix, `corpus/programs` stands at 113/113 passing. See
//! `corpus/programs/README.md` for this round's own full account, and
//! `checker_impl_summary.md`'s own "Known gaps" section for what's still
//! left open (one gap now: annotation values never checked against their
//! own derived expected type), including `corpus/programs/known_gaps/`'s
//! own runnable fixture pinning it down.

pub mod annotation;
pub mod ast;
pub mod check;
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
