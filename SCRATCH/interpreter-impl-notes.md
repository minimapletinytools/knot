auto generated interprteer note,se you can probbaly delet this file 


# Knot Interpreter — Implementation Notes

Language: Rust. Performance is not a primary concern for v0.
Key requirements: lazy evaluation (call-by-need) and caching.

---

## Major Pieces (roughly in dependency order)

### 1. Lexer
Turns raw source text into tokens — identifiers, keywords, operators, literals,
indentation markers, comments. Fairly mechanical.

### 2. Parser → AST
Turns token stream into a syntax tree. Main complexity:
- Operator precedence
- Haskell-style indentation-sensitive layout
- `@annotation` prefix (on declarations) and postfix (inline) forms

### 3. Desugarer (AST → Core)
Simplifies surface syntax into a small core language before evaluation. Desugared forms:
- `do` notation → nested `bind`/`pure` calls
- `|>` pipes → function application
- Pattern matching in function arguments → explicit `case`
- Record dot access → field projection functions

After desugaring, only a handful of core forms remain: `case`, `let`, `lambda`,
`apply`, literals. Everything else is gone.

### 4. Type Checker
Hindley-Milner inference — the standard algorithm for ML-family languages.

Handles:
- Let-polymorphism
- ADT constructor types
- Record types
- Extensible record row unification (`{ r | x : Float }` — the hardest part, requires
  row variable unification on top of standard HM)
- Interface constraints (`Eq a`, `Ord a` etc.) — simplified by the closed/built-in
  interface set; no full typeclass dictionary-passing needed

This is the hardest single piece to implement correctly.

### 5. Value Representation
How values exist at runtime. Core types:

```
Value =
  | Thunk(unevaluated: Closure)     -- suspended, not yet computed
  | WHNF(head: HeadForm)            -- computed to weak head normal form

HeadForm =
  | Int(i64)
  | Float(f64)
  | Bool(bool)
  | String(String)
  | Constructor(tag, Vec<Value>)    -- ADT variant
  | Closure(params, body, env)      -- function value
  | Record(Map<String, Value>)
```

Thunks require interior mutability in Rust — `Rc<RefCell<ThunkState>>` — because
forcing a thunk mutates it in place to store the result. This mutation is what gives
call-by-need its sharing property (a thunk is never evaluated twice).

### 6. Lazy Evaluator (thunk engine)
The core of the runtime. Two operations:

- **`force(thunk)`** — evaluate a thunk to WHNF, update it in place with the result
- **`eval(expr, env)`** — walk the core AST, building thunks for sub-expressions
  rather than evaluating them eagerly

Key discipline: only force a thunk when you actually need its value — when pattern
matching on a case, applying a function, or doing arithmetic. Everything else stays
as a suspended thunk.

### 7. Pattern Match Compiler
Compiles `case` expressions into decision trees. Handles nested patterns, wildcards,
constructors, record patterns. Output is a tree of "test the tag/field of this value,
then branch."

### 8. Environments / Closures
Functions capture their lexical environment at creation time as a persistent map from
name → `Value`. In a lazy language, recursive bindings work naturally — the environment
can contain thunks that reference the environment itself.

### 9. Built-ins
Arithmetic, comparison, string ops, `List`/`Map`/`Option`/`Result` operations, `show`,
`<>`. Implemented as Rust functions registered in the evaluator — not written in Knot.

### 10. Caching Layer
Two distinct levels:

- **Thunk sharing** — free from the lazy evaluator. Within one evaluation run, the
  same thunk is never computed twice.
- **Cross-run memoization** — for the node graph use case: change one input, don't
  recompute everything. A separate cache keyed on `(function_id, hash(inputs)) →
  output_value`. Pure functions only — IO nodes are never cached.

The `salsa` crate implements cross-run caching with full dependency tracking (knows
*which* inputs changed, not just that something changed). Use it directly rather than
reimplementing. Salsa caches at function-call granularity; laziness handles sharing
within a call. The two are complementary.

### 11. IO Runner
`IO a` values are pure descriptions of side effects — just data until the runtime
executes them. The runner is a small interpreter over `IO` actions that executes them
sequentially. Only invoked from `main`.

### 12. Error / Partial Function Handling
Runtime errors from partial functions (unmatched patterns, explicit `error`). Since
partiality is allowed, these should produce clean errors with source locations rather
than panics.

---

## Garbage Collection

In Rust, you get most of GC for free depending on value representation.

**Using `Rc<RefCell<...>>`** — Rust's `Rc` is reference-counting and handles most
allocation automatically. Values are freed when ref count hits zero. No separate GC
to write.

Gap: **reference cycles**. Lazy languages create cycles naturally (recursive `let`
bindings create thunks that reference themselves). `Rc` leaks cycles. Options:
- `Weak<>` references for back-edges — you designate which refs are owning vs weak
  at construction time. Requires discipline but no extra dependency.
- A cycle-collecting crate (`bacon-rajan-cc`) — drop-in replacement for `Rc` that
  handles cycles. Easiest fix if leaks become a problem.

**Using an arena allocator** — allocate all values into a typed arena (`bumpalo`),
use indices instead of pointers. GC is just "clear the arena" at the end of an
evaluation run. Fast and simple, but values that need to survive across runs (salsa
cache entries) need separate treatment.

**Recommendation for v0:** start with `Rc<RefCell<>>`. The cycle problem is manageable
since recursive knots are structured and you know where cycles can form. Swap to a
cycle-collecting crate if leaks become a real problem.

---

## Suggested Build Order

1. Lexer + Parser → get programs into memory
2. Desugarer → simplify before building the evaluator
3. Tree-walking evaluator, **eager first** — get correctness before adding laziness
4. Add thunk wrapping → swap eager eval for lazy thunk-building
5. Type checker — can be added after evaluator works (dynamic typing first, then typed)
6. Built-ins + IO runner
7. Caching layer last — plug salsa in once evaluation is correct

A tree-walking interpreter (walk the AST directly) is the right approach given that
performance is not a priority. A bytecode VM would be faster but significantly more
implementation work for no benefit here.
