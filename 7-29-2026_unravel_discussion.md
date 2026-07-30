# Unravel Design Discussion — 2026-07-29

A forked side-conversation working through `unravel` (reverse execution / pullback)
design, starting from research into Sketch-n-Sketch (the closest real academic prior
art) and ending at a fairly concrete signature proposal plus a live open question
(push-forward / constraint propagation). Nothing here is committed to the spec yet —
this is a discussion log to work from, not a decision record.

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

## 11. Open threads / not yet resolved

- Push-forward's "pin part of a hand-authored unravel's interior" mechanism — needs
  its own opt-in shape, not designed yet (§9).
- `List T`'s `Sensitivity` doesn't cleanly handle length changes (§6) — genuinely out
  of scope, or needs its own mechanism later.
- Whether `solver` gets formally upgraded to many-unravel's joint-solving shape (§8) —
  proposed, not written into spec.
- `unravel`'s interaction with the type checker (dictionary-passing, per
  `knot-type-checker-plan.md`) hasn't been discussed at all yet — `Sensitivity T`
  as a type-level transform presumably needs to exist inside that system somehow.
- None of this has been reconciled with spec §11's own acknowledged incompleteness
  ("TODO review this section... needs more work") — this document is input to that
  future pass, not a replacement for it.
