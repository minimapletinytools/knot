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
fixtures, `cargo run --example corpus_report -p knot-checker`), **not yet
fixed** — 16 of 43 fixtures fail, all tracing back to 7 distinct root
causes:

1. **`Semigroup`/`Monoid` have zero builtin instances anywhere** — `<>` and
   `empty` fail on every builtin type, including `String`. Breaks the most
   basic string-building pattern outright. Hits `text/*`,
   `errors/error-mapping.knot`, `interfaces/shape-show.knot`,
   `data_structures/json-value.knot`, `options/dict-lookup-fallback.knot`.
2. **Integer literals never unify with `Float`** — `x :: Float; x = 5` is a
   hard `Unify::Mismatch`. No numeric-literal polymorphism at all. (Avoided
   in this batch's own fixtures once known; still real, see the
   conversation history's own minimal repro.)
3. **`Collection`/`Context` are broken for both of their 2-parameter
   built-ins** — the `VarApp` mechanism only ever constructs a ctor
   application with exactly one argument and requires exact arity match
   against the concrete type, so it silently can't handle a constructor
   that takes two parameters where only the last one varies. Confirmed via
   real code this round: `do`-notation over `Result` fails in both
   `errors/parse-pipeline.knot` and `errors/validate-user.knot`.
4. **`given` facts are never closed over superclass relationships** — a
   signature's own `Ord a =>` only ever inserts `"Ord"` into `a`'s `given`
   set, never the implied `"Eq"`, even though every real `Ord` instance is
   *guaranteed* to have a matching `Eq` one (superclass coherence enforces
   this at declaration time). Same story for `Monoid` never implying
   `Semigroup`. Hits `data_structures/binary-search-tree.knot` (`contains`
   uses `==` under `Ord a =>`) and `interfaces/combine-all.knot` (`<>`
   under `Monoid a =>`). `solve.rs` populates `given` in exactly two places
   (`Constraint::Let`'s declared scheme, `Constraint::Given`), both a plain
   `insert`, no superclass closure — `interface::table::superclasses`
   already exists and is used for coherence checking, so this looks like a
   small, mechanical, low-risk fix once decided on.
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
