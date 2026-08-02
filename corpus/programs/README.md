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
2. **Integer literals never unify with `Float`** — `x :: Float; x = 5` is a
   hard `Unify::Mismatch`. No numeric-literal polymorphism at all. (Avoided
   in this batch's own fixtures once known; still real, see the
   conversation history's own minimal repro.) **Not yet fixed.**
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
5. **`InstanceTargetNotNominal` (this session's own Fix #9) blocks
   legitimate custom `Eq`/`Ord`/`Show` instances on records, not just the
   interfaces with no structural fallback** — `interfaces/point-ord.knot`
   wants `Ord` on a `Point` record *by magnitude*, a real, common pattern
   (order-by-derived-key) the automatic structural per-field derivation
   could never produce on its own. Fix #9's own reasoning ("no structural
   fallback exists, so an explicit instance is unreachable dead code") is
   right for `Semigroup`/`Monoid`/etc. but wrong for `Eq`/`Ord`/`Show`
   specifically, where a real fallback exists that an explicit instance
   should be allowed to *override*, not just duplicate. A likely
   overcorrection in that fix, found by using the feature it changed.
6. **`let`-bound local functions can't take parameters** — `let go acc
   rest = ... in ...` is a hard parse error (`expected \`=\``);
   `let_binding` in `knot-syntax` only ever parses a bare pattern then `=`,
   with no parameter-list sugar at all. Only `let go = \acc rest -> ...`
   works. Both Elm and Haskell support the parameterized form for local
   recursive helpers, which is exactly the idiom
   `data_structures/linked-list-reverse.knot` reaches for naturally.
   Left the fixture exactly as a real author would write it (not rewritten
   to dodge the gap) — it's a genuine parse failure, not a typo.
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
