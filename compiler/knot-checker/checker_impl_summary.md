# Knot Type Checker (`knot-checker`) — Implementation Summary

Covers what's actually built, as of this pass: all of TM0–TM9 from
`knot-type-checker-plan.md`. 130 tests in `knot-checker` alone (347 across
the whole `compiler/` workspace), `cargo clippy`/`cargo fmt` clean. This is
a summary of *the implementation*, not a plan — see `knot-type-checker-plan.md`
for the original design discussion and `language-spec-notes.md` for the
language being checked.

**There is still no public `check_module` entry point.** Everything through
TM9 works and is tested at the level each milestone operates on, but the
last mile — walking a whole `CModule` end to end and handing back a
finished, elaborated result — needs the `ast.rs` tree-walk gap closed first
(see TM7 below). Until then, the way to actually exercise the checker is
the pattern every test in the crate already uses: `constrain::decl::
constrain_module` → `solve::solve` → `interface::instance::check_pending`,
seeded with `prelude::seed`.

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
(not yet built) TExpr — an Elaborated AST with a type on every node
```

`knot-checker`'s own internal pipeline is **generate constraints, then
solve them separately** — the single biggest architectural choice, borrowed
deliberately from Elm's compiler (plan §2):

```
CExpr/CPattern
   │  constrain::{expr, pattern, decl}   (TM3/TM4)
   ▼
Constraint tree   (nothing checked yet — just "here's what must hold")
   │  solve::solve                        (TM5)
   ▼
Substitution (mutated) + SchemeEnv + Vec<PendingInstance> + Vec<TypeError>
   │  interface::instance::check_pending  (TM6)
   ▼
Vec<TypeError>  (fully resolved, including instance-table lookups)
```

---

## TM0 — `var.rs`: the arena substitution

A `Substitution` is an arena (`Vec<Slot>`); a `TypeVarId` is an index into
it. Each slot is `Unbound` (a genuine unknown), `Link` (a union-find
redirect), `Bound(Structure)` (resolved to a concrete shape), or `Rigid`
(a signature's own type variable — see TM4).

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

`Sensitivity` is stubbed here as an ordinary opaque one-argument type
(`Structure::App`, same treatment as `Option`) — per this session's
explicit decision, no recursive expansion into record/tuple shape yet
(spec §9.6 is still just a stub; see `annotation::sensitivity`).

## TM3 — `constrain::{expr, pattern}`: constraint generation

Each function is bottom-up: constrain the children, get their `TypeVarId`s
back, combine them via `Constraint::Equal`/`HasInstance`. A few
illustrative cases:

- A literal *is* its builtin type directly — no constraint needed.
- `App(f, arg)`: constrain `f`/`arg`, invent a fresh result variable, push
  one constraint — `f`'s type must equal `arg's type -> result`.
- `Var(Ref::Local(x))`: no constraint at all — look `x` up in `LocalScope`
  (a stack of name→type maps for lambda/case bindings, which are never
  generalized) and return that type directly.
- `Var(Ref::TopLevel/Builtin/Imported(...))`: push a `Constraint::Lookup` —
  "look this name's scheme up later, instantiate it fresh, unify the
  result against this variable." This is the "`CLocal`-equivalent" the
  plan names — the mechanism that makes forward/mutual references and real
  let-polymorphism possible at all, since a top-level binding's principal
  type isn't known until its own group finishes solving.
- `BinOp`/`Negate`: a small, closed operator → interface table (`Add` needs
  `Num` and unifies both operands; `Pow` is `(Num a, Integral b) => a -> b
  -> a`, the one operator needing two *different* interfaces on two
  different variables — this resolved a real spec gap, since `^` had no
  documented signature before this session).
- `Let`: delegates to `constrain::decl` (see TM4) — this is where the
  `todo!()` from earlier drafts got filled in.

**Not implemented**: `Do` (needs the Context interface's `pure`/`bind`,
spec §6.4 — entangled with the same higher-kinded-polymorphism gap TM8
hits). `Annotated` is handled, but deliberately shallow: annotation
*values* aren't constrained here at all yet (TM6's `annotation::table`
only derives what type a value *would* need, nothing checks a real one
against it).

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
become `Substitution::fresh_rigid` slots (built in TM0/TM1, unused until
now); its params/body are constrained directly against the signature's own
shape, so `unify`'s rigid-vs-concrete rejection actually gets exercised for
the first time.

**A real asymmetry, not a bug**: `knot-canonical`'s `Ref` has no separate
case for "`let`-bound name" — `Ref::Local` covers `let` the same as
lambda/case/do params (a name-*resolution* concern, indifferent to
polymorphism). So `Constraint::Let` carries a `top_level: bool`: a
module-level group gets fully generalized and installed into the global
scheme environment; a `let`-expression's group does not — its names stay
monomorphic for the lifetime of that `let` block, a documented, sound (just
occasionally more conservative than ML/Haskell) simplification, since
`Ref::Local` always resolves immediately regardless.

**Two real bugs found here** (both via the flagship "`identity` used at two
different types" end-to-end test — see TM5):

1. A group's own scope frame originally stayed open through the *entire*
   rest of the binding chain (needed for `let`, per the paragraph above).
   For top-level groups this let a *later, unrelated* group's code resolve
   an *earlier* one's name via raw `LocalScope` instead of the deferred
   `Lookup`+instantiate path — silently defeating polymorphism for it.
   Fixed: the frame now closes immediately after a top-level group's own
   bodies are constrained, before recursing into what comes next.
2. (See TM5 — the companion bug in `generalize` itself.)

## TM5 — `solve.rs`: generalize, instantiate, real let-polymorphism

The driver that actually calls `unify`. Walks the `Constraint` tree:

- `Equal` → call `unify` directly.
- `HasInstance` → **don't** classify it yet — just collect it into a
  `pending` list. A `HasInstance` obligation often can't be judged the
  moment it's seen (the type it's about might still be an unresolved
  variable that a *later* `Equal` in the same list pins down). Only after
  the *whole* tree's `Equal`s have run does `solve` go back and classify
  each one: rigid (checked against `given` facts — no instance table
  needed, since a rigid variable can never have more permissions than its
  own signature granted) or concrete (left in the returned list for TM6).
- `Lookup` → instantiate whatever scheme is currently installed (fresh
  copy of each quantified variable, deep-copying the stored structure),
  unify the fresh copy against the reference's own type.
- `Let` → push the group's `header_ty`s onto an `ambient` stack (so no
  *inner* `generalize` mistakes an outer, still-in-progress binding's type
  variable for one it owns), solve `header_con`, then for each *top-level*
  member: a signed one's scheme is just its signature restated; an
  unsigned one needs the real **`generalize`** — textbook `ftv(ty) \
  ftv(ambient)`, gathering any of its own now-pending obligations into the
  new scheme's `constraints` — then the **ambiguous-CAF check** (this
  session's arity-based replacement for Haskell's monomorphism
  restriction: a zero-argument binding generalized over a variable that
  still carries an interface obligation is an error, full stop; arity ≥ 1
  is never restricted).

**The companion bug to TM4's**: `ambient` originally still contained the
*very member being generalized* (pushed right before solving its own
`header_con`, never popped before `generalize` ran). `ftv(ty) \
ftv(ambient)` then always cancelled to empty — no top-level binding was
ever actually generalized, silently. Fixed by popping a group's own
`header_ty`s immediately after solving `header_con`, *before* generalizing
them. Both bugs were caught by the same test — `identity x = x;
useIdentity = (identity 1, identity True)` — which is exactly why that
test is worth having: it's the simplest program that only type-checks
correctly if let-polymorphism (both the generation-time scoping and the
solve-time generalization) actually works, not just looks like it does.

## TM6 — `interface::{table, instance}`, `annotation::{table, sensitivity}`

**`interface::table`**: the closed interface set (`Eq`, `Ord`, `Show`,
`Semigroup`, `Monoid`, `Num`, `Fractional`, `Integral`) and their
superclasses (`Ord`→`Eq`, `Monoid`→`Semigroup`, `Integral`→`{Num, Ord}`,
`Fractional`→`Num`) — a small hardcoded table, since the set can never grow.

**`interface::instance`**: `InstanceTable`, built from a module's own
`instance` declarations, checking **coherence** (at most one instance per
`(interface, head type)` pair) and **superclass existence** (`instance Ord
Shape` needs `Eq Shape` to already exist). `check_pending` resolves
`solve::solve`'s leftover `PendingInstance`s against it.

Two things explicitly *not* handled, both documented rather than silently
wrong: only `Structure::App`-headed obligations are checked at all (a
`Tuple`/`Record`-headed one is neither confirmed nor rejected — extending
`Eq`/`Ord`/`Show` to structural types needs a different, shape-based
lookup); and a *parametric* instance's own constraints (`instance Eq a =>
Eq (List a)`'s own `Eq a` requirement) aren't checked recursively — that's
real dictionary-construction work, correctly left to TM7.

**`annotation::table`**: derives an annotation key's expected type — fixed
shapes for `nodeId`/`position`/`label`/`doc`/`color`/`group`/`collapsed`,
and the real derivation rule for `unravel`: `Sensitivity Out -> UnravelInput
A -> UnravelInput B -> ... -> Option (A, B, C)`, collapsing to a bare
`Option A` for one parameter. `annotation::sensitivity::sensitivity_of` is
the single seam this calls through for `Sensitivity`'s own (currently
opaque) expansion — upgrading it later to the real record/tuple-recursive
behavior touches only that one function. Nothing yet checks a real
annotation value against the derived type (that's still `constrain::expr`'s
`Annotated` gap).

## TM7 — `ast.rs`/`elaborate.rs`: Elaborated AST, scoped down honestly

The plan's own framing calls this "the crate's actual deliverable" — a
full `CExpr` → `TExpr` tree walk producing a fully-typed AST with explicit
dictionaries at every constrained call site.

**What's real here**: `ast.rs` defines the target shape (`Dictionary`,
`ElaboratedRef`). `elaborate::resolve_dictionary`/`resolve_pending` fully
implement and test the part that's tractable in isolation — given a
concrete `HasInstance` obligation and a solved substitution + instance
table, determine exactly which instance answers it.

**What's deliberately not attempted**: a complete tree walk. `constrain::
expr`/`pattern` only ever return a bare `TypeVarId` per node — the
`Constraint` list they build is a flat `Vec`, not shaped like the original
`CExpr` tree, so there's no way to recover *which* `TypeVarId` belonged to
*which* node after the fact. Doing this for real needs those functions
retrofitted to return `(TypeVarId, TExpr)` pairs instead. That's
concrete, well-understood follow-up work — not attempted here as a
shortcut, because a version that merely *looked* like full elaboration but
silently correlated the wrong node to the wrong type would be worse than
not having one.

## TM8 — `prelude.rs`: built-in instances and schemes

Seeds a real `SchemeEnv`/`InstanceTable`: `Num Int`/`Num Float`, `Integral
Int`, `Fractional Float`; `Eq`/`Ord`/`Show` for `Int`/`Float`/`String`/
`Bool`/`Unit` and for `List`/`Option`/`Result` heads; schemes for
`compare`, `show`, `negate`/`abs`/`signum`, `recip`, `div`/`mod`,
`fromIntegral`, `empty`, `not`, and every built-in constructor.

**A genuine structural gap, not an oversight**: `map`, `foldl`, `foldr`,
`filter`, `length`, `pure`, `bind` are *not* seeded. Their signatures
(`map :: (a -> b) -> f a -> f b`) are polymorphic over `f` itself — a type
*constructor*, not an ordinary type — and `Structure::App(Ref, Vec
<TypeVarId>)` has no way to represent "some type constructor, not yet
known" as a variable, only a concrete `Ref` head. Giving these a correct
signature needs a real design decision (a new `Structure` variant for a
higher-kinded variable, plus updating everywhere that pattern-matches
`Structure`) — out of scope to invent as a side effect of seeding the
prelude, so it's flagged instead of worked around with an incorrect
concrete-`f` stand-in.

One test worth calling out: `f n = 1.0 + fromIntegral n` correctly
generalizes `f` to `Integral a => a -> Float`, *not* `Int -> Float` — `n`'s
type stays a free, `Integral`-constrained variable because this design has
no Haskell-style numeric-literal defaulting (a much earlier decision this
session). `Int` just happens to be the only seeded `Integral` instance;
the type system itself doesn't know that, by design — interfaces stay open.

## TM9 — `exhaustiveness.rs`: pattern-match usefulness checking (stretch)

Explicitly lower priority in the plan (spec only ever wants a warning
here), but implemented for real: Maranget's usefulness-checking algorithm,
the same one Elm's `Nitpick.PatternMatches` uses. A `CtorTable` tracks each
constructor's sibling set (built-in enums seeded, user `type` declarations
added per-module); `is_useful` recursively specializes a pattern matrix by
constructor, falling back to a "default matrix" (rows headed by a
wildcard, that column dropped) when the constructors seen don't exhaust
the type — or when they can't in principle (`Int`/`String` literals have
no enumerable complete set, matching Elm). `is_exhaustive` and
`redundant_arms` are both just different questions asked of the same
`is_useful` core.

List's `Cons`/`Nil` patterns are handled as their own two-constructor
pseudo-type directly (they're a distinct `CPattern` shape, not
`CPattern::Ctor`, so they don't go through `CtorTable` at all); tuples and
`Unit` are single-shape types, always trivially "complete."

Reports a boolean exhaustive/not and which arm indices are redundant, not
a constructed counter-example (`"missing: Circle _"`) — witness synthesis
is a further, well-understood extension of the same algorithm, left out to
keep this stretch milestone bounded. Entirely self-contained: nothing else
in the pipeline calls into it, since a warning-only pass doesn't need to be
wired into anything.

---

## Known gaps, all documented in place, none silently papered over

- **`Sensitivity` is an opaque stub** (spec §9.6's recursive record/tuple
  expansion isn't built) — single seam: `annotation::sensitivity::
  sensitivity_of`.
- **`let`-bound names don't get let-polymorphism** — a documented,
  sound simplification forced by `Ref::Local` covering both `let` and
  truly-monomorphic bindings with no way to tell them apart.
- **Higher-kinded polymorphism doesn't exist** — blocks `map`/`foldl`/
  `foldr`/`filter`/`length`/`pure`/`bind` from having any signature at all
  (TM8), and blocks `Do`-notation (TM3).
- **No full elaboration tree walk** — `ast.rs`/`elaborate.rs` give the
  target shape and a working dictionary-resolution primitive, not a
  complete `CExpr -> TExpr` pass (TM7).
- **Structural (`Tuple`/`Record`) and parametric instance checking** aren't
  implemented — `interface::instance` only confirms a concrete, named
  type's own head has an instance (TM6).
- **Annotation *values* are never type-checked** — `annotation::table`
  derives the expected type; nothing calls it from `constrain::expr` yet.
