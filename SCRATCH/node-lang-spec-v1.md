seems to be AI generated, but explains semantics and how it maps to nodes a bit more, 

you can probably delete this doc, 

nodegraphstart.md will explain node semastic,s
and langua-spec-notes shoud be built upn for syntax, I guess later we can add semantics to it
but three's also hella runtime stuff, so... anyways, we still don't need this doc.  ol combine with languae-specsnotes and nodegraphstart

# Node-Graph / Functional-Language Semantics — v1 Draft Spec

Status: working draft, first pass. Scope is intentionally narrow (see §0). This document
covers *semantics* — how the language and the node-graph UI relate to each other — not
concrete syntax details already settled elsewhere (Lisp-style `(declare ...)` / `(define ...)`,
prefix arrow types, etc.) or geometry/slicing-specific node types.

## 0. Scope for v1

- Monomorphic types only. No type variables, no generic containers, no user-defined
  typeclasses. (Revisit once the core language + engine are stable — see prior discussion
  on `Foldable`/typeclasses vs. a single built-in `Sequence` adapter interface.)
- No implicit lexical capture (§2). This is a deliberate simplification, not a permanent
  design commitment — see relaxation note in §2.3.
- Single output per function/node-group (§4.3). Multi-output deferred (§8).
- Recursion and higher-order functions proceed via the Definition/Callsite mechanism
  (§5) — not re-derived here.

## 1. Program Structure

- Every node graph corresponds to exactly one **`main`** function. `main` is the graph's
  entry point; the RTE (runtime/engine) invokes it to produce the graph's output.
- All other user-authored functions are ordinary top-level `declare`/`define` pairs,
  called directly or indirectly from `main`, or unreachable (dead code — a compile
  warning, not an error).
- `main`'s **output** maps to the exposed output connector(s) on the right edge of the
  node-graph canvas. v1: exactly one such connector.
- `main`'s **inputs** are *not* authored directly by the graph designer as ordinary
  function parameters. Instead, they are *derived*: they are exactly the set of RTE
  globals (§3) transitively used anywhere in the call graph reachable from `main`. The
  designer doesn't decide `main`'s signature up front — it falls out of what the graph
  actually uses.

## 2. Capture Rule (v1: no implicit capture)

### 2.1 Rule

Every free identifier referenced in a function's body must appear in that function's own
parameter list. A function may **not** silently read a value from an enclosing scope,
including its own enclosing function's parameters, sibling nodes outside its node-group
boundary, or RTE state — full stop. If a function needs a value from "outside," that
value must be an explicit parameter, and every caller must supply it (or itself have it
as a parameter, forwarded up the chain).

### 2.2 Consequence: global-threading

Since RTE globals are, by definition, values a function didn't create itself, the
capture rule forces them to be threaded explicitly:

- A function using a global declares a parameter tagged as sourced from that global
  (see §3.2 for tagging syntax).
- Every function that calls it, transitively, up to `main`, must also carry and forward
  that same value as an ordinary parameter — there is no special-cased injection
  mid-chain. `main` is the unique point where RTE-supplied values actually enter the
  system; everything below it is normal value-passing.

This is real, non-trivial authoring cost for deeply-nested call chains — a leaf function
needing "current settings" forces a pass-through parameter on every function between it
and `main`, even ones that don't otherwise care about settings at all.

### 2.3 Planned relaxation (v2 — not required for v1, designed to be forward-compatible)

v2 introduces auto-captured inputs: a value can be dragged through nested Definition-
node boundaries without manually re-wiring a pass-through parameter at each
intermediate level. This is **not** a v1 requirement — v1 ships with §2.1 enforced
strictly and no auto-capture at all — but the Definition-node mechanism (§5) should be
designed now so this slots in without a breaking change later.

Visual/structural design for v2 (draft, to be refined once implemented):

- Auto-captured lines are rendered in a **visually distinct color** from ordinary
  input wires, so a reader can immediately tell "this is a threaded/captured value,
  not something the caller at this level chose to supply."
- Auto-captured lines **penetrate the Definition node's boundary directly** rather
  than terminating on its parameter row alongside ordinary dangling inputs. That is,
  a captured value is drawn passing *through* the node's visual edge into the
  interior, distinct from parameters, which sit *on* the edge.
- Open sub-question (not yet resolved): since a captured line crosses the boundary
  at an arbitrary point rather than attaching to a fixed parameter slot, what happens
  when the Definition node is moved/resized/copy-pasted? Ordinary parameter rows
  reflow naturally with the node; a boundary-penetrating line needs its own rule for
  how it follows the node (or gets severed and must be reattached) under these
  operations. Work out once v2 design starts in earnest — flagged here so it isn't
  forgotten.
- Underlying representation is unaffected either way — even in v2, §2.1 still holds
  at the *language* level (there is no true hidden lexical capture in the semantics);
  auto-capture is purely the *editor* synthesizing the pass-through parameters and
  wires on the designer's behalf and rendering them differently so the plumbing
  doesn't visually clutter the graph.

## 3. Globals and Graph-Level Inputs

### 3.1 Two distinct mechanisms (resolved — not unified)

Globals (RTE-provided state) and designer-facing graph inputs are **separate**
mechanisms, not one:

- **Constants** — designer-facing graph inputs (geometry loaded on the plate, a
  user-exposed settings slider, a literal number/string/enum picked in the editor).
  These are natively supported by the node-graph editor itself, not by the
  global-threading machinery in §2/§3.2. Mechanically, a constant is either:
  - a dedicated node with outputs only (no inputs) — used for anything not trivially
    string-serializable (e.g. a geometry reference, a color swatch), or
  - inlined directly at the input point as a special-case editor widget (a number
    field, dropdown, etc. drawn right on the socket) for easily string-serializable
    types — no separate node needed on the canvas.
  The set of constant types is extensible (new constant kinds can be added later)
  but constants are always **leaves** — they have no inputs of their own and never
  need threading, since there's nothing upstream of them to thread through.
- **Globals** — RTE-provided state (current settings profile, build-plate state,
  printer profile) that is not something the designer picks per-node, but rather
  ambient context the engine supplies. These *do* go through the explicit-threading
  machinery in §2.1–§2.2, since a deeply-nested function reading a global has a real
  dependency that must be surfaced through every intervening caller up to `main`.

Practical distinction: a constant is something the *designer* placed on the canvas
as a value; a global is something the *RTE* makes available that a function author
opts into reading. `main`'s derived input set (§3.3) is built from globals only —
constants never appear there, since they're already fully resolved at their own node
and have nothing to forward.

### 3.2 Declaring a global-sourced parameter

Needs a marker in the parameter's type position distinguishing "this parameter is
ordinary — caller supplies it" from "this parameter is sourced from RTE global G, but
still must be explicitly threaded per §2.1" — e.g. (placeholder syntax, not final):

```lisp
(declare apply-fuzzy-skin (-> (global SettingsProfile) Geometry Geometry))
```

The `(global ...)` tag is documentation/bookkeeping, not a relaxation of §2.1 — the
parameter is still an ordinary explicit parameter that every caller must forward.

### 3.3 Deriving `main`'s inputs

`main`'s parameter list = the set of all distinct globals appearing (transitively,
via forwarding per §2.2) anywhere in the reachable call graph. This set maps 1-1 to
the graph's exposed input connectors on the left edge of the canvas. Unused globals
in unreachable/dead functions don't count.

## 4. Node Group ↔ Function Correspondence

### 4.1 Definition

A **node group** is a set of nodes plus the wires among them. Its unconnected inputs
and its externally-consumed outputs constitute its I/O.

### 4.2 Correspondence

Every user-authored function corresponds to exactly one node group, and vice versa.
`main` is itself a node group (the top-level canvas) like any other.

### 4.3 Function-node mechanism (overview)

A node group with **exactly one** output may be turned into a reusable, higher-order-
capable function via the Definition/Callsite mechanism (§5). Node groups with more
than one output cannot participate in this mechanism in v1 — the designer must
explicitly combine to a single output first (deferred properly in §8, per earlier
agreement to punt on multi-output for now).

## 5. Function Nodes: Definitions, Callsites, and Combined Forms

This section replaces the earlier informal term "box" with precise terminology, and
specifies the full state machine the editor needs to support.

### 5.1 Terminology

- **Definition node** — the extracted, canonical representation of a function. Its
  dangling inputs (at the moment of extraction) become its formal parameters. Exactly
  one output (§4.3). Two independent toggles live here (§5.2) — nowhere else.
- **Callsite node** — an instance/use of a Definition, created by dragging from a
  dedicated handle on the Definition node (distinct from the ordinary move-drag). A
  Callsite has no toggles of its own; it inherits behavior from the Definition it
  instantiates. Multiple Callsites may reference one Definition.
- **Combined node** — the default, merged visual presentation of a Definition with
  exactly one Callsite (§5.4) — the common case, since most functions are used once
  and showing separate indirection for that adds nothing.
- **Function value** — what a Higher-order-enabled Definition exposes: an output
  handle draggable into higher-order sockets, fresh Callsites elsewhere, or the
  Definition's own parameter row (recursion).

### 5.2 Definition node toggles

Settable only on Definition nodes (not on bare Callsites, which have none of their
own):

- **Reified-in-code** — ON: compiles to a named top-level `(declare ...)`/`(define
  ...)` pair, referenceable by name, listable in a reuse palette. OFF (default):
  compiles to an anonymous `(lambda (...) ...)`, inlined at its point of use in the
  generated text — appropriate for a Definition used exactly once that the designer
  never intended to name or reuse.
- **Higher-order-enabled** — ON: exposes the function-value output handle described
  above. OFF (default): the Definition can still be instantiated as ordinary
  Callsites, but is never itself draggable as a value — keeps the common case (a
  plain reusable function, called but never passed around) free of an unused socket.

### 5.3 Callsite creation

Dragging from a Definition node's dedicated instantiation handle produces a new
Callsite node on the canvas, with sockets matching the Definition's parameters and
output, wired into whatever surrounds it. This is distinct from ordinary node
movement (dragging the node body just repositions it).

### 5.4 Combined nodes and their connectivity states

If a Definition has exactly one Callsite, the editor shows them merged as a single
**Combined node** — no separate floating Definition is displayed. If a second
Callsite is later instantiated from the same Definition, the pair automatically
splits back into a visible standalone Definition plus its (now multiple) Callsites,
since a shared Definition can no longer be conflated with any one specific use.

A Combined node's state is determined by which of its sockets are wired:

| State | Condition | Meaning |
|---|---|---|
| **Direct-use** | all inputs connected, output connected | fully saturated ordinary use — no function abstraction is actually needed at runtime, but the grouping is kept for readability/collapse (§6) |
| **Incomplete** | ≥1 input disconnected **and** the output disconnected | eligible for conversion into a function value (§5.5), not yet resolved — a valid, persistent pending state (under-construction nodes are always allowed) |
| **Higher-order function node** | same connectivity as Incomplete, after explicit conversion | now a genuine function value — see §5.5 |

A Combined node with all inputs connected but its output *disconnected* is not
covered by the table above as a distinct case in v1 — treat as Incomplete (dead code
warning candidate, not a function-value candidate, since currying with zero
remaining parameters isn't meaningful). Flagged as a detail to confirm rather than
silently assumed.

### 5.5 Implicit Definition synthesis (currying via conversion)

Converting an Incomplete Combined node into a Higher-order function node implicitly
synthesizes a standalone Definition node — the designer never manually extracted one,
but the editor creates it as a side effect of the conversion:

- The synthesized Definition's formal parameters are exactly the previously-
  disconnected inputs, in their existing order (reorderable afterward as usual).
- The previously-*connected* inputs are **not** promoted to parameters — they are
  baked in as fixed constant values closed over by the synthesized Definition. This
  does not violate the no-capture rule (§2.1): a baked-in literal is not a reference
  to a sibling node or enclosing scope, it's a constant, structurally the same as the
  Constants described in §3.1.
- Reified-in-code defaults OFF on the synthesized Definition (anonymous lambda) —
  flip on manually if the designer wants to name and reuse this particular curried
  form.

Example, in generated text: original Definition `D : (-> A B C)`, with a Callsite
that has `A` wired to a constant `a0`, and both `B` and the output left disconnected,
converted to a function value — synthesizes:

```lisp
;; D already declared/defined elsewhere
(lambda (b) (D a0 b))
```

### 5.6 Recursion

Unchanged from prior agreement: recursion is the degenerate case of dragging a
Higher-order-enabled Definition's own function-value handle into its own parameter
row. Requires Higher-order-enabled = ON on that Definition (§5.2) — a Direct-use
Combined node has no function-value handle to drag in the first place, so recursion
implicitly forces that toggle on if it wasn't already.

Closures over sibling nodes/enclosing scope remain disallowed per §2.1 in v1 — a
Definition's inputs must resolve to either (a) its own formal parameters or (b) baked-
in constants per §5.5, never a live reference outside its boundary.

## 6. Visibility: Collapse

- A Definition node (§5.1) may be flagged **collapsed**, hiding its node-group
  internals in the graph UI and showing only its declared signature (name, parameter
  types, return type) at every Callsite.
- Open question (§8): is `collapse` a property of the Definition (uniform everywhere
  it's called) or overridable per Callsite?

## 7. Foreign/IO Escape Hatch

- A distinct top-level form for functions implemented outside the DSL — calling into
  host code (Rust), doing real I/O (submitting gcode to a printer, reading machine
  state), or wrapping host libraries. Modeled after the Coalton `lisp` embedding
  pattern discussed earlier: a declared signature in the DSL's type system, with an
  opaque host-language body.
- These are **always** collapsed — not a UI choice but a structural necessity, since
  there is no DSL-level node graph inside them to show.
- The capture rule (§2) still applies to their *declared* signature — no hidden inputs
  — but the engine cannot verify purity of their *implementation*. See open question
  in §8 re: caching/memoization safety.

## 8. Open Questions

1. ~~Global/graph-input unification~~ — **resolved**: constants (designer-facing) and
   globals (RTE-provided) are separate mechanisms. See §3.1.
2. **Multi-output functions/Definition nodes** — deferred. When revisited: tuple/
   struct return type, or side-step it via "named multiple entry points" analogous
   to `main` (e.g. `main-gcode`, `main-preview`) instead of a single `main` with a
   compound output?
3. **Per-Callsite collapse override** — can an otherwise-collapsed Definition be
   expanded at one specific Callsite without affecting other Callsites, or is
   collapse strictly a per-Definition flag?
4. **Can `main` recurse?** Recommend: disallow — `main` has no caller and is invoked
   once per graph run by the RTE, so self-reference isn't obviously meaningful. Confirm
   this is fine, or identify a use case (e.g. iterative re-slicing) that needs it.
5. **Foreign-node purity annotation** — a foreign/IO function may or may not be safe to
   memoize (a pure FFI math call vs. "submit gcode to a physical printer," which must
   never be cached/replayed). Needs an explicit annotation the Salsa-style caching
   layer can key off, since the engine can't infer this from a DSL-invisible body.
6. **Visual treatment of pass-through/global-forwarding parameters** — should these be
   visually de-emphasized on a Definition's parameter row (vs. "real" parameters a
   caller actually chooses to supply), so deep threading chains (§2.2) don't visually
   clutter every intermediate Definition in the chain?
7. **All-inputs-connected-but-output-disconnected Combined nodes (§5.4)** — currently
   treated as Incomplete/dead-code rather than a function-value candidate, since there
   would be zero remaining parameters to curry. Confirm this is the intended behavior
   rather than a case that should still support becoming a zero-argument function value
   (a thunk) for deferred/lazy evaluation of an otherwise fully-saturated computation.
