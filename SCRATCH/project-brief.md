# Project Brief: Node-Graph-Native 3D Print Slicer (Rust)

Read this alongside `node-lang-spec-v1.md` (detailed language/UI semantics). This
brief covers the surrounding architecture decisions that spec doesn't repeat.

## What this is

A from-scratch 3D print slicer, written in Rust, using Orca Slicer's algorithms as a
reference (not a fork — reimplemented). The differentiator: slicing pipelines are
authored as a visual node graph that has a genuine 1-1 correspondence with an
underlying Lisp-syntax functional language — not a graph that merely compiles *to*
code one-way, but a dual representation editable from either side.

## Core architecture decisions

- **Language**: custom Lisp-syntax DSL. Prefix arrow types for signatures, `(declare
  name (-> A B C))` / `(define (name a b) ...)`, following the Coalton/Typed-Racket
  convention rather than inventing new type-signature syntax.
- **Scope discipline**: DSL is composition glue only. Node *bodies* (geometry
  algorithms — perimeter generation, fill, support, path planning) are native Rust,
  ported from Orca's C++ as reference. The DSL never implements low-level geometry
  math itself.
- **Incrementality**: Rust's `salsa` crate (the same incremental-computation engine
  rust-analyzer uses) for "only recompute what depends on what changed." Node
  evaluation is **strict**, not lazy — incrementality comes from the Salsa
  dependency-tracked cache layer, not from language-level laziness. This was a
  deliberate choice after checking how Enso (a comparable dual visual/textual
  functional language) does it: Enso is strict-by-default with opt-in laziness, and
  gets its reactive-recompute feel from runtime caching, not from a lazy core.
- **Persistent/immutable data**: `im`/`rpds`-style structural sharing for graph-level
  values (layer lists etc.); the geometry kernel itself (triangle meshes, booleans)
  stays mutable/arena-based for performance — likely built on `parry3d`/`nalgebra`
  rather than reinvented.
- **UI**: node graph editor — Rete.js or React Flow under consideration, likely via
  Tauri (Rust core + web-tech frontend) rather than pure in-browser WASM, since
  mesh/slicing compute at scale is rough in-browser.
- **Type system**: monomorphic only for v1. No generics/typeclasses yet — deferred
  per the note in `node-lang-spec-v1.md` §0. When revisited, lean toward one
  built-in `Sequence`/`Iterable` adapter interface (Rust's `IntoIterator` pattern)
  rather than full Haskell-style user-extensible typeclasses, to avoid building a
  constraint-solving/dictionary-passing subsystem for what's mostly "let one `Map`
  node work over several concrete container types."

## Higher-order functions, recursion, and the box/Definition mechanism

This was the single hardest design problem and got the most iteration — full detail
lives in `node-lang-spec-v1.md`. Summary of the resolved design:

- Boxing a closed node selection is the **sole** mechanism for function values, reuse,
  partial application, *and* recursion — no separate "Call self" node type needed;
  recursion is the degenerate case of dragging a function's own value-handle into its
  own parameter row.
- Terminology: **Definition node** (canonical function, has the two toggles
  Reified-in-code / Higher-order-enabled), **Callsite node** (an instance of one),
  **Combined node** (default merged single-use presentation, with Direct-use /
  Incomplete / Higher-order-function-node states based on socket connectivity).
- **No implicit lexical capture in v1** — every free identifier must be an explicit
  parameter, threaded by hand through every intervening caller. This is real
  authoring cost (see `node-lang-spec-v1.md` §2.2) and is explicitly meant to be
  relaxed in v2 via auto-threaded/auto-captured inputs, rendered with a distinct wire
  color that penetrates the Definition's boundary rather than landing on its
  parameter row.
- **Constants** (designer-placed values — geometry on the plate, settings sliders)
  and **globals** (RTE-ambient state) are two separate mechanisms, not one — see
  `node-lang-spec-v1.md` §3.1. Only globals go through the explicit-threading rule.
- `main` is the implicit top-level function/canvas; its inputs are *derived* from
  which globals are transitively used, not authored directly.

## Recommended next steps for implementation

1. Toy prototype *before* touching the slicer: a minimal Salsa-backed evaluator for
   just the Definition/Callsite/recursion mechanics (factorial, then Fibonacci per
   the worked examples earlier in this project's history) — validate the UI/semantics
   actually feel right and that memoization composes correctly with recursion before
   investing in the real geometry pipeline.
2. Separately, a minimal end-to-end slicer MVP: mesh input → planar slice → basic
   perimeter → basic infill → gcode, as native Rust nodes wired via the DSL, proving
   the Salsa recompute loop feels right on real geometry.
3. Defer: non-planar slicing, interlocking layers, flexies, multi-output functions,
   auto-capture (v2), and generic containers/typeclasses — all explicitly out of scope
   for v1 per the notes above and in the spec.

## Open questions carried over (see `node-lang-spec-v1.md` §8 for full list)

Multi-output handling, per-Callsite collapse override, whether `main` can recurse,
foreign/IO node memoization-safety annotation, visual treatment of pass-through
parameters, and the all-inputs-connected-but-output-disconnected Combined-node case.
