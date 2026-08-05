# `corpus/programs/` — realistic whole-program examples (outcome-agnostic)

Sibling to `corpus/syntax/` and `corpus/semantic/`, but a different kind of
tier: those two both require knowing the expected outcome *before* writing
the fixture (`valid/` asserts zero errors, `invalid/` asserts one specific
error kind). This tier is the opposite — fixtures are written the way an
actual person using the language would write a real small program, without
first checking whether the checker happens to accept it. The point is
finding gaps that a systematic, feature-by-feature or interaction-matrix
pass doesn't surface, because real programs combine things in ways nobody
enumerates in advance.

**Workflow** (repeated as an ongoing cycle, not a one-time pass):
1. Write a batch of realistic programs across varied themes — don't filter
   for ones you already expect to pass.
2. Run `cargo run --example corpus_report -p knot-checker` (walks this
   whole tree, reports parse/canon/type-check pass or fail per file, plus a
   summary) and fix any *parse* errors in the fixtures themselves first
   (typos, not findings).
3. Commit the fixtures once they're syntactically clean.
4. Run the report again — *this* run's failures are the actual findings.
   Categorize each as a real bug (fix it) or a documented non-goal/known
   gap (note it and move on).
5. Once some fixes land, a fixture that used to fail either starts passing
   (leave it — it's now a real regression-test asset) or still fails for a
   *different*, now-more-specific reason (update expectations, keep going).

No `-- expect:`/`-- error-kind:` convention here — the report tool prints
whatever actually happened, it doesn't assert against a pre-declared
expectation. Once a specific finding from this tier is understood and
fixed, promoting the fixture (or a trimmed-down version of it) into
`corpus/semantic/valid` or `.../invalid` with a real assertion is the
natural way to lock in the fix as a permanent regression test — this tier
is where things are *discovered*, `corpus/semantic/` is where they're
*pinned down*.

## `known_gaps/` — a deliberate exception to "outcome-agnostic"

Every fixture in this one subdirectory *does* have a known, declared
expected outcome, unlike the rest of this tier — each one exists
specifically to pin down one already-understood, already-documented gap
from `knot-checker/checker_impl_summary.md`'s own "Known gaps" section,
not to discover something new. Every file starts with a `-- KNOWN GAP, not
a discovery:` comment explaining exactly which gap, why the current
behavior is what it is, and what the *correct* outcome would look like
once that gap is eventually closed. When `cargo run --example
corpus_report` reports one of these as a failure, that's expected and
correct (the checker rejecting something `checker_impl_summary.md` says
should currently still be rejected); when it reports one as `OK`, that
does *not* mean the fixture is fine — several of these are deliberately
*silently-wrong-acceptance* gaps, so `OK` there means "confirmed still
silently accepting something it shouldn't," not "passing." If a fixture
in this directory ever *changes* which of these it reports, that's a
signal the underlying gap moved (fixed, or newly broken worse) — update
`checker_impl_summary.md`'s own "Known gaps" section and this fixture's
own comment together, in the same change, rather than letting them drift
apart. As of this writing: 1 fixture, covering annotation values never
being type-checked against their own derived expected type. Five have
graduated out of this directory once fixed: exhaustiveness checking never
being wired into `check_module` (see `patterns/non-exhaustive-case-
warns.knot`); a record instance's own declared context never being
enforced against a real argument type — a real, demonstrated unsoundness,
not just an incompleteness (see `corpus/semantic/invalid/interfaces/
record-instance-context-not-satisfied.knot` and its own `valid/`
companion); the record-instance table's field-name-only (not
field-type-aware) keying together with custom instances still not being
able to target a `Tuple` — both fixed together via a new type-aware
`CanonicalType` keying scheme (see `corpus/semantic/valid/interfaces/
record-instance-keyed-by-type-not-just-name.knot` and `.../tuple-custom-
instance-target.knot`); and re-declaring an instance a builtin type
already has not being flagged as a `DuplicateInstance` (see
`corpus/semantic/invalid/interfaces/redeclare-builtin-instance-is-a-
duplicate.knot`). `checker_impl_summary.md`'s own "Known gaps" section has
the rest of every graduation story.

## Findings log

Kept here so the iteration history doesn't get lost across rounds. See
`knot-checker/src/lib.rs`'s own audit trail for the fixes once they land.

### Round 1 (2026-08-02)

Found by hand before this tier existed (see `lib.rs`'s own Fix #8-#10 and
the exchange that preceded this round), already fixed: superclass ordering,
`InstanceTargetNotNominal`, missing `check_module` entry point.

Found via a first batch of realistic-program probing (this round's own 43
fixtures, `cargo run --example corpus_report -p knot-checker`) — 16 of 43
fixtures failed initially, tracing back to 7 distinct root causes. After
fixing #1 and #4 below (`lib.rs`'s own Fix #11/#12), re-running dropped
that to 6 of 43 failing — the other 10 fixtures now pass cleanly with no
other changes needed, confirming both fixes landed exactly where expected.

1. **Fixed (Fix #11). `Semigroup`/`Monoid` had zero builtin instances
   anywhere** — `<>` and `empty` failed on every builtin type, including
   `String`. Broke the most basic string-building pattern outright. Hit
   `text/*`, `errors/error-mapping.knot`, `interfaces/shape-show.knot`,
   `data_structures/json-value.knot`, `options/dict-lookup-fallback.knot`
   (8 files) — all now pass.
2. **Fixed. Integer literals never unified with `Float`** — `x :: Float;
   x = 5` was a hard `Unify::Mismatch`. No numeric-literal polymorphism at
   all. **Fix**: `constrain::expr`'s own `CExpr::IntLit` handling gives an
   int literal a fresh, `Num`-obligated variable now (`Num a => a`,
   Haskell's own numeric-literal polymorphism, simplified) instead of
   hard-wiring `Int` — it unifies with whatever the context demands
   (`Float`, a signature, a sibling operand, even a custom `Num` instance
   like `interfaces/vector2-custom-num.knot`'s own `Vector2`) via ordinary
   unification. **Defaulting, in two places, both in `solve.rs`**: nothing
   else in this closed language can produce a bare, non-function,
   `Num`-polymorphic *value* except this one code path (every other route
   either applies a function -- resolving the type via unification with a
   concrete argument or signature -- or is itself function-typed, already
   exempt from `check_ambiguous`'s own zero-arg restriction), so defaulting
   an unresolved `"Num"` obligation to `Int` can never misfire on a real
   user-written constraint like `Ord a =>`. `generalize` defaults one right
   before it would otherwise become part of some binding's own generalized
   scheme (so `x = 5`, no signature, keeps working exactly as before,
   rather than tripping `AmbiguousConstraint` the moment literals stopped
   being hard-wired to `Int`); `solve_with_obligations`'s own final sweep
   catches the other shape -- an obligation that never becomes part of
   *any* scheme's own quantified variables at all (`f x y = x == y; result
   = f 1 2`: the literals' shared variable unifies with `f`'s own `a`, but
   `result`'s own inferred type is just `Bool`, so nothing ever quantifies
   over it) -- which would otherwise silently stay `StillAbstract` forever.
3. **Fixed. `Collection`/`Context` were broken for both of their
   2-parameter built-ins** — the `VarApp` mechanism only ever constructed a
   `Ctor` with zero leading arguments and required an *exact* arity match
   against the concrete `App` it met, so it silently couldn't handle a
   constructor that takes two parameters where only the last one varies.
   Hit `do`-notation over `Result` in both `errors/parse-pipeline.knot` and
   `errors/validate-user.knot` — both now pass. **Fix**: `ty::Structure::
   Ctor(Ref)` became `Ctor(Ref, Vec<TypeVarId>)`, carrying whichever
   *leading* arguments are already fixed (empty for a 1-parameter
   constructor like `List`/`Maybe` — the same shape as before this fix;
   one element, `e`, for `Result e a`/`k` for `Map k v`). `unify.rs`'s own
   `VarApp`-vs-`App` case now splits the concrete side's own argument list
   at `own_len - VarApp's own arg count` instead of demanding they already
   match: the leading remainder becomes the `Ctor`'s own partially-applied
   arguments, the trailing remainder (always exactly as many as the
   `VarApp` itself has) unifies pairwise as before. `check_instance`'s own
   `Ctor` lookup is unaffected (a `Collection`/`Context` instance is
   registered by name alone, regardless of how many arguments happen to
   already be applied) — this was purely a unification-arity bug, not an
   instance-resolution one, and doesn't touch this language's own,
   separate, still-standing decision to support no *user-defined*
   higher-kinded signatures at all (`ty::Structure::VarApp`'s own doc
   comment) — `VarApp` remains something only this crate's own hand-built
   `Collection`/`Context` prelude schemes ever produce.
4. **Fixed (Fix #12). `given` facts were never closed over superclass
   relationships** — a signature's own `Ord a =>` only ever inserted
   `"Ord"` into `a`'s `given` set, never the implied `"Eq"`, even though
   every real `Ord` instance is *guaranteed* to have a matching `Eq` one
   (superclass coherence enforces this at declaration time). Same story
   for `Monoid` never implying `Semigroup`. Hit
   `data_structures/binary-search-tree.knot` (`contains` uses `==` under
   `Ord a =>`) and `interfaces/combine-all.knot` (`<>` under
   `Monoid a =>`) — both now pass. Fixed via
   `solve::insert_given_with_superclasses`, reusing the existing
   `interface::table::superclasses`.
5. **Fixed. `InstanceTargetNotNominal` (this session's own Fix #9) blocked
   legitimate custom `Eq`/`Ord`/`Show` instances on records, not just the
   interfaces with no structural fallback** — `interfaces/point-ord.knot`
   wants `Ord` on a `Point` record *by magnitude*, a real, common pattern
   (order-by-derived-key) the automatic structural per-field derivation
   could never produce on its own. Fix #9's own reasoning ("no structural
   fallback exists, so an explicit instance is unreachable dead code") was
   right for `Semigroup`/`Monoid`/etc. but wrong for `Eq`/`Ord`/`Show`
   specifically, where a real fallback exists that an explicit instance
   should be allowed to *override*, not just duplicate. A likely
   overcorrection in that fix, found by using the feature it changed.
   **Broader than first documented (round 3, `interfaces/vector2-custom-
   num.knot`)**: `head_ref` rejected *any* record target regardless of
   interface, not just `Eq`/`Ord`/`Show` — a hand-declared `instance Num
   Vector2` (operator-overloaded 2D vector math, `Num` having no
   structural fallback at all) was blocked exactly the same way, then
   every use of `+`/`-` on a `Vector2` additionally reported its own
   `NoInstance("Num")` once the instance itself never made it into the
   table. Same root cause, wider blast radius than "Eq/Ord/Show on
   records" alone suggested -- and, as it turned out, this second half is
   the more important one: `Num`/`Semigroup`/anything user-defined having
   *no* structural fallback at all is exactly the case a custom record
   instance matters most for. **Fix**: `type alias` expansion
   (`knot-canonical::resolve::alias`) erases every alias reference to its
   literal underlying `CType` before this table ever runs, so by
   instance-declaration time there's no name left to key a `Ref` by at all
   -- `InstanceTable` now keeps a second, parallel key space
   (`record_entries`, keyed by `RecordKey`: a *closed* record's own sorted
   field-name set) alongside its original `Ref`-keyed one, populated
   whenever a target is `CType::Record` with no open row (`instance Eq
   (HasX a)`-style open targets still correctly report
   `InstanceTargetNotNominal` -- there's no fixed, exact shape to match
   against when a use site could still gain more fields via `a`).
   `check_instance` tries a matching record instance first, falling back
   to the structural `Eq`/`Ord`/`Show` derivation only when none exists,
   so a custom instance correctly overrides rather than merely duplicating
   it. **One accepted, documented limitation**: this keys purely on field
   *names*, not field *types*, so two unrelated record aliases that happen
   to share an identical field-name set and both want the *same* interface
   in the *same* module would wrongly collide as a duplicate -- narrow
   enough to accept rather than block the common case on.
6. **Fixed. `let`-bound local functions couldn't take parameters** —
   `let go acc rest = ... in ...` was a hard parse error (`expected \`=\``);
   `let_binding` in `knot-syntax` only ever parsed a bare pattern then `=`,
   with no parameter-list sugar at all. Only `let go = \acc rest -> ...`
   worked. Both Elm and Haskell support the parameterized form for local
   recursive helpers, which is exactly the idiom
   `data_structures/linked-list-reverse.knot` reaches for naturally, and
   which now passes. Fixed by reusing the exact same params-loop a
   top-level `FnDef` already has (`pattern_atom`s collected until `=` or
   the enclosing layout block ends) directly inside `let_binding`, folding
   any collected params into a `Lambda` wrapping the body -- the same
   desugared shape the `\acc rest -> ...` workaround already produced, so
   nothing downstream of the parser needed to change. Only a bare `Var`
   pattern can take params at all (`let (a, b) c = ...` still correctly
   rejects `c` rather than parsing it as a bogus parameter) -- there's no
   "call" a destructuring pattern makes sense as the head of.
7. **Fixed. `Map`'s own key-value API (`get`/`insert`/`empty`/...) didn't
   exist at all** — `collections/build-map.knot`, `collections/word-count-
   attempt.knot`. `knot-canonical`'s own name resolution already accepted
   `Map.get`-style qualified references at face value (trusted, no real
   `import Map` needed, in the snippet-mode `canonicalize_decls` this
   corpus and `corpus/semantic` both run through) — the type *checker*
   simply had no scheme registered for any of them (`UnboundValue`), even
   though `Map` itself was already a valid `Collection` target for
   `map`/`filter`/`foldl`/... once finding #3's own fix landed. **Fix**: a
   new `prelude::seed_map_module`, seeding `Ref::Imported(["Map"], _)`
   schemes for `empty`, `isEmpty`, `size`, `keys`, `values`, `toList`,
   `fromList`, `get`, `member`, `insert`, `remove` — concrete, ordinary
   constrained polymorphism over `k`/`v` (matching `compare`/`show`), not
   `Collection`/`Context`-generic, since every one of these is specific to
   `Map`'s own two-visible-parameters shape rather than "any collection."
   Every key-comparing operation requires `Eq k =>` (there's no other way
   to test key equality); ones that only touch values or shape don't.

Every finding from all three rounds is now fixed; `corpus/programs` is
80/80 clean.

### Round 2 (2026-08-02)

A deeper batch of 19 more fixtures (62 total): sorting algorithms, monoid-
based reports, layered config via record spread, a stack-language
interpreter, nested generics (`Maybe (List (Maybe a))`-shaped), and
multi-interface constraints. 50 of 62 passed initially. 6 of the failures
are already explained by Round 1's own findings above (#3, #5, #6, #7 —
same root causes, different fixtures: `errors/parse-pipeline.knot`-style
`Result` do-notation, `interfaces/point-ord.knot`-style custom `Ord`,
`data_structures/linked-list-reverse.knot`-style parameterized local `let`,
`collections/*`'s missing `Map` API). Of the remaining 6, one
(`patterns/currying-and-composition.knot`) turned out to be this round's
own authoring mistake, not a bug — `addFive :: Int -> Int -> Int`
genuinely still needs a second argument before the result is a plain `Int`
a `|>` stage downstream can accept; fixed the fixture, not the checker. The
other 5 traced back to three distinct root causes, all now fixed:

8. **Fixed (`knot-syntax`, no numbered `Fix #N` — that convention is
   `knot-checker/src/lib.rs`'s own). Unary negation wasn't recognized at
   the very start of a parenthesized or bracketed (sub)expression** —
   `classify_minus`'s whitespace-only heuristic answered `Subtraction` for
   symmetric spacing (both absent, as in `(-40.0)` or `[-5, -6]`'s first
   element) regardless of *where* the `-` sat, even immediately after `(`
   or `[` where no left operand could possibly exist for it to subtract
   from. Hard "Expected an expression" parse errors on `f (-5)`,
   `[-1, -2, -3]`, any parenthesized/bracketed leading negative literal.
   Found via `numeric/clamp-and-abs.knot`'s `clamp (-40.0) 50.0 raw`. Fixed
   by moving the `Subtraction`-shaped-spacing check into `expr_app`'s own
   trailing-argument loop (the one place a real preceding operand exists to
   back off to) and treating that spacing as negation everywhere else.
9. **Fixed (Fix #13). A signed function's header-vs-inferred-type `Equal`
   constraint solved *after* its own body, not before** — so a nested
   `let` inside the body (a hand-rolled quicksort's `smaller`/`larger`, or
   any similar locally-filtered/derived binding) generalized over a
   parameter-derived variable that hadn't been unified into the enclosing
   signature's rigid type yet, at that point in solving. It looked like a
   fresh, ambient-free, freely quantifiable variable, so its interface
   obligation (`Ord`, from `filter`'s own comparison) got dragged into the
   *nested* binding's own scheme instead of correctly staying tied to the
   already-`given`-covered rigid one — misfiring `AmbiguousConstraint` on
   perfectly ordinary code. Hit `algorithms/quicksort.knot` and
   `multi_interface/generic-function-multi-constraint.knot` (its own `let
   biggest = ...` inside `rankAndShow`) — both now pass. Fixed in
   `constrain::decl::constrain_group_chain` by solving the header `Equal`
   before the body's own constraints.
10. **Fixed (Fix #14). A parametric instance's own recursive `requires`
    check had no way to resolve a bare rigid variable** —
    `interface::instance::check_instance`'s recursion into e.g. `instance
    Ord a => Semigroup (Max a)`'s own `Ord` requirement on its argument hit
    a rigid `a` (a signature's or instance's own type variable), which
    `Substitution::resolve_structure` always answers `None` for by design
    — so the check answered `false` unconditionally no matter how
    thoroughly `given` already established the interface, misreporting
    `NoInstance` for the *outer* interface (`Semigroup`, not `Ord`). This
    also broke self-referential parametric instances recursing into their
    own element type (`instance Show a => Show (Tree a)` calling `show` on
    child nodes). Hit `monoids/max-min-via-ord.knot` and
    `multi_interface/recursive-tree-show.knot` — both now pass. Fixed by
    threading `given` (now also returned from `solve::
    solve_with_obligations`) through `check_instance`/`check_pending` and
    `elaborate`'s own dictionary-resolution functions.

After all three fixes, 56 of 62 pass; the remaining 6 are exactly the
Round-1-numbered failures above, unchanged.

### Round 3 (2026-08-02)

Another 18 fixtures (80 total), reaching into territory the first two
rounds hadn't touched: graph algorithms over association-list-encoded
graphs (BFS shortest path, union-find), a small arithmetic-expression
interpreter with `Result`-based error handling, deeply nested tuple/case
patterns, a fluent record-builder chain via `|>`, a hand-declared
`instance Num` overloading `+`/`-`/`*` on a custom `Vector2` record, and a
deliberate stress test of negative-number syntax across many more
positions (record fields, tuples, nested application) now that round 2's
own parser fix landed. 69 of 80 passed initially. 6 of the 11 failures are
already explained by earlier rounds' own findings (#3, #5, #6, #7 — same
`Map` API gap, the same parameterized-local-`let` parse gap, the same
`Result`-via-`Collection`/`Context` arity mismatch, and the same
`InstanceTargetNotNominal` blocking custom instances on records, now also
confirmed for `Num` via `interfaces/vector2-custom-num.knot` — see finding
#5's own updated text above). The other 5 all trace back to one single
root cause, **not yet fixed**:

11. **Fixed. Operator sections weren't parseable as expressions at all** —
    `(+)`, `(-)`, etc. were valid *method names* inside an
    `interface`/`instance` declaration (spec §6.2's own `(+) :: a -> a ->
    a`), but there was no corresponding expression-level syntax for
    referencing one as a first-class value the way `zipWith (+) xs ys` or
    `combine (+) l r` need to. `expr_paren_tuple_or_unit` only ever tried a
    full `self.expr()` for whatever followed `(` (or treated an immediate
    `)` as `Unit`) — a bare `+` there failed to start an atom, non-fatally,
    so `expr_app`'s own trailing-argument loop quietly backed off and
    stopped collecting arguments right there, rather than surfacing a real
    error. The enclosing binding ended up defined as just the un-applied
    head (e.g. `sampleSums = zipWith`, dropping `(+) [1, 2, 3] [10, 20,
    30]` entirely), and everything after it was reported as leftover
    input. Hit `collections/zip-and-unzip-manual.knot` and
    `interpreter/calculator-with-errors.knot` — both now pass.
    **Decided Elm-style, not Haskell-style**: only the bare `(op)` form is
    supported (parses to its own `Expr::OpRef` node in `try_operator_section`
    — does not desugar to a lambda, spec §7.6 — reusing `expr_binop_prec`'s
    own `peek_binop` table so it can't drift out of sync) — no partial
    sections like
    `(+ 1)`/`(1 +)`, which now hard-fail with a message pointing at the
    lambda-form alternative. Two things needed care: a bare `(-)` is
    unambiguously the *subtraction* function (never negation, matching
    Haskell's own rule there), while `-` immediately followed by anything
    *other* than `)` (e.g. `(-40.0)`'s own zero-spacing shape) must fall
    through to the ordinary negation path from round 2's own fix, not be
    misreported as a failed section; and `div`/`mod` never trigger section
    parsing at all, since (unlike every symbolic operator) they're already
    ordinary lowercase identifiers usable as a value with zero new syntax
    — treating them the same way misidentified plain applications like
    `(div n 2)` as a failed section attempt during testing, caught before
    landing.

After that fix, 71 of 80 pass. After also fixing finding #6, 72 of 80
pass. After also fixing finding #5 (see its own updated text above), 76 of
80 pass. After also fixing finding #3 (also see its own updated text
above), 78 of 80 pass. After also fixing finding #7 (Round 1's own, see
its updated text above), all 80 of 80 pass -- every finding logged across
all three rounds is now fixed.

### Round 4 (2026-08-02)

18 more fixtures (98 total), deliberately weighted toward monad/`do`-
notation coverage: `Result`-based validation pipelines (chained and
nested), a hand-declared `Context` instance on a brand-new user type
(`Box`, the simplest possible custom monad), a second custom `Context` on
a genuinely 2-parameter user ADT (`Outcome e a`, an `Either`-shaped type,
directly exercising finding #3's own fix on *user* code, not just the
builtin `Result`), `List`'s own do-notation (cartesian products, list-
comprehension style), realistic uses of the now-working `Map` API
(word-frequency counting, an inventory system, a graph's adjacency map),
bare operator sections used in real folds (`foldl (+) 0 xs`,
`foldl (<>) "" strs`), a custom `Num` instance combined with operator
sections (`foldl (+) origin vectors`), a deliberate numeric-literal-
polymorphism stress test, and a single record type carrying `Eq`+`Ord`+
`Show`+`Num` custom instances all at once. 97 of 98 passed initially.

12. **Fixed. `constrain::decl::constrain_method_body_against` had the
    identical header-vs-body solve-order bug Fix #13 fixed for ordinary
    function bindings, in instance methods' own separate code path** —
    `multi_interface/eq-ord-show-num-all-together.knot`'s own `Show Money`
    instance computes intermediate values via a local `let` (`dollars =
    div m.cents 100; remainder = mod m.cents 100`) before formatting them,
    exactly the same shape as Fix #13's own quicksort `smaller`/`larger`
    example — misfiring `AmbiguousConstraint("Integral")`. Fix #13 only
    ever touched `constrain_group_chain` (ordinary top-level/`let`
    bindings); `constrain_method_body_against` (instance methods'
    equivalent) independently builds the exact same header-`Equal`-after-
    body shape, so it never received that fix. **Fixed** the same way:
    solve the header `Equal` before `body_constraints`, so a param-derived
    variable is already unioned with the instance's own rigid target by
    the time a nested `let` inside the method body might otherwise
    misgeneralize over it.

After this fix, 98 of 98 pass.

### Round 5 (2026-08-03)

13 more fixtures (113 total), deliberately aimed at territory the first
four rounds never touched: a user-declared `Collection` instance that
calls the polymorphic `map`/`foldl` *recursively* on its own wrapped
`List` (`collections/custom-collection-bag-of-list.knot`) and, separately,
on a genuinely recursive ADT (`collections/binary-tree-collection-
fold.knot`); the brand-new custom-`Tuple`-instance feature (this session's
own Task #39, above) exercised in realistic code for the first time
outside `corpus/semantic`'s own minimal pinning fixtures — an insertion
sort keyed by a custom `Ord (Int, Int)` (`interfaces/point-tuple-ord-
sort.knot`), vector math via a custom `Num (Float, Float)` folded with an
operator section (`numeric/vector-tuple-num-fold.knot`), a 3-tuple
standing in for a lightweight row with custom `Eq`/`Show`
(`interfaces/tuple-show-eq-report.knot`), a record whose own field is a
custom-`Show` tuple (`records/record-with-tuple-field-custom-show.knot`),
a priority queue keyed directly by a tuple
(`data_structures/priority-queue-tuple-keys.knot`), and one generic
`Eq a =>` function called at both a custom-tuple and a custom-record
concrete type (`multi_interface/generic-eq-over-tuple-and-record.knot`);
exhaustiveness warnings (Task #37, above) in a realistic 4-constructor
expression-AST evaluator missing one arm, plus its clean companion
(`patterns/expr-ast-missing-arm-warns.knot` and `.../expr-ast-all-arms-
clean.knot`), and specifically *inside* a custom instance method's own
body rather than an ordinary top-level function (`patterns/instance-
method-non-exhaustive-show.knot`); annotations spanning a genuinely
mutually-recursive pair of
bindings (`patterns/annotations-on-mutual-recursion.knot`); and `@unravel`
on a real 2-argument function, exercising `derive_unravel_type`'s own
multi-parameter tuple-collapsing path from realistic-looking code
(`patterns/unravel-on-multi-arg-function.knot`). 111 of 113 passed
initially — no parse errors at all this round, both failures were
immediately genuine type-check findings:

13. **Fixed (Fix #15). `Ordering` (`LT`/`EQ`/`GT`) had no seeded
    `Eq`/`Ord`/`Show` instance at all** — an entirely ordinary idiom,
    `compare a b == LT`, was a hard `NoInstance("Eq")`, since `Ordering`
    is a plain 3-constructor ADT (`knot-canonical::prelude::
    BUILTIN_CONSTRUCTORS`) the exact same shape as `Bool`'s `True`/`False`,
    but `prelude::seed_instances`'s own Eq/Ord/Show loop only ever included
    `Int`/`Float`/`String`/`Bool`, never `Ordering`. Hit
    `interfaces/point-tuple-ord-sort.knot` and `data_structures/priority-
    queue-tuple-keys.knot`, both of which compare a `compare` result
    directly rather than only ever pattern-matching it — both now pass.
    **Fixed** by adding `"Ordering"` to that same loop.

After this fix, 113 of 113 pass.
