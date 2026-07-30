# Unravel Design Discussion — 2026-07-29 (continued 2026-07-30)

A forked side-conversation working through `unravel` (reverse execution / pullback)
design, starting from research into Sketch-n-Sketch (the closest real academic prior
art), through a concrete signature proposal and push-forward / constraint propagation,
and continuing into a second day covering materialization (§11) and list diffing
(§12) — cases that turned out not to be `unravel` proper, but sit close enough to it
that they needed working out in the same pass. Nothing here is committed to the spec
yet — this is a discussion log to work from, not a decision record. See also
`unravel-examples.md` for the same material in reference-card form.

---

## 1. Prior art: Sketch-n-Sketch

Researched directly (fetched and read, not from memory): "Programmatic and Direct
Manipulation, Together at Last" (PLDI 2016) and "Bidirectional Evaluation with Direct
Manipulation" (OOPSLA 2018), Ravi Chugh et al.

- **Trace**: alongside every numeric value, the evaluator records a syntax tree of
  exactly which source expression produced it (`n^t`, `t ::= ℓ | (op t₁…t_m)`).
  Dataflow-only — control flow is deliberately not recorded.
- **Update = equation solving**: given a new desired output value, invert primitive
  ops top-down, syntax-directed, to solve for a single source location — works when
  that location occurs exactly once ("univariate").
- **Ambiguity heuristics**: "fair" (round-robin over candidate locations not yet
  claimed by another shape) and a secondary "bias toward locations used in fewer
  traces." Both purely structural bookkeeping — no scoring, no search, computed once
  upfront into a precomputed "trigger" closure so live dragging is just applying a
  substitution.
- **"Shapes"/"zones"**: shapes are the rendered SVG output objects; zones are
  draggable handles on a shape, each corresponding to one attribute. Dragging a zone
  is the gesture that fires a trigger.
- **Stated scope/limitations**: explicitly cannot introduce new control flow, cannot
  change list length, cannot synthesize new code/structure — only adjusts existing
  constants. The Ferris wheel `numSpokes` example: structural changes get punted to an
  explicit UI slider instead of direct manipulation.
- **No per-function reverse rule**: it's a single *global* equation solve over the
  whole flattened trace, not compositional.

**Key architectural difference from Knot**: spec's `unravel` design (each function
carries its own local unravel; higher-order functions receive the callee's unravel and
invoke it during the backward pass) is compositional/local, structurally much closer
to **reverse-mode automatic differentiation** (a forward pass builds a graph, a
backward pass propagates a sensitivity through local per-node rules that compose) than
to Sketch-n-Sketch's global trace solve. This is a deliberate, more scalable
architecture — but see §2 for a concrete pitfall it doesn't automatically avoid.

---

## 2. The `sum`/`fold` pitfall

Worked example: `sum = foldl (+) 0`, applied to `[1..7]`. The trace is a fully
left-nested chain of `+`, not "every term at once." Sketch-n-Sketch's solver *can*
reach any individual term via top-down peeling regardless of nesting depth — but
disambiguating *which* term to blame for a given delta has zero real signal (all 7
literals are structurally tied, appearing in exactly one trace each), so its
heuristics degrade to arbitrary rotation, and it has no mechanism for distributing a
delta across multiple terms at once.

Naively auto-composing `(+)`'s own default unravel ("split evenly between the two
operands") through the fold's left-nested tree produces the **same kind of artifact**:
for 7 elements, the *last* one gets Δ/2 of the total sensitivity and the *first* gets
Δ/128 — purely from the fold's associativity, nothing to do with what "sum" means.

**Conclusion**: aggregate/fold-shaped functions need a hand-authored, genuinely
n-ary-aware default unravel (split evenly across *all* elements directly) rather than
relying on naive composition of a binary op's default through whatever tree shape the
implementation happens to build.

---

## 3. The unsolvable case: diagonal square (`xy`)

Sketch-n-Sketch's own worked example (§4.1): `let xy = 100 in rect 'red' xy xy ...` — a
square whose x and y are both bound to the same variable. Dragging diagonally is fine;
dragging purely horizontally (x should change, y shouldn't) is **overconstrained** —
`xy` would need two different values simultaneously. The paper's own fallback:
"apply substitutions in arbitrary order," satisfying "at least one" constraint — their
own terminology calls this a **plausible** update, distinct from and inferior to a
**faithful** one (satisfies everything).

This is the same underlying phenomenon as a bad regression fit (§7): a
degrees-of-freedom-vs-constraint-count mismatch. `xy` has 1 free parameter and 2
simultaneous exact demands — generically unsolvable, same as a 2-parameter line asked
to hit 5 independent points exactly.

---

## 4. Design principles established

- **Averaging as a convergence-conflict default is usually wrong** — produces a
  worst-of-both-worlds result satisfying nobody. Better to satisfy one constraint
  fully (via a declared priority) than partially satisfy all of them. Reserve
  averaging for cases the author explicitly opts into.
- **Two distinct failure modes**, worth keeping conceptually separate:
  1. Local failure — one unravel's own logic says "no solution."
  2. Structural/convergence failure — multiple downstream demands land on one shared
     upstream value, discovered by graph shape, not by any single unravel's own logic.
- **Multiple candidate solutions risk combinatorial explosion** if naively enumerated
  — addressed in §6 (laziness) and §9 (push-forward reduces reliance on blind search).
- **Exact-or-fail, never "plausible"** (§7) — a stronger stance than Sketch-n-Sketch's
  own fallback behavior.

---

## 5. Can we precompute whether an unravel will hit a convergence ("slipknot") before running it?

**In full generality: no**, for two independent reasons, both confirmed rather than
assumed:
1. Control flow is value-dependent (`if`/`case`), so the *realized* dependency graph
   for a given run is only known once branches are resolved — this is exactly why
   "define-by-run" autodiff systems (PyTorch-style) build their tape during actual
   execution rather than statically, unlike "define-and-run" systems that struggle
   with data-dependent control flow.
2. Unravels are arbitrary Turing-complete functions, so "will this ever produce a
   conflicting demand" is a non-trivial semantic property — undecidable in general by
   Rice's theorem.

**But a sound, useful check is still possible**, once the scope is corrected: the
relevant question isn't "does this value have multiple structural consumers anywhere
in the program" — it's "are there multiple things being *simultaneously, actively
demanded* by *this specific interaction*." That set is small and known upfront. Since
those specific targets must be *fully* forced anyway (to know their current values
before computing deltas), building the realized dependency trace from that deliberate,
complete forcing — not from whatever got forced incidentally — gives a sound answer
for that scope, decoupled from (and cheaper than) actually attempting the backward
solve. A value that's merely displayed but not actively demanded can still silently
change as a side effect (not a conflict — nothing was asked of it), which may warrant
a separate UX affordance, not a solver-level check.

*(Caught mid-discussion: an earlier draft of this reasoning relied on "whatever the
forward pass happened to force," which laziness can make incomplete/unsound. Fixed by
scoping to deliberate, complete forcing of the demand-set specifically.)*

---

## 6. Nested/partial output specification

Problem: an output like `f :: a -> b` where `b` is a struct `{x, y, z}` — caller may
only want to constrain `x`, leaving `y`/`z` unconstrained ("could be anything").

- **`Sensitivity T`, defined recursively**: push `Option` (or later, a richer sum
  type — §7) all the way to the *leaves*; recurse into records/tuples rather than
  wrapping the whole structure:
  ```
  Sensitivity(record{f1: T1, ...}) = record{f1: Sensitivity(T1), ...}
  Sensitivity(tuple(T1,T2,T3))     = tuple(Sensitivity(T1), Sensitivity(T2), Sensitivity(T3))
  Sensitivity(scalar T)            = Option T   -- later generalized, §7
  ```
  Same shape as "pytree" gradients in JAX/PyTorch-style autodiff for nested/structured
  outputs — precedent for exactly this recursive-mirroring pattern.
- **Semantic fork, resolved**: "don't care" (unconstrained, solver free to do
  anything) is *not* the same as "keep unchanged" (an active zero-delta constraint,
  which is *stricter* and can make an otherwise-solvable request infeasible — same
  shape as the `xy` overconstraint). `None`/unconstrained is the correct reading of
  "could be anything."
- **This reduces recursively to the same convergence problem**: once multiple fields
  are simultaneously constrained, the unravel body has to reconcile them using shared
  upstream inputs — identical in kind to top-level convergence, just scoped inside one
  function.
- **Ergonomics proposal**: let authors default to independent per-field unravels
  (simple, common case — most fields don't actually interact) composed via the same
  convergence machinery used at the top level; reserve a full joint/whole-record
  unravel for fields known to be genuinely coupled.
- **Rejected alternative**: representing partiality via row-polymorphic "subset of
  fields present" records. Elegant on paper, but doesn't fit an HM + row-polymorphism
  type system (which gives "at least these fields" polymorphism, not runtime dispatch
  over *which* subset is present — a fundamentally stronger feature not being built).
- **Rejected alternative**: reusing the `_` hole syntax for "don't care" — would
  overload an already-different meaning (static "not yet written" vs. a valid runtime
  "unconstrained" value).
- **Related, unsolved wrinkle**: `List T` doesn't fit the recursive scheme cleanly —
  `List (Sensitivity T)` assumes a fixed, matching length, which runs into the
  already-established "no length changes" boundary. Flagged, not solved.

---

## 7. Exact vs. approximate solving

Pushback on an earlier "least-squares fit" framing for many-unravel (§8): **not good
enough**. Resolved as:

- Adopt **exact-or-fail** as the invariant everywhere — never emit a Sketch-n-Sketch
  style "plausible" (partially-satisfying) result.
- This reframes solving as a **feasibility problem**, not an optimization problem:
  does *any* value satisfy every stated constraint exactly — if yes, find one (or a
  ranked few); if no, fail cleanly.
- Unifies §3's `xy` case and a bad regression fit as *the same phenomenon*
  (degrees-of-freedom vs. constraint-count mismatch), not two separate problems.
- **Richer `Sensitivity` vocabulary**, generalizing the `Option`-based one from §6, as
  the principled way to allow flexibility without silently violating an explicit
  demand — the user opts into slack per-field, the solver's contract stays exact:
  ```
  type Sensitivity a
    = Exact a          -- must land on this value precisely
    | Range a a         -- must land within [lo, hi]
    | Tolerance a a      -- must land within target ± epsilon
    | Free               -- genuinely unconstrained
  ```
  (Kept conceptually distinct from floating-point epsilon-equality, which is an
  implementation-hygiene concern, not a user-facing constraint kind.)

---

## 8. Higher-order functions and "many-unravel"

### Higher-order functions (`map`, `foldl`, etc.)

- **Bundling mechanism**: no new syntax — a function value carrying an existing
  `@{unravel = ...}` annotation just needs that pairing preserved when passed around
  as a first-class value (spec §11.3's "function strands carry their unravel bundled
  alongside their forward implementation," made concrete).
- **Worked signature (`map`)**:
  ```
  map :: (a -> b) -> List a -> List b
  map_unravel :: Sensitivity (List b) -> (a -> b) -> List a -> List (List a)
  ```
  The callback itself is treated as fixed/given, not something solved for — rewriting
  it would be synthesizing new code, already out of scope.
- **Combinatorial blowup solved for free by laziness**: `List` in Knot is already
  call-by-need. If the combined-candidate list is built lazily (head = everyone's
  first-choice combined, tail = further combinations only on demand), the cost of
  exploring alternatives is only paid if something downstream actually backtracks —
  no special mechanism needed beyond using the language's existing laziness correctly.
- **No unravel on the callback**: falls back to deriving one from composed
  primitive-op defaults if the body is simple enough, else the empty-candidate-list
  ("no solution") convention.
- **`foldl` flagged as harder**, not solved generically: same accumulator-coupling
  issue as §2's `sum` example — needs its own hand-authored n-ary-aware default,
  not a generic higher-order composition rule.

### "Many-unravel": solving for a shared function/parameters across multiple examples

Hypothetical explored: what if the callback itself (or its captured parameters) could
change, given a *list* of (input, desired-output) pairs simultaneously — e.g.
`map (g m c) xs` where `g = \m c x -> m*x + c`, wanting to move several outputs at
once. This is literally curve-fitting/regression *if* approximate solving were
allowed (§7 says it isn't by default) — reframed as: `g` is really `params -> p -> q`,
and many-unravel solves for one shared `params` given N simultaneous constraints.

```
g_manyunravel :: List (Sensitivity q, p) -> params -> List params
```

**Relationship to general convergence — corrected after initial overclaim**: not
"exactly the same problem," but the same *shape* with different *tractability*.
`map`'s batch of simultaneous demands is free/structural — known statically from the
list, no discovery needed. The general convergence case (independent bindings sharing
an ancestor, like `xy`) requires genuine dynamic graph-reachability discovery, and has
no natural single function to route the aggregated demands to unless the author
anticipated the coupling and wrote a joint unravel by hand.

**Proposed unification**: spec §11.4's existing `solver` annotation (attached to the
converging node itself, currently framed as "combine already-computed values, default
average") could be upgraded to the same joint-solving shape as many-unravel — "here
are N (target, context) pairs, jointly solve" instead of "average N values." Same
power, attached to the discovered convergence point rather than a statically-known
function. `map`'s version stays cheaper regardless, since it never pays for discovery.

---

## 9. Push-forward / constraint propagation (open thread — not fully resolved)

Proposal: instead of (or alongside) backtracking through a candidate list blindly,
when a conflict or unsolvable node is found, push a *derived/forced* value into a
neighboring branch and re-solve that branch specifically for the remaining unknowns,
cascading recursively as needed.

- **Identified as forward checking / constraint propagation** from the CSP
  literature (arc consistency, "maintaining arc consistency" search) — not a novel
  mechanism, a rediscovery of the standard fix for blind backtracking's weakness.
- **Concrete example showing it's more than an optimization** (`scaled`/`offset`/
  `double_scaled`, §-worked in discussion): a forced constraint from one branch
  (`double_scaled` forces `scaled = 22`) isn't necessarily among another branch's
  locally-generated candidates (`result`'s own guesses about how to split its delta
  across `scaled`/`offset`, generated in ignorance of the other constraint). Push-forward
  re-solves `result`'s equation *with* the forced value, directly deriving `offset`,
  rather than hoping a blind guess happens to coincide.
- **More aligned with exact-or-fail (§7) than backtracking is** — backtracking through
  uninformed candidates can produce false negatives (report "no solution" when one
  exists but wasn't among the pre-generated guesses); propagation derives the answer
  directly from the real constraint.
- **Composes with backtracking rather than replacing it**: propagate to a fixed point
  first, fall back to search only for genuine remaining ambiguity — standard CSP
  "propagation + backtracking" hybrid shape.
- **Open cost/design question**: pushing forward *between* nodes (pin an entire
  input, re-invoke a neighbor's whole unravel) is free/structural. Pushing forward
  *into* one hand-authored unravel's interior (pin one part of what it solves for,
  leave the rest free) needs the author to have structured it to accept that — free
  for primitive-op-composed unravels (already doing per-operand inversion), not free
  in general. Also: propagation cost isn't necessarily local/cheap in a densely
  connected graph — correct behavior, but a real cascading cost, not dismissed.

---

## 10. Signature evolution

In order, with what changed and why. Not all of these are "correct" — this is the
reasoning trail, including corrections.

1. **Pre-existing spec sketch** (from earlier notes, TODO.txt "7/27 unravel"):
   ```
   f_normal_unravel: out -> out -> a -> b -> c -> (a, b, c)
   ```
   (desired output, then original output, then original inputs → new inputs)

2. **Starting point for this discussion**:
   ```
   f_unravel :: Sensitivity b -> a -> a
   ```
   target_b, orig_a → new_a. First use of "target value" semantics over vaguer
   "sensitivity/delta" framing — validated as more general, since delta/subtraction
   isn't well-defined for arbitrary (non-numeric) types the way "target value" is.

3. **Added `orig_b` back in** (to give the unravel body a concrete anchor for
   "how much did the targeted fields change" and context for don't-care fields):
   ```
   f_unravel :: Sensitivity b -> b -> a -> a
   ```

4. **Multi-argument generalization of #3**:
   ```
   f_unravel :: Sensitivity out -> out -> a -> b -> c -> (a, b, c)
   ```

5. **Dropped `orig_b` again** — realized it's always cheaply available via the
   runtime's existing content-hash-keyed cache (from the node-identity-hashing design),
   since `f` is pure and `f orig_a` is already computed/cached as part of the ordinary
   forward pass. Back to the simpler:
   ```
   f_unravel :: Sensitivity b -> a -> a
   ```

6. **Added failure/multiple-candidates via `List`** (empty = no solution, ordered =
   preference, reusing `List`'s existing semantics rather than a separate `Result`
   type):
   ```
   f_unravel :: Sensitivity b -> a -> List a
   ```
   Multi-arg form:
   ```
   f_unravel :: Sensitivity out -> a -> b -> c -> List (a, b, c)
   ```

7. **Higher-order (`map`)**:
   ```
   map_unravel :: Sensitivity (List b) -> (a -> b) -> List a -> List (List a)
   ```

8. **Many-unravel** (§8, opt-in via something like `@manyunravel`):
   ```
   g_manyunravel :: List (Sensitivity q, p) -> params -> List params
   ```

9. **`Sensitivity` generalized** from `Option`-per-leaf to a richer constraint
   vocabulary (§7):
   ```
   type Sensitivity a = Exact a | Range a a | Tolerance a a | Free
   ```

10. **Current/latest**: replaced the returned candidate `List` with an *input*-side
    hints list (push-forward recommendations), collapsing the return to a single
    optional answer — restructures where search/multiplicity lives (moves from "every
    unravel enumerates its own guesses" to "the framework proposes informed hints,
    unravels just check them"):
    ```
    type alias UnravelInput a = { orig : a, hints : List a }

    f_unravel :: Sensitivity b -> UnravelInput a -> Option a
    ```
    Multi-arg form:
    ```
    f_unravel :: Sensitivity out -> UnravelInput a -> UnravelInput b -> UnravelInput c -> Option (a, b, c)
    ```

---

## 11. Materialization: making it a framework feature

Motivating case: a UI lets a user add a new object to a scene graph directly (e.g.
dropping a new Cube onto the canvas), which is a `List SceneObject` in source, built by
collecting individually-named object bindings. This turned out *not* to be `unravel` —
the new object's value is fully known to the UI the instant it's created, so there's no
unknown to solve for and `Sensitivity`/`UnravelInput` never enter the picture. What's
actually needed is a different operation:

1. **Serialize** the known value into Knot source via a canonical, always-reparseable
   structural walk — related to, but distinct from, an overridable `Show` instance
   (a user's hand-customized `show` might not round-trip; this needs the derivation-
   based version specifically).
2. **Mint identity**: a fresh, non-colliding top-level binding name, plus
   `@nodeId`/`@position` annotations synthesized from data the UI already has (a
   freshly-minted id, the canvas drop location) — giving the new object the same
   shape as any hand-authored one, individually annotatable/unravelable from here on.
3. **Locate** the target list via the node-identity/span tracking already required for
   UI-layout preservation and partial re-eval (no new discovery mechanism).
4. **Splice**: insert the new binding, append a reference to it.

Deletion is the natural mirror, same non-`unravel` mechanism.

**Revised: no interface needed at all — default-on, opt-out instead of opt-in.**
The first pass here proposed a `Growable` interface (`insert`/`remove` dispatched per
container type). That was solving a problem that doesn't exist: insert/remove on a
literal `[e1, e2, ...]` node are pure AST rewrites (permute/extend the `Vec`, reprint)
that need zero knowledge of the element type, so there's nothing to abstract over
per-type — the operation is uniform across *any* element type by construction, not
because a dictionary made it so. Given that, gating it behind an opt-in annotation adds
friction without buying anything: *any* literal list, anywhere in a program, is
materializable by default, no annotation needed to turn it on. The real gatekeeping was
never a language-level concern anyway — a UI only ever offers a materialize gesture for
things it actually renders as editable, so an always-on capability at the language
level is harmless; nothing invokes it unless the UI specifically decides to.

The annotation that's actually useful is the opposite — an explicit **opt-out**, for
the (rare) case where the default convenience behavior would be semantically wrong:
```knot
@nomaterial(())
sceneObjects :: List SceneObject
sceneObjects = [cube1, sphere1]
```
`@nomaterial` makes an *attempted* materialize/remove against that binding fail
cleanly rather than silently falling back to the default — consistent with
exact-or-fail everywhere else in this design. (`@nomaterial(())`, not a bare
`@nomaterial` — `knot-syntax`'s annotation grammar currently requires a parenthesized
argument after every key; a bare-flag form is a real, small, not-yet-made parser
change, not something to assume already works. `()` rather than `True`/`False` since
only presence is ever meaningful.)

Two genuinely separate reasons materialize can be unavailable, worth keeping distinct:
`@nomaterial` is "mechanically possible, author doesn't want it here"; a **type
constraint** — the element type needs a *derived* (not overridden) `Show` instance,
plain data, no embedded functions — is "mechanically impossible regardless of what the
author wants," checked automatically, a compile-time error instead of a runtime
surprise for something like a `List (Int -> Int)`, and makes the "no new code
synthesis" boundary (established repeatedly throughout this whole discussion) visible
as a type check rather than an algorithmic limitation nobody notices until they hit it.

**`Map` needs no separate treatment**: Knot has no `Map` literal syntax at all (spec
§2.2 — `Map.fromList [...]` is the only constructor), so a `Map` in source is always,
syntactically, a function call wrapping an ordinary `List (k, v)` literal.
Materializing "into the Map" is really materializing a new `(key, value)` tuple into
*that* underlying list literal — same mechanism, nothing Map-specific needed, no
interface required to make this generalize. **Tuples don't apply**: fixed arity means
"growing" one would change its type, not just its value — out of scope by construction,
different from what materialization is for; per-position value change within an
existing tuple is already ordinary unravel (nested `Sensitivity` recursion, §6),
nothing new needed there either.

See `unravel-examples.md`'s "Scene graph list append" entry for the full worked
example.

---

## 12. List diffing: identity vs. position

Once materialization existed for pure add/remove, the natural next question: what
about a list-shaped output whose elements *change value* in place, or get *reordered*,
with the length unchanged either way?

**Value change, same length**: already fully handled, no new mechanism. A list
literal's slots are ordinary expression positions — `Sensitivity (List b) =
List (Sensitivity b)` (§6) already routes a per-slot target backward through whatever
occupies that slot (a bare `Var` reference is trivially invertible), recursing into
that binding's own unravel. Nothing about being inside a list changes anything here.

**Reordering — where positional correspondence actually breaks**: `[cube1, sphere1]`
→ `[sphere1, cube1]`, same elements, same length, just swapped. Applying the *same*
positional logic here is wrong despite the length being unchanged: slot 0 (currently
`cube1`) would be told "become `sphere1`'s current value" and vice versa — that tries
to swap *identities*, not swap *positions*, semantically backwards and potentially a
spurious type error or a coincidentally-typechecking but meaningless result. Length
being unchanged is not, by itself, sufficient grounds for positional correspondence.

**The fix**: match old-list to new-list elements by *identity* (`nodeId`), not index —
the same "keyed diffing" approach React's virtual-DOM list reconciliation (and similar
UI frameworks) uses list keys for, for exactly this reason. Given key-based matching:
- new key, no old match → **materialize** (insert)
- old key, no new match → **remove**
- key in both, different index → **reorder**: pure source-text rearrangement of the
  list literal's existing element references, no solving — same "already fully known,
  nothing to solve for" character as materialization itself
- key in both, same/moved index, different value → **ordinary unravel**

These four classifications are independent, so mixed edits (add + reorder + one
value-change, all at once) decompose cleanly per-key rather than needing one uniform
diff strategy.

**Same revision as §11: no interface, default-on, opt-out.** Reorder needs *even less*
machinery than materialize — it never touches element values or types at all, purely
rearranging which existing sub-expression sits where in a literal `Expr::List` node, so
there was never anything to abstract over per-type in the first place. Default-on for
any recognized list literal; opt out explicitly:
```knot
@noreorder(())
```
same shape and same clean-failure guarantee as `@nomaterial` — an attempted reorder
against a `@noreorder`-marked list fails rather than silently proceeding. Kept as an
*independent* annotation from `@nomaterial` rather than combined: a fixed-size top-3
leaderboard might be reorderable but not growable; a tag bag might be growable with no
meaningful order at all; a list can disable neither, either, or both.

**Open, not resolved**: lists whose elements carry no identity at all have nothing to
match on — a value-based fallback (Myers/LCS-style, treating value-equality as the
match key) is strictly less precise (can't tell "coincidentally equal" from "same
thing"), so requiring `nodeId` for anything living in a list where reorder/materialize
might apply is probably better than maintaining two diffing strategies, but this hasn't
been decided.

See `unravel-examples.md`'s "List element value change" and "List reordering" entries.

---

## 13. Open threads / not yet resolved

- Push-forward's "pin part of a hand-authored unravel's interior" mechanism — needs
  its own opt-in shape, not designed yet (§9).
- Whether `solver` gets formally upgraded to many-unravel's joint-solving shape (§8) —
  proposed, not written into spec.
- `unravel`'s interaction with the type checker (dictionary-passing, per
  `knot-type-checker-plan.md`) — partially addressed: the plan now has a §3.5 covering
  the general mechanism (annotation-key → expected-type derivation table) with
  `unravel` as the worked case, but `@nomaterial`/`@noreorder` and the "element type
  needs a derived `Show` instance" constraint haven't been folded into that plan yet
  (no interface to add now that `Growable` was dropped — just two annotation-key
  entries and one type constraint).
- List-length changes are no longer a flat "out of scope" for `Sensitivity T` (§6) —
  split into materialize/remove and reorder (§§11–12) for the case where the UI
  already knows the concrete new/rearranged state, both resolved to default-on,
  opt-out (`@nomaterial`/`@noreorder`), no interface needed. Still genuinely open: a
  new element's parameters needing to be *derived* from existing scene state (not just
  a literal drop) would reintroduce real unravel-style solving on top of the
  materialization mechanism — not designed.
- `@noreorder`'s (and `@nomaterial`'s) fallback for elements with no identity to key on
  (§12) — flagged, not decided.
- `@nomaterial(())`/`@noreorder(())` needing an explicit `()` argument, rather than a
  bare `@nomaterial`, is a real current constraint of `knot-syntax`'s annotation
  grammar (`expect_byte(b'(')` is unconditional after every key) — a bare-flag parser
  addition would be a small, easy ergonomic win but hasn't been scoped or built.
- None of this has been reconciled with spec §11's own acknowledged incompleteness
  ("TODO review this section... needs more work") — this document is input to that
  future pass, not a replacement for it.
