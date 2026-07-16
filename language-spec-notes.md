# Knot Language Spec — v0.1

Knot is a lazy, pure functional language in the ML/Haskell family, designed to be the
textual representation of a visual node graph (Tangle). Every language feature must have
a clear, unambiguous node-graph representation — this constraint shapes every design
decision below.

---

## 1. Design Principles

- **Lazy evaluation (call-by-need)** — values are not computed until required. This is
  load-bearing: it enables natural expression of the node graph's deferred-computation
  model, infinite/co-recursive structures, and efficient composition of transformations.
- **Pure functional** — no mutable state. "Updates" always produce new values.
- **Immutable values** — structural sharing for efficiency where needed (runtime concern,
  not language-level).
- **Partial functions allowed** — `error`, `undefined`, and inexhaustive patterns are
  permitted. Compiler warns on incomplete matches but does not error.
- Syntax leans Haskell with selective borrowings from Elm (record syntax, `type` keyword,
  `let`-only scoping, pipe operators).

---

## 2. Relationship to Haskell & Elm

Knot represents a middle ground between Haskell and Elm, tailored specifically to visual node graphs.

### 2.1 From Haskell
* **Lazy evaluation (call-by-need)**: Values are computed only when demanded (unlike Elm's strict evaluation). This allows for infinite structures and deferred graph computation.
* **Built-in Interfaces**: Overloaded operations use a typeclass-like interface system (e.g., `Eq`, `Ord`, `Num`, `Integral`, `Fractional`).
* **Type Signatures**: Standard `::` syntax.
* **Spaced Dot Composition**: Spaced dot (`f . g`) denotes standard function composition.
* **Monadic/List Operators**: Operators like bind (`>>=`) and list cons (`::`) are preserved.

### 2.2 From Elm
* **Record Syntax**: `{ field = value }` for construction, `{ record | field = value }` for updates, and dot access without spaces (`record.field`).
* **Type Declarations**: ADTs are defined using the `type` keyword instead of `data`, and aliases use `type alias`.
* **Module & Import System**: Every file is a module, imports are qualified by default, and namespace control is managed via the `exposing` keyword.
* **No `where` clauses**: All local scope bindings must use `let...in` (matching Elm's let-only scoping).
* **Pipe Operators**: Forward pipe (`|>`) and forward composition (`>>`) are preferred for chaining operations.
* **No Map Literals**: Maps are constructed using `Map.fromList` (matching Elm's `Dict.fromList`).
* **Simplified Imports**: No `qualified` or `hiding` keywords (matching Elm's import design).

### 2.3 Omitted from Haskell/Elm
* **No user-defined interfaces/instances**: Unlike Haskell's typeclasses (or Elm's extensible record polymorphism), the interface set in v0 is closed to keep compile-time dictionary passing and type checking simple.
* **No custom symbolic operators**: Unlike both Haskell and Elm, users cannot define new operators (e.g., `+++`), ensuring 1-to-1 parsing and node-graph mapping remain clean.

### 2.4 Different / Custom
* **Closed Interface Instances**: Unlike Haskell, interface instances are pre-defined by the compiler for core primitive types.
* **Metadata & Annotations (`@name(...)` / `@{...}`)**: A compiler-checked layout and tool annotation system designed for graph coordinates, documentation, and metadata.
* **Unravel System (Reverse Execution)**: Backwards execution solving rule annotations (`unravel`), allowing changes to flow in reverse through the graph.

---

## 3. Syntax Conventions

- Type signatures use `::` (Haskell style)
- `type` keyword for ADTs (Elm style, not `data`)
- `type alias` for type aliases
- `let...in` for all local bindings — **no `where` clauses**
- Lambdas: `\x -> expr`
- Line comments: `--`
- Block comments: `{- -}`
- **Dot access**: `record.field` with no spaces = field access;
  `f . g` with spaces = function composition operator (unchanged from Haskell)

---

## 4. Type System

### 3.1 Primitive Types

| Type     | Description           |
|----------|-----------------------|
| `Bool`   | `True` / `False`      |
| `Int`    | Integer               |
| `Float`  | 64-bit floating point |
| `String` | UTF-8 text            |
| `Unit`   | `()` — the unit type  |

### 3.2 Unit-Aware Numeric Types

Deferred — not in v0. All physical quantities represented as `Float` for now.

### 3.3 Records

Elm-style record syntax throughout.

```knot
-- type alias
type alias Point = { x : Float, y : Float }

-- construction
{ x = 1.0, y = 2.0 }

-- field access (dot notation, no spaces around dot)
point.x

-- update (always produces a new record)
{ point | x = 3.0 }
{ point | x = 3.0, y = 4.0 }
```

### 3.4 Extensible Record Types

A type variable in record position means "any record with at least these fields":

```knot
distance :: { r | x : Float, y : Float } -> Float
```

Allows ad-hoc polymorphism over records without requiring full typeclasses.

### 3.5 Algebraic Data Types

```knot
type Shape
  = Circle Float
  | Rectangle Float Float
  | Triangle Float Float Float

type Option a
  = Some a
  | None

type Result e a
  = Ok a
  | Err e
```

### 3.6 Generic Containers (Built-in)

| Type         | Description                               |
|--------------|-------------------------------------------|
| `List a`     | Ordered sequence                          |
| `Map k v`    | Key-value map (`k` must implement `Ord`)  |
| `Option a`   | Nullable / missing value                  |
| `Result e a` | Success or failure with a typed error     |

### 3.7 Function Types

`a -> b -> c` — standard curried function type. All functions are curried by default.

### 3.8 Tuple Types

Fixed-size collections of values of potentially different types.

```knot
-- construction
pair :: (Int, String)
pair = (42, "hello")

triple :: (Float, Float, Float)
triple = (1.0, 2.0, 3.0)

-- pattern matching
case pair of
  (x, y) -> x
```

---

## 5. Expressions

### 4.1 Function Application

Standard prefix application: `f x y`. Left-associative.

### 4.2 Pipe Operators

```knot
-- forward pipe: passes left value as argument to right function
x |> f |> g

-- forward composition: produces a new function (f then g)
f >> g       -- equivalent to \x -> g (f x)
```

`|>` and `>>` are the preferred style for chaining. Haskell's `.` (backward composition)
is still available but `>>` is preferred for readability.

### 4.3 Let Bindings

```knot
let
  radius = 5.0
  area   = 3.14159 * radius * radius
in
  area
```

All local bindings use `let...in`. No `where`.

### 4.4 Lambdas

```knot
\x -> x + 1
\x y -> x + y
```

### 4.5 Conditionals

```knot
if condition then valueA else valueB
```

Always an expression. Both branches must have the same type.

### 4.6 Pattern Matching

```knot
case shape of
  Circle r       -> 3.14159 * r * r
  Rectangle w h  -> w * h
  Triangle a b c -> ...
```

Incomplete matches produce a compiler warning (not an error — partial functions allowed).

### 4.7 List & Map Literals

#### List Literals
```knot
[1, 2, 3]
```

#### Map Construction
Following Elm, Knot does not define a custom map literal syntax. Instead, maps are constructed from a list of tuple key-value pairs using the built-in `Map.fromList` function:

```knot
myMap :: Map String Int
myMap = Map.fromList [ ("apple", 1), ("banana", 2) ]
```

### 4.8 Built-in Operators & Precedence

To ensure unambiguous parsing and 1-to-1 visual node graph mapping, Knot defines a strict set of built-in operators and precedence rules:

| Precedence | Operators | Associativity | Description |
|:---|:---|:---|:---|
| 9 | `.` | Left | Function composition |
| 8 | `^` | Right | Exponentiation |
| 7 | `*`, `/`, `div`, `mod` | Left | Multiplication, division, modulo |
| 6 | `+`, `-`, `<>` | Left | Addition, subtraction, semigroup append |
| 5 | `::` | Right | List construction (cons) |
| 4 | `==`, `/=`, `<`, `<=`, `>`, `>=` | None | Comparison |
| 3 | `&&` | Right | Logical AND (lazy evaluation) |
| 2 | `\|\|` | Right | Logical OR (lazy evaluation) |
| 1 | `\|>`, `>>` | Left | Forward pipe, forward composition |
| 0 | `$` | Right | Function application |

#### Boolean Operators
- `(&&) :: Bool -> Bool -> Bool` — Logical AND. Short-circuits (lazy in its second argument).
- `(||) :: Bool -> Bool -> Bool` — Logical OR. Short-circuits (lazy in its second argument).
- `not  :: Bool -> Bool` — Logical negation (prefix function).

---

## 6. Definitions

```knot
-- type signature
name :: Type

-- function definition
name arg1 arg2 = expr

-- with let
name arg1 =
  let
    helper = ...
  in
    helper arg1
```

---

## 7. Interfaces (Built-in, Closed in v0)

The interface set is fixed — user-defined interfaces are not supported in v0 (see §13
for the v2 plan). Only built-in types have instances.

### 6.1 Core Interfaces

| Interface     | Key Operations                                   |
|---------------|--------------------------------------------------|
| `Eq a`        | `(==) :: a -> a -> Bool`                        |
| `Ord a`       | `compare :: a -> a -> Ordering` (implies `Eq`)  |
| `Show a`      | `show :: a -> String`                           |
| `Semigroup a` | `(<>) :: a -> a -> a`                           |
| `Monoid a`    | `empty :: a` (implies `Semigroup`)              |

The `Ordering` ADT is defined as:
```knot
type Ordering
  = LT
  | EQ
  | GT
```

### 6.2 Numeric Interfaces

Following Haskell's design, Knot uses interfaces to support overloaded arithmetic operations on `Int` and `Float`:

```knot
-- Basic numeric operations
interface Num a where
  (+)    :: a -> a -> a
  (-)    :: a -> a -> a
  (*)    :: a -> a -> a
  negate :: a -> a
  abs    :: a -> a
  signum :: a -> a

-- Fractional types (Float)
interface Num a => Fractional a where
  (/)    :: a -> a -> a
  recip  :: a -> a

-- Integral types (Int)
interface (Num a, Ord a) => Integral a where
  div    :: a -> a -> a
  mod    :: a -> a -> a
```

Instances are built-in for:
- `Num Int` and `Num Float`
- `Integral Int`
- `Fractional Float`

#### Conversion Helpers
- `fromIntegral :: (Integral a, Num b) => a -> b`
  Converts an integral value (e.g. `Int`) to any other numeric type (e.g. `Float`).

### 6.3 Collection Interface 

Implemented by `List` and `Map`:

```knot
map    :: (a -> b) -> f a -> f b
foldl  :: (b -> a -> b) -> b -> f a -> b
foldr  :: (a -> b -> b) -> b -> f a -> b
filter :: (a -> Bool) -> f a -> f a
length :: f a -> Int
```

Consider doing functor, foldable, and doing interface hierarchy, but the above is fine for V1

### 6.4 Context Interface (Monadic Chaining)

```knot
pure :: a -> f a
bind :: f a -> (a -> f b) -> f b   -- also exposed as (>>=)
```

Built-in instances: `Option`, `Result`, `IO`, `List`.

---

## 8. Built-in Monads & Do-Notation

| Type         | Purpose                                      |
|--------------|----------------------------------------------|
| `IO a`       | Side effects (file I/O, printer comms, etc.) |
| `Option a`   | Nullable / missing values                    |
| `Result e a` | Fallible computations with typed errors      |

Do-notation desugars to `bind`/`>>=` and `pure`:

```knot
do
  x <- someOption
  y <- anotherOption
  pure (x + y)
```

---

## 9. Modules & Imports

Knot adopts Elm-style module and import syntax. Every file defines a single module.

### 8.1 Module Declaration

The module header specifies the module name and the list of exposed types, ADT variants, and functions:

```knot
module Geometry exposing (Point, distance, Shape(..))
```

Exposing everything in the module:
```knot
module Geometry exposing (..)
```

### 8.2 Imports

Imports are **qualified by default** to prevent namespace pollution:

```knot
import List
import Map as M
import String exposing (length, concat)
```

- `import List` imports the module `List`. Functions must be qualified: `List.map`.
- `import Map as M` imports the module `Map` with an alias. Functions are accessed via `M.lookup`.
- `import String exposing (length, concat)` brings `length` and `concat` directly into the local scope, while keeping other functions qualified (e.g. `String.reverse`).

---

## 10. Metadata & Annotations

Annotations can be attached to any named binding. They carry node graph layout metadata,
stable IDs, reverse execution logic, documentation, and other tooling hints. They have
no effect on forward runtime semantics.

Annotations are evaluated at **graph construction time** — after type-checking but
before the graph runs. This means annotation values can be full Knot expressions
(including function references and conditionals), and all annotation fields are
type-checked like any other Knot code.

Two syntaxes are supported and can be freely mixed:

### 8.1 Stacked single-key form: `@name(args)`

Placed on the line(s) immediately above a top-level definition or a `let` binding.
Good for simple scalar annotations:

```knot
@nodeId("f1")
@position(100, 200)
@label("My Function")
myFunc :: Int -> Int
myFunc x = x + 1

let
  @nodeId("n1")
  @position(150, 300)
  result = myFunc 42
in result
```

Multiple `@` lines stack — all apply to the binding that follows.

### 8.2 Block form: `@{ ... }`

A single annotation block containing a record expression. Any valid Knot expression
is allowed as a field value — function references, lambdas, conditionals, let bindings.
Good for complex annotations or when many keys are needed at once:

```knot
@{
  nodeId   = "f1",
  position = (100.0, 200.0),
  label    = "My Function",
  color    = "#a0c4ff"
}
myFunc :: Int -> Int
myFunc x = x + 1
```

Both forms can be mixed on the same binding — stacked `@name` lines and a `@{ }` block
are merged, with the block taking precedence on any key that appears in both:

```knot
@nodeId("f1")
@position(100, 200)
@{ color = "#a0c4ff", unravel = myCustomUnraveler }
myFunc :: Int -> Int
myFunc x = x + 1
```

### 8.3 Inline sub-expression annotations

For annotating individual stages of a pipeline, `@annotation` is written **postfix**,
immediately after the expression atom it targets. Both forms work inline:

```knot
x = f @nodeId("n1") @position(100, 200)
  |> g @{ nodeId = "n2", position = (200.0, 200.0) }
  |> h
```

For complex sub-expressions wrap in parens first:

```knot
x = (f a b) @{ nodeId = "n1", position = (100.0, 200.0) }
  |> g
```

Prefer extracting to named `let` bindings when inline annotations get unwieldy.

### 8.4 Standard annotation keys

| Key | Type | Meaning |
|---|---|---|
| `nodeId` | `String` | Stable unique ID — persists across edits |
| `position` | `(Float, Float)` | Canvas position |
| `label` | `String` | Display name override |
| `doc` | `String` | Documentation string shown in graph UI |
| `color` | `String` | Node color (hex) |
| `group` | `String` | Visual group/cluster |
| `collapsed` | `Bool` | Whether node renders collapsed by default |
| `unravel` | `Unraveler` | Reverse execution function — see §11 |

The annotation set is open — new keys can be added without changing the language.

---

## 11. Unravel (Reverse Execution)

Every binding in the node graph can optionally carry an **unravel function** — a
reverse execution rule that, given a desired change in a node's output, computes the
corresponding desired changes to its inputs. This is what enables the graph to be
driven backwards: change a visualized output, propagate the change upstream to find
which input values (typically literals) to modify.

### 9.1 What an unravel function is

An unravel function is a regular Knot function attached to a binding via the `unravel`
annotation key. Its signature mirrors the forward function but runs in reverse:

```knot
-- forward:  inputs -> output
-- unravel:  (inputs, output_sensitivity) -> input_sensitivities

@{ unravel = \inputs sensitivity -> ... }
x = f y z
```

The runtime calls a node's unravel during a backward pass through the graph, passing
the desired output change (sensitivity) and the original inputs. The unravel returns
desired changes for each input, which are then propagated further upstream.

### 9.2 Default unravelers

Built-in operations have sensible default unravelers — no annotation needed for simple
cases. For example, addition splits the output sensitivity evenly across its inputs by
default. The annotation only needs to appear when overriding the default behavior.

### 9.3 Unravel on higher-order functions

When a node takes a function as input (e.g. `foldl`, `map`), its unravel receives not
just the function value but also that function's own unravel, and calls it during the
backward pass. Function strands in the graph carry their unravel bundled alongside
their forward implementation — passing a function to a higher-order node automatically
makes its unravel available for the backward pass.

### 9.4 Conflict resolution

When multiple downstream paths converge on the same node during a backward pass, that
node may receive conflicting desired values from different paths. The default strategy
is to average them; this is configurable per-node via a `solver` annotation key
(distinct from `unravel`). If the system cannot find a stable solution within a set
number of iterations, it surfaces a warning rather than silently producing a wrong
result.

### 9.5 Annotation examples

```knot
-- simple override: assign full delta to first argument instead of splitting
@{ unravel = \(x, y) delta -> (delta, 0.0) }
sum = x + y

-- conditional strategy based on input values
@{
  unravel = \inputs sensitivity ->
    if isLinear inputs
    then leastNormUnravel inputs sensitivity
    else iterativeUnravel inputs sensitivity
}
result = complexTransform input

-- reference a named unravel function defined elsewhere
@{ unravel = myDomainUnraveler }
output = domainSpecificOp input
```

---

## 12. Open Questions

1. **Metadata/annotation syntax** — TBD.
2. **Logging / observable side effects** — what's the story for debug output or
   structured logging within `IO`? Needs user stories before designing.

---

## 13. Planned for v2

- **User-defined interfaces** — allow users to declare their own interfaces and implement
  them for custom types. Requires a constraint-solving/dictionary-passing subsystem;
  deferred to keep the v0 type system tractable.
- **Unit-aware numeric types** — `Length`, `Speed`, `Temperature`, etc. as distinct types
  with homogeneous `+`/`-` and explicit scaling functions. Deferred until the core
  language is stable.
