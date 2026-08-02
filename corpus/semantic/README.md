# `corpus/semantic/` — plan (2026-08-01)

Sibling to `corpus/syntax/` (the pre-existing corpus, moved there in this same
change — see its own fixtures for pure-grammar coverage). This tier exercises
`knot-canonical` + `knot-checker`: does a realistic, fully-resolvable program
get accepted/rejected correctly, all the way through canonicalization,
constraint generation, solving, and instance/obligation checking?

Fixtures/runner don't exist yet — that's §5's checklist. The prerequisite
entry point they need to run against, `check::check_module`, does now (Fix
#10, done as part of this same plan). This document is the plan: an
inventory of every semantic the checker implements, an interaction matrix of
which pairs are actually likely to break each other (grounded in this
session's own findings, not guessed), and a checklist of fixtures to write
against it.

## 0. Why this tier didn't exist before

`knot-type-checker-plan.md` §8 explicitly decided *against* reusing
`corpus/syntax/valid/` for this: those fixtures are deliberately small,
grammar-focused, and often use free/undeclared variables — fine for parsing,
meaningless to type-check. The plan instead relied on ~230 inline
`#[cfg(test)]` unit tests across `knot-canonical`/`knot-checker`, each
hand-building one isolated scenario.

That worked for testing mechanisms in isolation, but it's exactly why real
bugs kept surfacing only when someone (this session) manually combined two
features in one program: alias expansion + extensible records, ADTs +
constructor schemes, Collection/Context + user types. Unit tests, by
construction, don't combine features unless someone thinks to write that
specific combination by hand.

**A blocking prerequisite, not just "more tests": there was still no
`check_module` entry point** (`knot-checker/src/lib.rs`'s own doc comment
flagged this). `constrain_module`, `solve`, `build_instance_table`, and
`check_pending` were wired together by hand, differently, in every single
test — and critically, **nothing anywhere merged `prelude::seed`'s builtin
`InstanceTable` with a module's own `build_instance_table` result.** Every
existing test either used a bare `InstanceTable::new()` plus hand-picked
`insert_builtin` calls for exactly what that one test needed, or never
called `check_pending` at all. This is now fixed (`check::check_module`,
Fix #10) — see §2 and §5.

This was confirmed live while drafting this plan: a scratch test for the
utterly mundane `addX :: Float -> Float -> Float; addX a b = a + b` reported
a spurious `NoInstance("Num")` — not a real bug, but a self-inflicted one
from forgetting to merge the two tables by hand. That near-miss is the
argument for building the merge (ideally as part of a real `check_module`)
*before* writing more than a couple of semantic fixtures — otherwise every
fixture's own test harness risks silently asserting the wrong thing, the
same way this one almost did.

## 1. Semantics inventory

Grouped by area. Each has an ID (`S#`) used in the interaction matrix below.

**Core type shapes**
- `S1` Primitive literals/types: `Int`, `Float`, `String`, `Bool`, `Unit`
- `S2` Tuples (arity 2–3)
- `S3` Closed records (construct, update, field access)
- `S4` Extensible records / row polymorphism (`{ r | field : T }`)
- `S5` Type aliases (non-parametric, parametric, alias-of-alias, wrapping a
  record/extensible-record/tuple/ADT reference)
- `S6` ADTs (multi-variant, polymorphic params, self-recursive)
- `S7` Constructors as values & in patterns (arity checking, generalization
  over unused type params)
- `S8` Built-in generic containers: `List`, `Map`, `Option`, `Result`, `IO`

**Binding / control flow**
- `S9` Lambda / currying / partial application
- `S10` If/then/else
- `S11` Case + pattern matching (wildcard, var, literal, ctor, tuple, cons,
  as-patterns, nesting)
- `S12` Exhaustiveness/redundancy warnings (TM9 — warning-only, never blocks)
- `S13` `let` bindings: single/multi/nested, self-recursive, mutually
  recursive (SCC grouping), let-polymorphism
- `S14` Top-level bindings — same SCC/generalization machinery as `S13`, but
  installed into the global `SchemeEnv`
- `S15` `Do`-notation (desugars to `bind`/`pure`, needs a `Context` instance)
- `S16` Holes (`_` — lambda body, let-discard, record field, anonymous arg)
- `S17` Annotations (block/stacked/inline forms) — desugaring only; real
  "sensitivity" checking is still a stub, out of scope for this corpus tier

**Interfaces**
- `S18` Constrained signatures (`Ord a => ...`), given/rigid propagation
- `S19` Instance declarations for the 8 ordinary interfaces (`Eq`, `Ord`,
  `Show`, `Semigroup`, `Monoid`, `Num`, `Fractional`, `Integral`)
- `S20` Superclass requirements & coherence (`Ord` needs `Eq`, `Monoid` needs
  `Semigroup`, `Integral` needs `Num`+`Ord`; duplicate-instance rejection)
- `S21` Instance method body type-checking (against each interface's
  `MethodShape`)
- `S22` `Collection`/`Context` ctor-kinded interfaces (`map`/`foldl`/`foldr`/
  `filter`/`length`, `pure`/`bind`) and their own `CtorMethodShape` body
  checking
- `S23` Structural (non-table) `Eq`/`Ord`/`Show` for `Tuple`/`Record`/`Unit`,
  recursive over components
- `S24` Ambiguous-constraint / dangling-obligation classification at
  generalization boundaries (`StillAbstract` vs `NoInstance` vs
  `Structural`)
- `S25` Built-in instance wiring (`Num Int`/`Float`, `Integral Int`,
  `Fractional Float`, `Eq`/`Ord`/`Show` for primitives + containers)

**Modules / naming**
- `S26` Module header, imports (qualified, aliased, exposing subset/wildcard)
- `S27` Ambiguous-import / unknown-qualifier detection
- `S28` Prelude/builtin name resolution vs. local shadowing

**Elaboration**
- `S29` Stage A/B dictionary elaboration (obligation → `Concrete`/
  `Structural`/`StillAbstract`)

### Explicit non-goals (still worth a fixture, as a documented boundary)

- Cross-module alias/type resolution — not implemented; fixtures stay
  single-module.
- User-defined interfaces — closed set only, by design (spec §2.3).
- A type variable applied to an argument (`f a` where `f` is itself a
  variable) has no grammar support at all (`knot-syntax`'s own
  `type_variable_takes_no_arguments` test proves `a b` parses as `a` and
  leftover ` b`). Consequence: a user's *own* signature can never be
  generically constrained by `Collection`/`Context` — those interfaces are
  reachable only through the built-in `map`/`foldl`/... functions or a
  concrete instance's own method bodies, never abstracted over further by
  user code. Worth one fixture pinning this down as intentional so a future
  grammar change doesn't silently alter it.
- Dictionary-*parameter* codegen for polymorphic bindings — existence
  checking only (`S24`'s `StillAbstract`), no runtime dictionary passing.

## 2. Confirmed findings so far (this session)

Already fixed (see git log / `knot-checker/src/lib.rs`'s own audit trail):
user-defined ADT constructor schemes, type alias expansion (incl. cycles),
Stage B structural-obligation misclassification, `Collection`/`Context`
instance method checking, `knot-canonical::BUILTIN_INTERFACES` missing
`Collection`/`Context`, and extensible-record alias expansion against a
concrete argument.

**Also newly found while drafting this plan — all three now fixed** (see
`knot-checker/src/lib.rs`'s own audit trail, Fixes #8-#10):

1. **Fixed (Fix #8).** Instance superclass coherence used to be
   declaration-order-dependent within one module — `instance Ord Shape
   where ...` followed later in the *same file* by `instance Eq Shape
   where ...` wrongly raised `MissingSuperclassInstance { interface: "Ord",
   superclass: "Eq" }`. `build_instance_table` now runs two passes:
   register every instance's own existence first, then check every
   accepted instance's superclasses against the now-fully-populated table.
2. **Fixed (Fix #9).** A user instance for a non-`Eq`/`Ord`/`Show`
   interface targeting a `Record`/`Tuple`/`Unit`-shaped type used to be
   silently unreachable — `instance Semigroup Point where ...` (`Point` a
   record type alias) parsed, canonicalized, and even type-checked its own
   method body, but `head_ref` returned `None` for a non-`Named` target, so
   `build_instance_table` just skipped inserting it with **no error
   anywhere**. Now reports `TypeErrorKind::InstanceTargetNotNominal` at the
   declaration site.
3. **Fixed (Fix #10).** No code path merged `prelude::seed`'s builtin
   `InstanceTable` with a module's own declared one — see §0 above. Closed
   by the new `check::check_module` entry point, which does the merge
   internally (`InstanceTable::merge_from`) before running `check_pending`.
   One narrow gap remains, inherited rather than newly introduced: a user
   re-declaring an instance a *builtin* type already has (`instance Eq Int
   where ...`) isn't flagged as a duplicate, since coherence-checking
   happens before the builtin table is merged in. Worth its own fixture
   (see the checklist below) documenting this as a known limitation rather
   than leaving it ambiguous.

## 3. Interaction matrix

Only pairs with a concrete, architecture-grounded reason to interact badly —
not the full combinatorial set. "Risk" explains *why*; "Fixture" is the
planned corpus program (✅ = confirmed bug exists today, needs a fix before
the fixture can be a `valid/` case; otherwise the fixture is expected to
pass once the merge in §0 lands).

| # | Feature A | Feature B | Risk | Planned fixture |
|---|-----------|-----------|------|------------------|
| 1 | `S5` alias | `S4` extensible record | Alias substitution must merge concrete fields into an open row, not just rename. | ✅ fixed this session — `Selectable Foo`-style fixture, both directions (concrete arg, forwarded-still-open arg). |
| 2 | `S5` alias | `S6` ADT variant fields | Alias expansion must rewrite *inside* a variant's own field types, not just top-level signatures. | Already unit-tested (`an_adts_own_variant_field_type_expands_an_alias_too`); promote to a full pipeline fixture combined with pattern matching (`S11`) on the resulting constructor. |
| 3 | `S5` alias | `S19` instance target | An alias used as `instance Eq SomeAlias where` must expand before instance-table keying. | Already unit-tested for a tuple alias; add a *record* alias case — this is where finding #2 above lives, so pair with a `Semigroup`/non-Eq-Ord-Show interface to hit it. |
| 4 | `S19` instance decl | `S20` superclass order | Declaring a subclass instance before its own superclass, same file. | ✅ confirmed bug (finding #1) — needs the fix first, then both orderings become `valid/` fixtures. |
| 5 | `S7` constructor (polymorphic) | `S14` top-level generalization | A polymorphic constructor (`Box a`) must be usable at two different concrete types from two different top-level bindings in the same module. | Unit-tested in isolation already; add a fixture combining it with an interface use (`Eq`) at one of the two instantiations. |
| 6 | `S6` ADT | `S19`/`S21` instance + method body | User declares `Eq`/`Ord` on their own recursive ADT, method body pattern-matches (`S11`) and recurses into sub-fields. | New — no existing test recurses an instance method body into `case` on the same ADT it's instancing. |
| 7 | `S22` Collection/Context | `S6` user ADT + `S18` given constraints | Confirmed *not currently expressible*: a user's own signature can't be generic over "any Collection" (see §1's non-goals) since `f a` isn't parseable. Fixture should pin down what *is* possible: calling `map`/`foldl` directly against a concrete user `Collection` instance from inside an ordinary function. | New. |
| 8 | `S15` Do-notation | `S22` Context, user-defined | `do` blocks desugar to `bind`/`pure` — confirm this works against a *user*-declared `Context` instance, not just built-in `IO`/`List`/etc. | New — existing `Do` tests likely only exercise built-ins. |
| 9 | `S13`/`S14` mutually recursive group | `S18` constrained signature | One member of a mutually-recursive SCC has a signature with a constraint (`Ord a =>`), another doesn't — confirm the `given` fact is visible to both without leaking to unrelated bindings. | New. |
| 10 | `S24` ambiguous/dangling obligation | `S14` top-level generalization | A polymorphic top-level binding whose body's obligation is never resolved *in that module* — confirm `StillAbstract`, not a spurious `NoInstance`, and confirm it interacts correctly with a *second* binding that *does* use it concretely (so one binding's obligation resolves concretely while a sibling's stays abstract). | Partially unit-tested (`elaborate.rs`); no full-pipeline version. |
| 11 | `S23` structural Eq/Ord/Show | `S2`/`S3` nested Tuple-in-Record-in-Tuple | Structural recursion must actually recurse through mixed nesting, not just one level. | New. |
| 12 | `S23` structural interfaces | `S20` non-Eq/Ord/Show interface on same shape | Confirmed bug (finding #2) — `Semigroup`/`Monoid`/`Num` on a `Record`/`Tuple` target is silently unreachable, unlike `Eq`/`Ord`/`Show` which fall back to the hardcoded structural rule. | ✅ needs the diagnostic fix first. |
| 13 | `S26` imports/qualified names | `S5` alias | A locally-shadowed name vs. a qualified reference to the same name from an import — confirm no accidental alias-expansion or resolution mixup. | New — imports are barely exercised past `knot-syntax`'s own corpus. |
| 14 | `S16` holes | `S13` let + `S11` case | A hole inside a `let`-bound pattern discard, inside a `case` arm, inside a lambda — confirm holes don't interfere with SCC grouping or generalization. | New. |
| 15 | `S9` currying/partial application | `S19` instance method (operator) | Partially-applied operator method (e.g. passing `(==)`-shaped section, if/when supported) against a declared instance. | New — check current operator-as-value support first; likely still N/A per `knot-canonical::prelude`'s own doc comment on symbolic operators never being expression-referenceable. |
| 16 | `S17` annotations | `S13`/`S14` bindings | Annotation desugaring interacting with SCC/let-grouping — confirm an annotated binding's own group membership is unaffected. | New. |

## 4. Proposed layout

Mirrors `corpus/syntax/`'s own `valid/`/`invalid/` split, but organized by
*interaction* rather than by single grammar feature, since that's the point
of this tier:

```
corpus/semantic/
  README.md              (this file)
  valid/
    aliases/              (rows 1-3)
    interfaces/           (rows 4-6, 9-12, 15)
    collections/           (rows 7-8)
    generalization/        (rows 5, 10)
    structural/            (row 11)
    modules/               (row 13)
    misc/                  (rows 14, 16)
  invalid/
    interfaces/           (row 4 wrong-order case, once fixed — expected
                            rejection would be a genuinely-missing
                            superclass, not an ordering artifact)
    aliases/              (extending a non-record type, conflicting fields —
                            already unit-tested in knot-canonical, worth a
                            pipeline-level fixture too)
```

Each fixture: a small realistic `.knot` program plus (for `valid/`) an
assertion that `check_module` accepts it with zero errors, and (for
`invalid/`) an assertion of *which* error kind is expected — not just
pass/fail, mirroring `corpus/syntax/invalid/`'s convention of a leading
comment stating the expected failure reason.

## 5. Checklist

**Prerequisite (blocks everything below) — done:**
- [x] Merge `prelude::seed`'s `InstanceTable` with a module's own
      `build_instance_table` result — `check::check_module(decls: &[Spanned
      <CDecl>]) -> Vec<TypeError>` (`knot-checker/src/check.rs`, Fix #10).
      Deliberately doesn't also run `elaborate::elaborate_module` — see its
      own doc comment on the double-counting/incompleteness reasons why.
- [x] Fix finding #1 (superclass order-dependence) — two-pass
      `build_instance_table` (Fix #8).
- [x] Fix or diagnose finding #2 (silent non-Eq/Ord/Show structural
      instance) — `TypeErrorKind::InstanceTargetNotNominal` at the
      `instance` declaration site (Fix #9).

**Harness (not yet built):**
- [ ] `knot-checker/tests/corpus.rs`, mirroring `knot-syntax`'s own —
      walks `corpus/semantic/valid|invalid`, canonicalizes each fixture
      (`knot_canonical::canonicalize_decls`) then runs `check::check_module`,
      asserts accept/reject + (for `invalid/`) the expected error kind via a
      leading fixture comment. A canonicalization failure on a `valid/`
      fixture should fail the test the same way a type error would — the
      corpus is about the whole pipeline, not just `check_module` alone.

**Fixtures** (one `.knot` file per interaction-matrix row, `valid/` unless
noted `invalid/`):
- [ ] Row 1 — alias × extensible record (both directions)
- [ ] Row 2 — alias × ADT variant × pattern match
- [ ] Row 3 — alias × instance target (record + tuple)
- [ ] Row 4 — superclass declared before/after, same module (one `valid/`
      pair once fixed; keep one genuinely-missing-superclass `invalid/`
      case for regression)
- [ ] Row 5 — polymorphic constructor × top-level generalization × Eq use
- [ ] Row 6 — recursive ADT × Eq/Ord instance × recursive pattern match
- [ ] Row 7 — Collection instance × concrete use from an ordinary function
- [ ] Row 8 — Do-notation × user-defined Context instance
- [ ] Row 9 — mutually recursive group × mixed constrained/unconstrained
      signatures
- [ ] Row 10 — StillAbstract × sibling concrete use in the same module
      (needs `elaborate::elaborate_module`, not `check_module` alone —
      `check_module` deliberately doesn't classify obligations this way,
      see Fix #10's own doc comment)
- [ ] Row 11 — nested Tuple/Record structural Eq/Ord/Show
- [ ] Row 12 — non-Eq/Ord/Show interface on Record/Tuple: `invalid/`,
      asserting `TypeErrorKind::InstanceTargetNotNominal` (Fix #9)
- [ ] Row 13 — qualified import × alias interaction
- [ ] Row 14 — holes × let × case nesting
- [ ] Row 15 — confirm operator-as-value is still N/A (pin down as a
      documented non-goal fixture, or skip if genuinely inapplicable)
- [ ] Row 16 — annotations × SCC grouping
- [ ] Row 17 (new) — re-declaring an instance a *builtin* type already has
      (`instance Eq Int where ...`) — `invalid/` once/if this gets its own
      diagnostic, otherwise a `valid/` fixture documenting the known gap
      Fix #10's own doc comment names (silently accepted today, not flagged
      as a duplicate)
