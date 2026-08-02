# Unravel Examples

A running collection of concrete scenarios worked through during unravel design —
intended to grow over time as new cases come up. Each entry: what's being asked, what
actually happens, and the one thing worth remembering from it. Full derivations for
the earlier entries live in `7-29-2026_unravel_discussion.md`; this doc is the
index/summary, not a replacement for that reasoning.

Signature reference (current, per the discussion doc §10):
```knot
type alias UnravelInput a = { orig : a, hints : List a }

type Sensitivity a = Exact a | Range a a | Tolerance a a | Free

f_unravel :: Sensitivity out -> UnravelInput a -> UnravelInput b -> UnravelInput c -> Option (a, b, c)
```

---

## Baseline: single invertible operation

**Setup**: `x = f y`, `f` a simple invertible built-in (e.g. `negate`, or `+` with a
constant).

**What happens**: `Sensitivity b` is `Exact target`, `f_unravel` inverts the single
operation directly, returns `Some new_a`. No ambiguity, no candidates needed.

**Category**: trivial / baseline.

**Lesson**: this is the "nothing interesting happens" case every other example is
interesting *relative to* — worth keeping in the collection as the reference point.

---

## `sum`/`foldl` — naive composition produces a lopsided, wrong-feeling result

**Setup**: `sum = foldl (+) 0`, applied to `[1..7]`. Want the total to change by +7.

**What happens**: the trace is a fully left-nested chain of `+` (matches how `fold`
actually executes, not "every term at once"). Naively auto-composing `(+)`'s own
default unravel ("split evenly between the two operands") through that nested tree
gives the *last* list element half the total delta and the *first* element 1/128th of
it — purely an artifact of the fold's associativity, nothing to do with what "sum"
means.

**Category**: default-unraveler pitfall.

**Lesson**: aggregate/fold-shaped functions need a hand-authored, genuinely n-ary-aware
default unravel (split evenly across *all* elements directly), not naive composition
of a binary op's default through whatever tree shape the implementation happens to
build. General rule: be suspicious of any default that's derived by composing smaller
defaults through a data structure whose *shape* wasn't chosen for this purpose.

---

## Diagonal square (`xy`) — structurally unsolvable

**Setup** (Sketch-n-Sketch's own worked example): `xy = 100`, `x = xy`, `y = xy` — a
square whose position is bound to one shared variable. Want to move `x` without moving
`y` (or vice versa).

**What happens**: overconstrained — `xy` would need two different values
simultaneously. Sketch-n-Sketch's own fallback ("apply substitutions in arbitrary
order," satisfying "at least one" constraint) is explicitly *not* what Knot should do
— that's their own "plausible, not faithful" update, and Knot's stance is exact-or-fail
everywhere.

**Category**: structurally unsolvable / convergence.

**Lesson**: this is the same underlying phenomenon as a bad regression fit below —
degrees-of-freedom vs. constraint-count. 1 free parameter, 2 simultaneous exact
demands, generically no exact solution. Averaging as a default response to this shape
of conflict produces a worst-of-both-worlds result satisfying nobody — better to
satisfy one constraint fully (declared priority) or fail cleanly.

---

## Linear fit through `map` — many-unravel, and why "close" isn't good enough

**Setup**: `g = \m c x -> m * x + c`, used as `map (g m c) xs`. User drags several
output points at once.

**What happens**: reframe `g` as `params -> p -> q` (params = `(m, c)`), and solving
for a shared `params` across N simultaneous (target, input) pairs is literally linear
regression *if approximate solving were allowed*. It isn't (per the exact-or-fail
principle) — so a many-unravel for this needs either an exact fit (only possible if
the targets happen to be consistent with the family of functions `g` can represent) or
a clean failure, never a least-squares approximation presented as if it satisfied the
request.

**Category**: many-unravel / exact-vs-approximate.

**Lesson**: this is what motivated the richer `Sensitivity` vocabulary (`Exact | Range
| Tolerance | Free`) — "approximate" should never be something the solver silently
decides to do; it should be something the *user* explicitly opts into per-field via
`Range`/`Tolerance`, keeping the solver's own contract exact-satisfaction throughout.

---

## `scaled`/`offset`/`double_scaled` — why push-forward beats blind backtracking

**Setup**:
```
base = 10
scaled = base * 2      -- 20
offset = base + 5      -- 15
result = scaled + offset   -- 35
double_scaled = scaled * 2   -- 40
```
Want `result` → 40 and `double_scaled` → 44, simultaneously.

**What happens**: `double_scaled`'s demand is univariate and forced — `scaled` *must*
become 22, no alternative. `result`'s demand is locally ambiguous on its own (could
adjust `scaled` or `offset`) and its own candidate-generation has no way to know
"scaled=22" is the value that's actually needed — it wasn't computed with that
constraint in mind. Blind backtracking through `result`'s own guesses can miss the
real solution entirely. Push-forward fixes this by re-solving `result`'s equation
*with* the forced value already known: `40 = 22 + offset → offset = 18`, derived
directly rather than guessed.

**Category**: push-forward / constraint propagation.

**Lesson**: this is what motivated collapsing the unravel's return from `List a`
(caller-side backtracking through self-generated candidates) to `UnravelInput a` with
a `hints` field (framework proposes *informed* candidates, unravel just checks them).
Named precedent: this is "forward checking" from the CSP literature, not a new idea —
propagate what's forced first, fall back to search only for genuine remaining
ambiguity.

---

## Scene graph list append — turns out not to be unravel at all

**Setup**: UI lets a user add a new object to the scene graph directly (not by editing
an existing output's value, but by creating a brand new one — e.g. dropping a new Cube
onto the canvas). The scene graph is a `List SceneObject` in source, built by
collecting individually-named object bindings.

```knot
cube1 :: SceneObject
cube1 = Cube { pos = (0.0, 0.0, 0.0), size = 1.0 }

sceneObjects :: List SceneObject
sceneObjects = [cube1]
```
becomes
```knot
cube1 :: SceneObject
cube1 = Cube { pos = (0.0, 0.0, 0.0), size = 1.0 }

@nodeId("obj-a3f9e1")
@position(320.0, 180.0)
cube2 :: SceneObject
cube2 = Cube { pos = (3.0, 3.0, 3.0), size = 1.0 }

sceneObjects :: List SceneObject
sceneObjects = [cube1, cube2]
```

**What happens**: the new object's value is fully known to the UI the moment it's
created — there's no unknown to solve for, so `Sensitivity`/`UnravelInput` never enter
the picture. What's actually needed is a different operation, **materialization**:
(1) serialize the known value into Knot source via a canonical, always-reparseable
structural walk (related to, but distinct from, an overridable `Show` instance, which
might not round-trip); (2) mint a fresh binding name plus `@nodeId`/`@position`
annotations from data the UI already has; (3) locate the target list via the
node-identity/span tracking already needed for UI-layout preservation; (4) splice in
the new binding and a reference to it. Deletion is the natural mirror operation, same
non-unravel mechanism.

**Making this a framework feature, not a one-off, without a new interface**: turned out
insert/remove don't need type-class dispatch at all, and inventing one (an earlier
draft of this entry proposed a `Growable` interface) was solving a problem that
doesn't exist — insert/remove are pure AST rewrites over a *literal* `[e1, e2, ...]`
node (permute/extend the `Vec`, reprint), which needs zero knowledge of the element
type, so there's nothing to abstract over per-type. Any `Expr::List` literal, anywhere
in a program, is materializable by default — no annotation required to turn it on. The
useful annotation is the opposite: an explicit **opt-out**, for the (rare) case where
the default convenience behavior would be semantically wrong for a specific list:
```knot
@nomaterial(())
sceneObjects :: List SceneObject
sceneObjects = [cube1, sphere1]
```
`@nomaterial` doesn't just hide a UI affordance — it makes any *attempted*
materialize/remove against that binding fail cleanly, consistent with exact-or-fail
everywhere else in this design, rather than silently doing the default anyway. (Note:
`@nomaterial(())` rather than a bare `@nomaterial` — `knot-syntax`'s annotation grammar
currently requires a parenthesized argument after every key; a bare-flag form would
need an actual, small parser change, not assumed to already work. `()` is used rather
than `True`/`False` since only presence is ever meaningful — there's no sensible
`@nomaterial(False)`.)

There's a second, independent reason materialize can be unavailable, not covered by
`@nomaterial`: the element type needs a *derived* (not overridden) `Show` instance —
plain data, no embedded functions — checked automatically by the type checker, a
compile-time error rather than a runtime surprise for something like a
`List (Int -> Int)`. `@nomaterial` is for "mechanically possible, author doesn't want
it"; a non-serializable element type is "mechanically impossible regardless of what
the author wants" — kept as two distinct reasons, not conflated into one check.

`Map` needs no separate treatment either, for a reason specific to this language:
Knot has no `Map` literal syntax at all (spec §2.2 — `Map.fromList [...]` is the only
constructor), so a `Map` in source is always, syntactically, a function call wrapping
an ordinary `List (k, v)` literal. Materializing into "the Map" is really materializing
a new `(key, value)` tuple into *that* underlying list literal — the exact same
mechanism, no Map-specific logic needed. Tuples, by contrast, genuinely don't apply:
fixed arity means "growing" one would change its type, not just its value, a different
and harder operation than materialization is meant to cover; per-position value change
within an existing tuple is already just ordinary unravel (nested `Sensitivity`
recursion, §6 of the discussion doc), nothing new needed there either.

**Category**: not unravel — a distinct, simpler, always-succeeding mechanism; default
behavior for a closed set of recognized literal shapes (today: list literals only),
opt-out rather than opt-in.

**Lesson**: worth keeping as a boundary marker. The dividing line between "materialize"
and "unravel" is exactly "is the new/changed value already fully known, or does
something need to be solved for." List-length changes in general were already flagged
as out of scope for unravel proper (Sketch-n-Sketch can't do them either, for the same
underlying reason) — this example shows the *reason* a length change specifically
tends to be tractable anyway when it comes from a UI-side creation gesture: the
apparent "new code synthesis" problem dissolves because there's no synthesis
happening, just serialization of an already-concrete value. A case where the new
element's parameters needed to be *derived* from existing scene state (not just a
literal drop) would reintroduce genuine unravel-style solving on top of this base
mechanism — still noted as a possible future entry, not built here. Also worth
remembering: the "must be a literal, not a computed expression" restriction isn't
unique to any one mechanism here — it's the actual scope boundary for the whole
materialize/reorder family, more fundamental than any annotation.

---

## List element value change — already covered, no new mechanism

**Setup**: same scene, `sceneObjects = [cube1, sphere1]`. Want `cube1`'s position to
change — list stays the same length, same elements occupy the same slots, only one
element's *value* differs.

**What happens**: nothing new required. A list literal's slots are ordinary expression
positions like any other — `Sensitivity (List b) = List (Sensitivity b)` (§6 of the
discussion doc) already routes a per-slot target backward through whatever occupies
that slot. Slot 0 is a bare `Var("cube1")` reference, trivially invertible (identity),
so the target backward-propagates straight to `cube1`, and `cube1`'s own unravel
(hand-written, or the recursive per-field default) takes it from there. Being "inside a
list" changes nothing about how unravel already works for the thing occupying a slot.

**Category**: baseline (list case).

**Lesson**: worth keeping as the explicit contrast case for the two entries below —
it's what confirms *length being unchanged* is not, by itself, sufficient grounds for
positional correspondence to be correct. It's correct here specifically because
nothing's identity moved, only a value changed in place.

---

## List reordering — positional matching gives the wrong answer

**Setup**: same list, same elements, same length — `[cube1, sphere1]` →
`[sphere1, cube1]`. Nothing about either object's own value changed, they just swapped
position.

**What happens**: applying the same positional `Sensitivity (List b) = List
(Sensitivity b)` logic here is actually wrong, despite the length being unchanged —
slot 0 (currently `cube1`) gets told "become `sphere1`'s current value" and vice versa.
That tries to make the two objects swap *identities*, not swap *positions* — semantically
backwards, and if the two ever have incompatible shapes, either a spurious type error or
a coincidentally-typechecking but meaningless result.

**The fix**: match old-list elements to new-list elements by *identity* (`nodeId`), not
by index — the same "keyed diffing" approach React's virtual-DOM list reconciliation
(and similar UI frameworks) uses list keys for, for exactly this reason. Once matched
by key:
- new-list key with no old match → **materialize** (insert)
- old-list key with no new match → **remove**
- key present in both, different index → **reorder**: pure source-text rearrangement
  of the list literal's existing element references, no solving at all — same
  "already fully known, nothing to solve for" character as materialization
- key present in both, same/moved index, different value → **ordinary unravel** (the
  entry above)

These four classifications are independent, so mixed edits (something added, something
else reordered, a third thing's value changed, all in one interaction) decompose
cleanly per-key rather than needing one uniform diff strategy.

**Category**: list diffing / identity vs. position.

**Lesson**: like materialize/remove, reorder turned out to need no interface and no
opt-in — it's a pure source-text rearrangement of a literal `Expr::List` node, which
needs zero knowledge of element type or value, so there's nothing to dispatch on.
Default-on for any recognized list literal; opt out explicitly, same shape as
`@nomaterial`:
```knot
@noreorder(())
```
Kept independent from `@nomaterial` rather than combined — a fixed-size top-3
leaderboard might be reorderable but not growable; a tag bag might be growable with no
meaningful order at all; a list can disable neither, either, or both. `@noreorder`
gives the same clean-failure guarantee as `@nomaterial`: an attempted reorder against
a `@noreorder`-marked list fails rather than silently proceeding. Open fallback
question, not resolved: lists whose elements carry no identity at all have no key to
match on — value-based diffing (Myers/LCS-style, treating value-equality as the match
key) is the fallback, but it's strictly less precise (can't distinguish "coincidentally
equal value" from "same underlying thing"), so requiring `nodeId` for anything in a
list where reorder/materialize might apply is probably better than building and
maintaining two diffing strategies.

---

## Open format note

Each entry above follows: Setup → What happens → Category → Lesson. Keep using that
shape for new entries so this stays a fast reference rather than another discussion
transcript. Categories seen so far: baseline, default-unraveler pitfall, structurally
unsolvable / convergence, many-unravel / exact-vs-approximate, push-forward /
constraint propagation, not-unravel, list diffing / identity vs. position. New
categories are fine — flag one explicitly when an example doesn't fit the existing
set, since that itself is usually the interesting part.
