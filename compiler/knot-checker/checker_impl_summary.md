# Knot Type Checker (`knot-checker`) — Implementation Summary

Covers what's actually built, as of this pass. This document was originally
written right after TM0–TM9 (`knot-type-checker-plan.md`) first landed; a
great deal has happened since — a public `check_module` entry point, real
higher-kinded `Collection`/`Context` polymorphism, a complete `CExpr` ->
`TExpr` elaboration tree walk, custom instances on record types, numeric-
literal polymorphism, a full `Map` stdlib module, and ~15 real bugs found
by throwing realistic whole programs at the checker (`corpus/programs/`)
rather than only checking features in isolation. This revision folds all
of that in. See `knot-type-checker-plan.md` for the original design
discussion, `knot-checker-gaps-plan.md` for the post-TM9 audit, `lib.rs`'s
own doc comment for the complete, dated, blow-by-blow fix log this
document summarizes, and `language-spec-notes.md` for the language being
checked.

**A public `check_module` entry point exists** (`check.rs`, Fix #10):
`constrain::decl::constrain_module` → `solve::solve_with_obligations` →
`interface::instance::check_pending`, seeded with `prelude::seed` and a
module's own `build_instance_table`, all wired into one call. It
deliberately does *not* also call `elaborate::elaborate_module` — see
TM7 below for why.

---

## The pipeline

```
"f x = x + 1"
   │  knot-syntax::parse
   ▼
Expr/Pattern      (raw AST — names are just strings)
   │  knot-canonical::canonicalize_module
   ▼
CExpr/CPattern    (Canonical AST — every name is a Ref: Local/TopLevel/Imported/Builtin/Unresolved)
   │  knot-checker  (this crate)
   ▼
TExpr/TPattern    (Elaborated AST — a type on every node, Stage A of TM7)
```

`knot-checker`'s own internal pipeline is **generate constraints, then
solve them separately** — the single biggest architectural choice, borrowed
deliberately from Elm's compiler (plan §2):

```
CExpr/CPattern
   │  constrain::{expr, pattern, decl}   (TM3/TM4) -- also builds TExpr/TPattern (Fix #3)
   ▼
Constraint tree   (nothing checked yet — just "here's what must hold")
   │  solve::solve / solve_with_obligations  (TM5)
   ▼
Substitution (mutated) + SchemeEnv + Vec<PendingInstance> + Vec<TypeError> + given map
   │  interface::instance::check_pending  (TM6)
   ▼
Vec<TypeError>  (fully resolved, including instance-table lookups)

-- separately, not part of check_module's own pipeline (see TM7) --
(TExpr tree, ObligationMap)
   │  elaborate::elaborate_module  (TM7 Stage B)
   ▼
HashMap<(interface, TypeVarId), ObligationResolution>  (Concrete/Structural/StillAbstract)
```

---

## TM0 — `var.rs`: the arena substitution

A `Substitution` is an arena (`Vec<Slot>`); a `TypeVarId` is an index into
it. Each slot is `Unbound` (a genuine unknown), `Link` (a union-find
redirect), `Bound(Structure)` (resolved to a concrete shape), or `Rigid`
(a signature's own type variable — see TM4). Unchanged since TM0.

## TM1/TM2 — `unify.rs`: structural unification

Given two `TypeVarId`s, `unify` decides whether they can be the same type
and mutates the `Substitution` to record it:

- **Structural comparison** — `App`/`Fn`/`Tuple`/`Unit` recurse into
  matching shapes, fail on a head mismatch.
- **Occurs check** — refuses to bind `a` to `List a` (an infinite type).
- **Rigid variables** — a rigid var unifies fine with a fresh flexible var
  (an ordinary function call) but never with another rigid var or a
  concrete type. This is what makes a signature an *enforced promise*
  rather than decoration — without it, nothing would stop a function's body
  from silently assuming its generic parameter is secretly `Int`.
- **Record field-gathering** (TM2) — two records unify by splitting fields
  into "both have it" (unify pairwise) vs. "only one side has it" (that
  side needs an open row-variable to absorb the difference, or it's a
  mismatch). Every reconciliation routes back through the top-level
  `unify` rather than a raw bind, which is what makes *chained* open
  records flatten correctly via plain recursion, and what makes a rigid
  row variable (`{ r | x : Float, y : Float }`) correctly refuse to be
  forced into a concrete shape — for free, no extra code needed.
- **`Ctor`/`VarApp`** (added by Fix #2, extended this session): a
  constructor-*variable*-headed application (`f a` in `map :: (a -> b) -> f
  a -> f b`) unifies against a concrete `App` by pinning the variable to a
  `Structure::Ctor`, now itself *partially applicable* — `Ctor(Ref,
  Vec<TypeVarId>)`, carrying whichever leading arguments of a
  2-parameter constructor (`Map k v`, `Result e a`) are already fixed once
  the `VarApp`'s own trailing argument is split off the end. Originally
  demanded an *exact* arity match, which made every 2-parameter
  `Collection`/`Context` instance unconditionally fail to unify at all
  (found via `corpus/programs`, see the "Since this summary was written"
  section below).

`Sensitivity` is stubbed here as an ordinary opaque one-argument type
(`Structure::App`, same treatment as `Maybe`) — still true today, no
recursive expansion into record/tuple shape (spec §9.6 is still just a
stub; see `annotation::sensitivity`, and "Known gaps" below).

## TM3 — `constrain::{expr, pattern}`: constraint generation

Each function is bottom-up: constrain the children, get their `TypeVarId`s
back, combine them via `Constraint::Equal`/`HasInstance`. Since Fix #3,
every one of these functions returns a `Typed<TExpr>`/`Typed<TPattern>`
(`ast.rs`) — the fully-typed node itself, not just a bare `TypeVarId` — so
this stage builds *both* the constraint tree TM5 solves *and* Stage A of
real elaboration (TM7) in the same walk. A few illustrative cases:

- An integer literal is `Num a => a`, a fresh variable carrying its own
  `HasInstance("Num", _)` obligation — **not** hard-wired to `Int` (that
  changed this session; see "Since this summary was written"). A float
  literal is still unconditionally, immediately `Float` — there's no
  sensible integer reading of a decimal-point literal to be polymorphic
  over.
- `App(f, arg)`: constrain `f`/`arg`, invent a fresh result variable, push
  one constraint — `f`'s type must equal `arg`'s type -> result.
- `Var(Ref::Local(x))`: no constraint at all — look `x` up in `LocalScope`
  (a stack of name→type maps for lambda/case bindings, which are never
  generalized) and return that type directly.
- `Var(Ref::TopLevel/Builtin/Imported(...))`: push a `Constraint::Lookup` —
  "look this name's scheme up later, instantiate it fresh, unify the
  result against this variable." A `let`-bound name generalizable within
  its own block instead pushes `Constraint::LookupLocal` (Fix #1) — see
  TM4/TM5.
- `BinOp`/`Negate`: a small, closed operator → interface table (`Add` needs
  `Num` and unifies both operands; `Pow` is `(Num a, Integral b) => a -> b
  -> a`).
- `Let`: delegates to `constrain::decl` (see TM4).
- `Do` (spec §6.4/§8): **real now** (Fix #2) — desugars straight to
  `bind`/`pure` calls before ever reaching the rest of this module, once
  `Collection`/`Context` gave those two names real, checkable signatures.
  `do { x <- e1; rest }` becomes `bind e1 (\x -> rest)`; a bare statement
  becomes `bind e1 (\_ -> rest)`.

**Still not implemented**: `Annotated` is handled, but deliberately
shallow — annotation *values* still aren't constrained/checked here at
all (`annotation::table` only derives what type a value *would* need,
nothing calls that from here to check a real one against it; see "Known
gaps").

## TM4 — `constrain::decl`: SCC splitting and `Constraint::Let`

The hard problem: bindings can be mutually recursive (`isOdd`/`isEven`),
so you can't check either one "first," but you also can't just treat the
*whole* module as one big blob, or `identity` used at two different types
in the same module would be forced to agree on one type.

**Tarjan's SCC** over the "who references whom" graph splits bindings into
strongly-connected components, processed in dependency order. Each
component becomes a `Constraint::Let { members, header_con, body_con }`,
nested so a dependency's `Let` is always the *outer* one. Self/mutual
references *within* one group resolve monomorphically (via `LocalScope`,
not `Lookup`) while that group's own bodies are being constrained; anything
referencing an *already-processed* group still goes through the deferred
`Lookup` path, which is exactly what lets it get properly generalized
later.

**Rigid variables, for real**: a signed binding's declared type variables
become `Substitution::fresh_rigid` slots; its params/body are constrained
directly against the signature's own shape, so `unify`'s rigid-vs-concrete
rejection actually gets exercised.

**`let`-bound names get real let-polymorphism too, not just top-level ones
(Fix #1)** — the paragraph in the previous revision of this document about
`let`-bound names being forced monomorphic as "a documented, sound
simplification" is **superseded**: `constrain::LocalBinding`/
`Constraint::LookupLocal`/`solve.rs`'s own `local_env` now mirror the
`Ref::TopLevel`/`Lookup`/`SchemeEnv` machinery, letting a `let`-bound name
be `promote_to_generalizable`d and used polymorphically at two different
types within its own block, exactly like a top-level one. `Ref::Local`
still covers both `let` and truly-monomorphic lambda/case/do params — the
distinction now lives in *which constraint* a reference to it emits
(`LookupLocal` vs. nothing at all), not in whether it can be polymorphic.

**Instance method bodies are real, type-checked function bodies (Fix #5,
extended by Fix #6)** — `constrain_instance` instantiates each method's
shape (`interface::table::METHODS`/`CTOR_METHODS`, covering both the eight
ordinary interfaces and `Collection`/`Context`) against the instance's own
target, and checks the body against it exactly like a signed top-level
binding. `constrain_method_body_against` is genuinely a second copy of
this logic, not a call into the ordinary-binding path — see "Since this
summary was written" for a bug that fell out of that duplication.

**Two real bugs found here originally** (both via the flagship "`identity`
used at two different types" end-to-end test — see TM5):

1. A group's own scope frame originally stayed open through the *entire*
   rest of the binding chain. Fixed: the frame now closes immediately
   after a top-level group's own bodies are constrained.
2. (See TM5 — the companion bug in `generalize` itself.)

## TM5 — `solve.rs`: generalize, instantiate, real let-polymorphism

The driver that actually calls `unify`. Walks the `Constraint` tree:

- `Equal` → call `unify` directly.
- `HasInstance` → collect into a `pending` list, classified only after the
  *whole* tree's `Equal`s have run: rigid (checked against `given` facts,
  now closed over the interface hierarchy's own superclasses — `Ord a =>`
  implies `given Eq a` too, Fix #12) or concrete (left for TM6).
- `Lookup`/`LookupLocal` → instantiate whatever scheme is currently
  installed (fresh copy of each quantified variable), unify the fresh copy
  against the reference's own type.
- `Let` → push the group's `header_ty`s onto an `ambient` stack, solve
  `header_con`, then for each member: a signed one's scheme is just its
  signature restated; an unsigned one needs the real **`generalize`** —
  textbook `ftv(ty) \ ftv(ambient)` — then the **ambiguous-CAF check**
  (arity-based replacement for Haskell's monomorphism restriction: a
  zero-argument binding generalized over a variable that still carries an
  interface obligation is an error, full stop — *except* a still-dangling
  bare `"Num"` obligation, which **defaults to `Int`** instead of erroring
  or staying pending forever; see "Since this summary was written").

**The companion bug to TM4's #1**: `ambient` originally still contained
the *very member being generalized*. Fixed by popping a group's own
`header_ty`s immediately after solving `header_con`, before generalizing
them.

## TM6 — `interface::{table, instance}`, `annotation::{table, sensitivity}`

**`interface::table`**: the closed interface set (`Eq`, `Ord`, `Show`,
`Semigroup`, `Monoid`, `Num`, `Fractional`, `Integral`, plus `Collection`/
`Context` since Fix #2) and their superclasses — a small hardcoded table,
since the set can never grow. Also holds each interface's own method
*shapes* (`METHODS`/`CTOR_METHODS`, Fix #5/#6) for checking instance
method bodies.

**`interface::instance`**: `InstanceTable`, built from a module's own
`instance` declarations, checking **coherence** (at most one instance per
head) and **superclass existence**. `check_instance` (Fix #4) is the real,
*recursive* answer to "does `ty` have `interface`" — a parametric
instance's own `requires` (`instance Eq a => Eq (List a)`'s `Eq a`) is
checked against each corresponding argument, including through a rigid
variable buried inside an otherwise-concrete type (Fix #14) and through a
`Collection`/`Context`-style partially-applied `Ctor` (this session).
`Unit` (and any `Record`/`Tuple` with no matching custom instance) get
hardcoded structural `Eq`/`Ord`/`Show` rules. **A closed `Record` or a
`Tuple` can now declare a real, custom instance for *any* interface**
(this session) — `InstanceTable` keeps two further key spaces,
`record_entries` and `tuple_entries`, keyed by `RecordKey`/`TupleKey`
rather than `Ref`, since `type alias` expansion has already erased any
nominal name by the time this table is built. Both keys are genuinely
*type*-aware, not just name/arity-aware: a `CanonicalType` walk replaces
every type variable with a first-occurrence-order index (so `{ value : a
}` and `{ value : b }` key identically — same shape — while `{ value :
Int }` and `{ value : String }` key differently — genuinely different),
closing the false-positive `DuplicateInstance` collision two same-field-
name-but-different-type records used to hit. `check_instance` tries a
matching record/tuple instance first, falling back to the structural
derivation only when none exists — letting a custom `Eq`/`Ord`/`Show`
correctly *override* the automatic one, and giving both access to
interfaces (`Num`, `Semigroup`, anything user-defined) that have no
structural fallback at all. A record instance's own declared context
(`instance Eq a => Eq { value : a }`'s `Eq a`) is also now actually
re-checked against a real call site's own field type, via
`InstanceEntry.field_requires` — previously accepted into the table but
never enforced (a real, demonstrated unsoundness before this session's
fix).

**A user ADT can now `deriving (...)` `Eq`/`Ord`/`Show`/`Semigroup`/
`Monoid` too** (Fix #17, this session) — `derive_instances`, called from
`build_instance_table` alongside its existing hand-written-instance scan,
walks every field of every constructor for each derived interface and
either computes a positional `requires` (a bare use of one of the type's
own declared parameters contributes a requirement, exactly like a
hand-written `instance Eq a => Eq (List a)`; a *canonical* self-reference
— the type's own name applied to its own parameters in the same order —
contributes nothing, its correctness following inductively once the entry
itself is inserted; a concrete zero-argument type needs an existing
instance already in `builtins`, deliberately builtins-only for now, not
cross-referencing another type in the same module) or reports a new
`CannotDeriveInterface` error for a field shape too complex for this pass
(a nested generic argument, a tuple/record, a non-canonical self-
reference) or an interface this feature doesn't support deriving at all
(`Num`/`Fractional`/`Integral` — mechanically possible via the same
pointwise trick but with no single obviously-correct meaning for `*`;
`Collection`/`Context` — needs real traversal logic, not a field scan).
`Semigroup`/`Monoid` are additionally restricted to a single-constructor
type (no sane `<>` between two different constructors of a sum type). A
derived instance is folded into the *same* `declared` list a hand-written
`instance` produces, so coherence and superclass checking apply with zero
special-casing — `deriving (Ord)` without `Eq` correctly reports
`MissingSuperclassInstance`, and `deriving (Eq)` alongside a hand-written
`instance Eq` on the same type correctly reports `DuplicateInstance`.
There is deliberately no actual method-body synthesis anywhere in this —
since there's still no execution backend at all (see TM7's own "Full
dictionary-passing codegen doesn't exist" note), a derived instance is a
trusted table entry with no body to check, exactly like a builtin one.

**`annotation::table`**: derives an annotation key's expected type — fixed
shapes for `nodeId`/`position`/`label`/`doc`/`color`/`group`/`collapsed`,
and the real derivation rule for `unravel`. **Still true**: nothing yet
checks a real annotation *value* against the derived type — see "Known
gaps".

## TM7 — `ast.rs`/`elaborate.rs`: Elaborated AST — **now a complete tree walk (Fix #3)**

The plan's own framing calls this "the crate's actual deliverable." This
was the biggest gap in the original version of this document — it no
longer is.

**Stage A, complete**: `constrain::expr`/`pattern` (TM3) now return a
`Typed<TExpr>`/`Typed<TPattern>` per node directly, not a bare `TypeVarId`
— there is a real `CExpr -> TExpr` tree for every binding
(`LetMember.elaborated_body`).

**Stage B, real but intentionally scoped down, and *not* wired into
`check_module`**: `elaborate::elaborate_module` walks every top-level
binding's elaborated body and resolves each obligation it carries via
`resolve_dictionary`, producing `ObligationResolution::{Concrete,
Structural, StillAbstract}` — telling apart "resolved to a real
`Dictionary`," "structurally derived, no `Dictionary` representation
exists for it," and "still generic, correctly deferred to the binding's
own callers" (the last one matters: without it, `useEq x y = x == y`'s own
never-instantiated `Eq` obligation would be misreported as `NoInstance`).
`check_module` deliberately never calls this — doing so today would
double-count `NoInstance` errors for ordinary bindings while *also*
missing every obligation from inside an instance method's own body
(`elaborate_module` only walks `LetMember`s; instance methods have no
`LetMember`-shaped home — never generalized, never scheme-installed,
dispatched by `(interface, head)` instead of by name). `check_pending`
alone remains the complete, correct source of truth for "does every
obligation in this module resolve" — instance-method obligations *are*
caught (as existence errors), just never elaborated into a resolved
dictionary.

**Still out of scope entirely**: actually compiling a polymorphic binding
to *take* an extra dictionary parameter and threading a caller's own
dictionary down through it (the full Wadler & Blott transform) — there is
no codegen anywhere in this project yet for that to plug into.

## TM8 — `prelude.rs`: built-in instances and schemes

Seeds a real `SchemeEnv`/`InstanceTable`. **The biggest gap in the
original version of this document is now closed**: `map`, `foldl`,
`foldr`, `filter`, `length`, `pure`, `bind` (spec §6.3/§6.4) *are* seeded,
with real, polymorphic-over-a-type-constructor signatures (Fix #2) —
`ty::Structure::Ctor`/`VarApp` and `Substitution::fresh_ctor_unbound` give
a constructor variable the same `Unbound`/`Bound` machinery any other type
variable already has, constrained by two new interfaces, `Collection`
(`List`, `Map`) and `Context` (`Maybe`, `Result`, `IO`, `List`), rather
than full kind polymorphism (deliberately — see `ty::Structure::VarApp`'s
own doc comment on why this stays a closed, seven-built-in-type mechanism,
not something a user's own Knot source can extend to a new higher-kinded
type... with one caveat: a user's own type constructor *can* get a
`Collection`/`Context` **instance** declared for it, e.g.
`corpus/programs/contexts/either-like-custom-result-context.knot`'s own
`Outcome e a`; what stays closed is the *interface set* itself, not which
constructors can join `Collection`/`Context`).

**`Map`'s own qualified key-value API is real too** (this session, closing
a documented gap): `Map.empty`/`get`/`insert`/`remove`/`member`/`size`/
`isEmpty`/`keys`/`values`/`toList`/`fromList`, seeded as concrete,
ordinarily-constrained-polymorphic schemes (`Eq k =>` wherever a key
comparison is needed) under `Ref::Imported(["Map"], _)`.

**`Ordering` (`LT`/`EQ`/`GT`) now has seeded `Eq`/`Ord`/`Show` instances
too** (Fix #15, found via `corpus/programs`'s own round 5) — previously
missing entirely despite being an ordinary 3-ctor ADT the same shape as
`Bool`'s `True`/`False`, so the everyday idiom `compare a b == LT` was a
hard `NoInstance("Eq")`.

**A user's own signed function can now be genuinely generic over
`Collection`/`Context` too** (Fix #16, this session) — previously only the
seven hand-built prelude schemes (`map`/`foldl`/`foldr`/`filter`/`length`/
`pure`/`bind`) could ever produce a `Structure::VarApp`, since the surface
grammar had no way to write "type variable applied to an argument" at all
(`f a` parsed as a bare `f`, silently leaving `a` as leftover input).
`knot-syntax::ast::ty::Type` gained a `VarApp(String, Vec<Type>)` variant
(mirrored by `knot-canonical::ast::CType::VarApp`), and `constrain::decl`'s
`instantiate_rigid`/`instantiate_flexible` gained a matching case that
builds a real `Structure::VarApp` around a rigid (or flexible, for a data
constructor's own field) variable. Nothing else needed to change:
`unify.rs`'s own `VarApp`-vs-`App`/`VarApp`-vs-`VarApp` cases and
`check_instance`'s rigid-defers-to-`given` dispatch already handled this
shape generically (Fix #2's own `Collection`/`Context` schemes were
already exercising both), so `Collection f => f a -> f b` written as an
ordinary user signature just works once the grammar can parse it and the
checker can build the right `Structure` for it — see `corpus/semantic/
valid/collections/user-signature-generic-over-collection.knot`.

**Numeric-literal polymorphism is real now too** (this session, reversing
this document's own former praise of "no Haskell-style defaulting" as a
design win): an int literal is `Num a => a`, unifying with `Int`,
`Float`, or any user's own `Num` instance depending on context, and
defaulting to `Int` (in exactly the two places nothing else can pin it
down — see TM5, and the "Since this summary was written" section) when
nothing does. `f n = 1.0 + fromIntegral n` still correctly generalizes `n`
to a free, `Integral`-constrained variable (`fromIntegral`'s own
signature, unrelated to literal defaulting, is untouched) — that part of
the original test's own reasoning still holds.

## TM9 — `exhaustiveness.rs`: pattern-match usefulness checking (stretch)

Implemented for real: Maranget's usefulness-checking algorithm. A
`CtorTable` tracks each constructor's sibling set; `is_useful` recursively
specializes a pattern matrix by constructor, falling back to a "default
matrix" when the constructors seen don't exhaust the type (or can't in
principle — `Int`/`String` literals have no enumerable complete set).
`is_exhaustive` and `redundant_arms` are both just different questions
asked of the same `is_useful` core.

**Fixed**: `check::check_module_with_warnings` now calls this
(`check_module_exhaustiveness`, added directly to this module) for every
binding's own body. A `case` missing an entire constructor arm
(`Circle`/`Square` handled, `Triangle` silently missing) produces a real
`Warning` now, never a `TypeError` — see "Known gaps" for why that split
matters and TM7's own `check_module`/`check_module_with_warnings` split
this reuses.

---

## Since this summary was written: everything from Fix #1 through this session

The original TM0–TM9 pass left a five-item gaps plan
(`knot-checker-gaps-plan.md`), which is now fully closed (Fix #1–#6), plus
a steady stream of further fixes found by grounding decisions in live
tests and, starting 2026-08-02, an entire new corpus tier
(`corpus/programs/`) of realistic whole programs written the way an
actual user would, rather than checked feature-by-feature. **`lib.rs`'s
own doc comment is the complete, dated blow-by-blow** — this is a
condensed pointer, not a replacement:

- **Fix #1**: real let-polymorphism for `let`-bound names (TM4/TM5 above).
- **Fix #2**: real `Collection`/`Context` higher-kinded signatures, and
  real `Do`-notation as a consequence (TM3/TM8 above).
- **Fix #3**: the complete `CExpr -> TExpr` elaboration tree walk (TM7
  above), plus `ObligationResolution::StillAbstract` for a genuinely
  still-polymorphic obligation.
- **Fix #4**: `check_instance`'s real recursive parametric-instance check,
  plus hardcoded structural `Eq`/`Ord`/`Show` for `Tuple`/`Record`/`Unit`
  (TM6 above).
- **Fix #5/#6**: instance method bodies (ordinary interfaces, then
  `Collection`/`Context`) are real, type-checked function bodies (TM4
  above).
- **Two post-gaps-plan finds**: user-defined ADT constructors had no
  schemes at all (`seed_user_constructors`); `type alias` never expanded
  (fixed in `knot-canonical`).
- **Fix #7** (`knot-canonical`): an extensible-record alias applied to a
  concrete argument in its own row-extension position never actually
  substituted anything in.
- **Fix #8**: superclass coherence checking depended on declaration order
  within one module; now two passes.
- **Fix #9**: a non-`Eq`/`Ord`/`Show` instance targeting a `Tuple`/
  `Record`/`Unit`/bare-variable/`Fn` shape used to vanish with no
  diagnostic; now `InstanceTargetNotNominal`. (This session narrowed the
  scope of what this actually blocks — see below.)
- **Fix #10**: added `check::check_module` itself.
- **Fix #11**: `Semigroup`/`Monoid` had zero builtin instances anywhere
  (`<>`/`empty` failed on every builtin type, `String` included).
- **Fix #12**: `given` facts weren't closed over superclasses (`Ord a =>`
  didn't imply `given Eq a`).
- **A `knot-syntax` parser bug**: unary negation wasn't recognized at the
  very start of a parenthesized/bracketed (sub)expression (`(-40.0)`,
  `[-5, -6]`, `f (-5)` all hard-failed).
- **Fix #13**: a signed binding's header-vs-inferred-type `Equal`
  constraint solved *after* its own body, not before, misfiring
  `AmbiguousConstraint` on a nested `let` inside an ordinary function.
- **Fix #14**: `check_instance`'s recursive per-argument check had no way
  to resolve a bare rigid variable, breaking parametric instances whose
  own `requires` needed a `given`-only type (`instance Ord a => Semigroup
  (Max a)`) and self-referential ones (`instance Show a => Show (Tree a)`).
- **Elm-style bare operator sections**: `(+)`, `(::)`, `(<>)`, ... as
  first-class values — there was previously no expression-level grammar
  for this at all, only for naming an interface method inside
  `instance`/`interface ... where`. Deliberately no Haskell-style partial
  sections (`(+ 1)`/`(1 +)`). Does **not** desugar to a lambda (spec
  §7.6) — `Expr::OpRef`/`CExpr::OpRef`/`TExpr::OpRef` is its own node
  through the whole pipeline, typed as the operator's curried function
  type (reusing `constrain_binop` against two fresh type variables), so
  `(+) a b` type-checks via ordinary `App`, identically to `a + b`.
- **`let`-bound local functions can take parameters**: `let go acc rest =
  ... in ...` was a hard parse error; only the `\acc rest -> ...`-lambda
  workaround used to work.
- **Custom instances on closed records** (TM6 above) — this **narrows**
  Fix #9's own original scope: a record target is no longer unconditionally
  `InstanceTargetNotNominal`, only an *open* (row-polymorphic) one still is.
- **Numeric-literal polymorphism** (TM3/TM5/TM8 above) — this **reverses**
  this document's own former framing of "no defaulting" as a settled
  design decision.
- **The 2-parameter `Collection`/`Context` fix** (TM1/TM2 above) — `Map`/
  `Result` were unconditionally unable to unify as a `Collection`/`Context`
  target at all before this; do-notation over `Result`, and `map`/
  `filter`/`foldl` over `Map`'s own values, simply didn't work.
- **`Map`'s own key-value API** (TM8 above).
- **Instance methods' own header-vs-body solve order**: the exact same bug
  as Fix #13, independently present in `constrain_method_body_against`
  (instance methods' own separate code path from ordinary bindings),
  found the same way (a real fixture, not inspection).

All fixes in this section were found the same way: writing (or reading)
real code and watching it fail for the wrong reason, not by auditing the
implementation for suspicious-looking gaps. `corpus/programs/README.md`
has the full round-by-round account, including several findings that
turned out to be this session's own authoring mistakes rather than real
bugs — also worth reading as a record of what *didn't* need fixing.

---

## Known gaps, all documented in place, none silently papered over

Verified against the current code (not just old doc comments) while
writing this revision — see `corpus/programs/known_gaps/` for a runnable
fixture demonstrating each of the ones a `.knot` program can actually
exercise.

- **Fixed. `exhaustiveness.rs` is now wired in** — `check::
  check_module_with_warnings` (a new sibling of `check_module`, which
  keeps its own existing signature so its ~30 existing callers don't have
  to change) walks every binding's body, checks every `CExpr::Case` it
  finds via `exhaustiveness::check_module_exhaustiveness`, and returns a
  `Vec<exhaustiveness::Warning>` alongside the usual `Vec<TypeError>` — a
  wholly separate channel, never able to turn a program `check_module`
  used to accept into one it now rejects (the spec only ever wants a
  warning here). `corpus_report` now prints `OK*` plus the warning list
  for a fixture that type-checks cleanly but still has one; see
  `corpus/programs/patterns/non-exhaustive-case-warns.knot`.
- **Fixed. A record instance's own declared context is now enforced at use
  sites** — was a real, demonstrated unsoundness, not just an
  incompleteness: `instance Eq a => Eq { value : a } where ...` type-
  checked and was accepted into the table, but `interface::instance::
  instance_requires` only ever populated `requires` for a `CType::Named`
  target, so a `CType::Record` target's own declared constraints were
  silently dropped — `check_instance` confirmed `Eq { value :
  SomeTypeWithNoEqInstance }` held (the *shape* has an instance) without
  ever verifying `SomeTypeWithNoEqInstance` itself has `Eq`. New
  `instance_field_requires` (the record-target counterpart of
  `instance_requires`, keyed by field *name* instead of positional index)
  populates a new `InstanceEntry.field_requires`, which `check_instance`'s
  own `Structure::Record` arm now checks recursively against each named
  field's real argument type, exactly like `Structure::App`'s existing
  positional `requires` check. Moved from `known_gaps/` to
  `corpus/semantic/invalid/interfaces/record-instance-context-not-
  satisfied.knot` (plus a `valid/` companion proving the fix isn't overly
  conservative when the context genuinely is satisfied).
- **Fixed. Record/tuple instance keys are now type-aware, and a `Tuple`
  target can now declare a custom instance too** — previously the
  record-instance table keyed purely on the sorted field-*name* set, so
  two unrelated record aliases sharing an identical field-name set but
  different field *types* (`{ value : Int }` vs `{ value : String }`)
  wrongly collided as a `DuplicateInstance`; and a `Tuple` target
  (`instance Semigroup (Int, Int) where ...`) was rejected outright as
  `InstanceTargetNotNominal`, even though the equivalent record case
  already worked. Both fixed together, since they share the same root
  cause: a new `CanonicalType` walk (`canonicalize_ctype` over a fresh
  declaration's own `CType`, `canonicalize_structure` over a real call
  site's resolved `Structure`) replaces every type variable with a
  first-occurrence-order index, giving `RecordKey`
  (`Vec<(String, CanonicalType)>`) and the new `TupleKey`
  (`Vec<CanonicalType>`) genuine type-awareness rather than just
  name/arity-awareness — `{ value : a }`/`{ value : b }` still key
  identically (same shape, arbitrary variable spelling) while `{ value :
  Int }`/`{ value : String }` now correctly key differently.
  `InstanceTable` gained a third parallel key space, `tuple_entries`
  (mirroring `record_entries`), and `check_instance`'s `Structure::Tuple`
  arm now tries a matching custom instance first, exactly like the
  `Record` arm, falling back to the structural `Eq`/`Ord`/`Show`
  derivation only when none exists. Moved from `known_gaps/` to
  `corpus/semantic/valid/interfaces/record-instance-keyed-by-type-not-
  just-name.knot` and `.../tuple-custom-instance-target.knot`.
- **Annotation *values* are never type-checked against their own derived
  expected type**, and relatedly, **`Sensitivity` stays an opaque,
  non-introspecting stub** (spec §9.6) — `annotation::table` can derive
  what type `@unravel`'s own value *should* have from the annotated
  binding's signature, but nothing calls that from `constrain::expr`'s
  `Annotated` handling to check a real value against it, so `@unravel(42)`
  on a binding whose derived `unravel` type is a multi-argument function
  is silently accepted. Fixing the first half doesn't by itself fix the
  second: `Sensitivity`'s own eventual recursive record/tuple expansion
  (`annotation::sensitivity::sensitivity_of`) is a separate, still-stubbed
  piece needed before an annotation value *involving* a real
  `Sensitivity`-shaped argument could be checked meaningfully.
- **No real cross-module resolution.** A qualified reference (`Map.get`,
  `List.map`, or anything else spelled `Module.name`) is trusted at face
  value — there's no project-wide module loader to confirm the named
  module really exports that name, or even really exists. In snippet mode
  (`canonicalize_decls`, what `corpus/programs`/`corpus/semantic` both run
  fixtures through) this is unconditional; in real module mode
  (`canonicalize_module`) a qualifier is at least checked against the
  current module's own `import` list, but nothing verifies what that
  import's target module *actually contains*.
- **Fixed. Re-declaring an instance a *builtin* type already has is now
  flagged as a duplicate** — `build_instance_table` (Task #40) now takes
  the seeded builtin table as its own `builtins` parameter, consulted
  alongside the module's own table-so-far in pass 1's duplicate check
  (`instance Eq Int where ...` now correctly reports `DuplicateInstance`
  rather than being silently accepted and then merged in by `merge_from`
  afterward with no diagnostic either way). Pass 2's own superclass check
  deliberately doesn't need the same treatment — every interface this
  crate currently seeds already seeds its own superclass alongside it, so
  a newly-accepted instance can never have a superclass obligation only
  the builtin table could satisfy. Moved from `known_gaps/` to
  `corpus/semantic/invalid/interfaces/redeclare-builtin-instance-is-a-
  duplicate.knot`.
- **Full dictionary-passing codegen doesn't exist** (TM7 above) — not a
  regression, since there is no codegen anywhere in this project yet for
  it to plug into, but worth naming precisely: `elaborate::
  elaborate_module` answers *which* instance resolves an obligation, not
  *how* to compile a polymorphic function to actually receive one at
  runtime.
- **Elaboration (TM7 Stage B) isn't wired into `check_module`, and never
  covers instance method bodies even when run directly** — `check_pending`
  alone is the correct, complete source of truth for whether a module
  type-checks (instance-method obligations *are* checked for existence);
  `elaborate_module` is a separate, narrower, currently-disconnected
  question ("what dictionary would this resolve to") that only ever
  walks ordinary top-level `LetMember`s.
