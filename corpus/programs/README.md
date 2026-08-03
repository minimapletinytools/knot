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
3. **`Collection`/`Context` are broken for both of their 2-parameter
   built-ins** — the `VarApp` mechanism only ever constructs a ctor
   application with exactly one argument and requires exact arity match
   against the concrete type, so it silently can't handle a constructor
   that takes two parameters where only the last one varies. Confirmed via
   real code this round: `do`-notation over `Result` fails in both
   `errors/parse-pipeline.knot` and `errors/validate-user.knot`.
   **Not yet fixed** — needs a real design for partially-applied
   constructors in `VarApp`, not a quick patch.
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
7. **`Map`'s own key-value API (`get`/`insert`/`empty`/...) doesn't exist**
   — `collections/build-map.knot`, `collections/word-count-attempt.knot`.
   Already a documented, known-open design question
   (`knot-canonical::prelude`'s own doc comment on qualified `List.map`/
   `Map.lookup`-style access), not a fresh discovery — noted here for
   completeness since it's exactly what a real program hits immediately.

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
    supported (desugars to `\x y -> x op y` at parse time, in
    `try_operator_section`, reusing `expr_binop_prec`'s own `peek_binop`
    table so it can't drift out of sync) — no partial sections like
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
80 pass; the remaining 4 are exactly the still-open Round 1 findings (#3,
#7) -- every fixture written across all three rounds so far now passes
except the `Map` API gap and the `Collection`/`Context` 2-parameter arity
mismatch.
