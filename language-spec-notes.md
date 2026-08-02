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
* **Monadic/List Operators**: Operators like bind (`>>=`) and list cons (`:`) are preserved.
* **Holes**: `_` and `_somename` can be used in place of expressions as holes

### 2.2 From Elm
* **Record Syntax**: `{ field = value }` for construction, `{ record | field = value }` for updates, and dot access without spaces (`record.field`).
* **Type Declarations**: ADTs are defined using the `type` keyword instead of `data`, and aliases use `type alias`.
* **Module & Import System**: Every file is a module, imports are qualified by default, and namespace control is managed via the `exposing` keyword.
* **No `where` clauses**: All local scope bindings must use `let...in` (matching Elm's let-only scoping).
* **Pipe Operators**: Forward pipe (`|>`) and forward composition (`>>`) replace `.` and `$`.
* **No `.` or `$` Operators**: Haskell's backward composition (`.`) and low-precedence right-associative application (`$`) are both omitted — `|>`/`>>` cover chaining and composition, and parentheses handle the rest.
* **No Map Literals**: Maps are constructed using `Map.fromList` (matching Elm's `Dict.fromList`).
* **Simplified Imports**: No `qualified` or `hiding` keywords (matching Elm's import design).
* **Pattern Aliases (`as`)**: Pattern aliases use `as` instead of Haskell's `@` to prevent syntactic conflicts with layout annotations.
* **No Guards**: Pattern matching does not support guards, matching Elm's simpler `case` expressions.

### 2.3 Omitted from Haskell/Elm
* **No user-defined interfaces**: Unlike Haskell's typeclasses (or Elm's extensible record polymorphism), the *set* of interfaces in v0 is closed (fixed at `Eq`, `Ord`, `Show`, `Semigroup`, `Monoid`, `Num`, `Fractional`, `Integral`) to keep compile-time dictionary passing and type checking simple — users cannot declare a brand-new interface. Instances of these existing interfaces are open, though — see §2.4.
* **No custom symbolic operators**: Unlike both Haskell and Elm, users cannot define new operators (e.g., `+++`), ensuring 1-to-1 parsing and node-graph mapping remain clean.
* **No List Comprehensions**: Haskell's list comprehension syntax (`[x | x <- xs]`) is omitted in favor of standard map/filter functions.
* **No Record Shorthand Destructuring**: Elm's shorthand record pattern matching (`{ x, y }`) is omitted.
* **Right to Left functor operators `<|` `<<` `=<<`**: not supported
* **No `.` function composition operator**
* **No multi-clause function definitions**: Unlike Haskell's `f 0 = ...` / `f n = ...` style, a function name may be bound by only one equation; branch on argument patterns using `case`, matching Elm.
* **Basically anything else that's in Haskell but not Elm**

### 2.4 Different / Custom
* **Open Instances, Closed Interfaces**: The *set* of interfaces is closed (no new ones), but instances of the existing interfaces are open — users may implement `Eq`/`Ord`/`Show`/etc. for their own types, not just built-in primitives.
* **Metadata & Annotations (`@name(...)` / `@{...}`)**: A compiler-checked layout and tool annotation system designed for graph coordinates, documentation, and metadata.
* **Unravel System (Reverse Execution)**: Backwards execution solving rule annotations (`unravel`), allowing changes to flow in reverse through the graph.
* **Holes in LHS of let bindings**: `let _ = expression` is allowed and dropped by the compiler.

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

type Maybe a
  = Just a
  | Nothing

type Result e a
  = Ok a
  | Err e
```

### 3.6 Generic Containers (Built-in)

| Type         | Description                               |
|--------------|-------------------------------------------|
| `List a`     | Ordered sequence                          |
| `Map k v`    | Key-value map (`k` must implement `Ord`)  |
| `Maybe a`    | Nullable / missing value                  |
| `Result e a` | Success or failure with a typed error     |

### 3.8 Tuple Types

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

Tuple arity is capped at 3 elements (pairs and triples only), matching Elm — larger fixed-size groupings should use a record instead.

---

## 5. Expressions

### 4.2 Pipe Operators

```knot
-- forward pipe: passes left value as argument to right function
x |> f |> g

-- forward composition: produces a new function (f then g)
f >> g       -- equivalent to \x -> g (f x)
```

### 4.3 Let Bindings

```knot
let
  radius = 5.0
  area   = 3.14159 * radius * radius
in
  area
```

### 4.6 Pattern Matching

```knot
case shape of
  Circle r       -> 3.14159 * r * r
  Rectangle w h  -> w * h
  Triangle a b c -> ...
```

#### Pattern Matching Rules:
- **No guards** (see §2.2): use nested `if...then...else` inside match branches instead.
- **List pattern matching** via `:` cons:
  ```knot
  case list of
    []     -> 0
    x : xs -> x + sum xs
  ```
- **Pattern aliases** (`as`, see §2.2):
  ```knot
  case list of
    (x : xs) as fullList -> fullList
  ```
- **No record shorthand** (see §2.3): match via alias + dot notation instead: `r as point -> point.x`.
- **No Float literal patterns** (matches Elm — float equality is unsound): compare explicitly with `if` inside the match arm instead. `Int` and `String` literal patterns are both fine (also matching Elm).


### 4.7 List & Map Literals

```knot
[1, 2, 3]
```

```knot
myMap :: Map String Int
myMap = Map.fromList [ ("apple", 1), ("banana", 2) ]
```

### 4.8 Built-in Operators & Precedence

To ensure unambiguous parsing and 1-to-1 visual node graph mapping, Knot defines a strict set of built-in operators and precedence rules:

| Precedence | Operators | Associativity | Description |
|:---|:---|:---|:---|
| 8 | `^` | Right | Exponentiation |
| 7 | `*`, `/`, `div`, `mod` | Left | Multiplication, division, modulo |
| 6 | `+`, `-`, `<>` | Left | Addition, subtraction, semigroup append |
| 5 | `:` | Right | List construction (cons) |
| 4 | `==`, `/=`, `<`, `<=`, `>`, `>=` | None | Comparison |
| 3 | `&&` | Right | Logical AND (lazy evaluation) |
| 2 | `\|\|` | Right | Logical OR (lazy evaluation) |
| 1 | `\|>`, `>>` | Left | Forward pipe, forward composition |

*(Fixed contradictions from earlier drafts: cons was previously listed here as `::`, which collides with the type-signature operator — cons is `:`, matching §2.1 and §4.6. Neither `.` nor `$` exist in Knot — see §2.3 — so neither has a precedence slot; use `>>`/`|>` instead.)*

Because Knot's operator set is closed (§2.3 — no user-defined operators), this table is exhaustive and fixed at parse time; unlike Elm, a parser never needs to consult import declarations to learn an operator's fixity.

#### Boolean Operators
- `not :: Bool -> Bool` — logical negation (prefix function; `&&`/`||` are in the table above and short-circuit as usual).

### 4.9 Unary Negation

`-` is overloaded between subtraction and negation; Knot resolves this exactly like Elm
does — a `-` with whitespace before but none after is unary negation binding tighter than
application (`f -1` is `f (-1)`, not `f - 1`). Symmetric spacing (`a - 1` or `a-1`) is
ordinary subtraction. The remaining case — whitespace *after* `-` but not before it
(`f- 1`) — is a parse error: it's ambiguous between the two and must be disambiguated
with parentheses or by fixing the spacing.

---

## 6. Definitions

```knot
name :: Type
name arg1 arg2 = expr
```

---

## 7. Interfaces (Built-in, Closed in v0)

The interface set is fixed — user-defined interfaces are not supported in v0 (see §14
for the v2 plan). Instances of these interfaces, however, are open: both built-in and
user-defined types may implement them (concrete instance-declaration syntax is TBD — see
§13). Users may also write their own function signatures constrained by these interfaces,
e.g. `myMax :: Ord a => a -> a -> a`, usable with any type that has an `Ord` instance.

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

#### Exponentiation

`^` (spec §4.8) is not a method of any single interface above — unlike
`+`/`-`/`*`/`/`/`div`/`mod`, its signature constrains *two different* type
variables via two different interfaces at once, so it's a standalone built-in
signature, matching Haskell exactly:

```knot
(^) :: (Num a, Integral b) => a -> b -> a
```

Base and exponent may differ in type — `2.5 ^ 3` is a `Float` base raised to
an `Int` exponent — the same shape as `fromIntegral` below, just infix.

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

Built-in instances: `Maybe`, `Result`, `IO`, `List`.

---

## 8. Built-in Monads & Do-Notation

| Type         | Purpose                                      |
|--------------|----------------------------------------------|
| `IO a`       | Side effects (file I/O, printer comms, etc.) |
| `Maybe a`    | Nullable / missing values                    |
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

A module's declared name should match its file path, dot-separated (e.g. module `Geometry.Shapes` lives at `Geometry/Shapes.knot`), matching Elm's convention — this lets the parser/module-loader map an import to a file without a separate lookup table.

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
- `import Foo exposing (..)` is also allowed — brings every name `Foo` exposes into unqualified scope, mirroring the module header's own `exposing (..)` wildcard.

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

**Desugaring**: `@name(args)` desugars into a single key of the equivalent `@{ ... }` block — a single argument becomes a bare value (`@label("My Function")` ≡ `@{ label = "My Function" }`); multiple arguments become a tuple (`@position(100, 200)` ≡ `@{ position = (100, 200) }`).

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

For annotating individual stages of a pipeline, `@annotation` is written **prefix**,
immediately before the expression atom it targets. Both forms work inline:

```knot
x = @nodeId("n1") @position(100, 200) f
  |> @{ nodeId = "n2", position = (200.0, 200.0) } g
  |> h
```

**Binding rule**: a prefix `@annotation` attaches only to the single closest-following atom (literal, identifier, parenthesized expression, or record/list/tuple literal) — never to a wider application or operator expression. `f @nodeId("n1") y` annotates `y`, not `f y`; hence the need for parens below:

For complex sub-expressions wrap in parens first:

```knot
x = @{ nodeId = "n1", position = (100.0, 200.0) } (f a b)
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

### 8.5 Annotations Are Typed, Prefix Functions

An annotation key is not a special grammatical category — it's an ordinary, prefix-
position Knot function with a real type signature, and `@key(args)` is checked exactly
like an application of that function: `args` is type-checked against `key`'s declared
parameter type, then the whole thing behaves as `identity` on the value it's attached to
at runtime (per this section's "no effect on forward runtime semantics"). Standard keys'
signatures:

- `unravel :: Unraveler -> ...` — takes a **function type** (see §11; `Unraveler`'s exact
  shape mirrors the annotated binding's own signature).
- `nodeId`, `label`, `doc`, `color`, `group` — take `String`.
- `position` — takes `(Float, Float)`.
- `collapsed` — takes `Bool`.

New keys (the set is open, per above) are added the same way: give the key a type.

**Scoping**: when an annotation's value is itself a function — as with `unravel` — that
function may reference anything in scope at the point the annotation is written: the
same `let`-bound names, function parameters, and imports visible to the binding it
annotates. This is what lets an `unravel` body close over its own function's parameters
(§11.1's `\inputs sensitivity -> ...`).

**Placement — the one special rule**: as ordinary prefix functions, annotations follow
the expression-atom binding rule of §8.3. The one addition to that rule is that an
annotation may also be stacked immediately above three things that are *not* expression
atoms: the right-hand side of a `let` binding, a function definition, and a `name ::
Type` signature line. The first two are really just the atom rule applied with the whole
bound value (or function body) as the atom; the signature-line case is the genuine
exception, since a bare `:: Type` line isn't an expression at all — §8.1's stacked form
is this rule combined with the AST-level merging of a signature and its definition into
one binding.

---

## 11. Unravel (Reverse Execution) 

TODO review this section ,you never reviewed this lol.
TODO you also need to add the special "unraveller" output node that attaches both to an output value to unravel, and an application output object for positioning in the UI

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

### 9.6 `Sensitivity` Is a Recursive Type Function

`Sensitivity` (the output-change type threaded through §9.1's unravel signatures) is not
an ordinary parametric type applied wholesale to a binding's output — it's a type-level
function that recurses into that output type's *shape*, mirroring structure rather than
wrapping it:

```
Sensitivity(record { f1 : T1, f2 : T2, ... }) = record { f1 : Sensitivity(T1), f2 : Sensitivity(T2), ... }
Sensitivity(tuple(T1, T2, T3))                = tuple(Sensitivity(T1), Sensitivity(T2), Sensitivity(T3))
Sensitivity(scalar T)                         = <leaf sensitivity vocabulary for T>  -- TBD, §13
```

This recursion applies uniformly to **user-defined record types**, not just built-in
tuples/records: if a binding's output is `type alias Point = { x : Float, y : Float }`,
its `Sensitivity Point` is `{ x : Sensitivity Float, y : Sensitivity Float }`. That lets
an unravel caller constrain `x` while leaving `y` free, rather than being forced to
specify (or leave entirely unconstrained) the whole record at once. The leaf case —
`Sensitivity` of a scalar type — is where the actual constraint vocabulary lives; still
open, see §13.

**Scope of the recursion — product shapes only, so far.** The rule above only covers
*product* shapes: record and tuple, where the set of fields is fixed and statically known.
It does not (yet) cover *sum* shapes — any multi-constructor `type` (§3.5), including
`List` (`Nil | Cons a (List a)` — `:`/`[]` are surface sugar over an ordinary recursive ADT
here, exactly as in Haskell/Elm, not a structurally distinct case), `Maybe`, `Result`, or a
user's own `type Shape = Circle Float | Rectangle Float Float`. Recursing `Sensitivity` into
a sum type would mean letting a constraint change *which constructor* is active (`Cons h t`
becoming `Nil`, `Just x` becoming `Nothing`) — a change to the value's shape, not just to a
field within a fixed shape — and that's a genuinely harder, unsolved problem, not merely an
unimplemented case of the rule above. Until it's designed, `Sensitivity` of any sum type
(built-in or user-defined, `List` included) falls to the same scalar leaf case as any other
non-product type.

---

## 12. Typed Holes

Knot uses `_` as a **typed hole** in expression position — a placeholder for an expression
that has not yet been supplied. Holes are intentionally invalid there: programs containing
an expression-position hole do not compile, but the compiler reports the expected type at
each hole site, making incomplete programs informative rather than silent. Named holes
(`_name`) are a separate, narrower feature restricted to pattern/binding discard position
(§12.3) — they are **not** valid as expression placeholders.

### 12.2 Expression holes

`_` is valid in any expression position, in its bare unnamed form only — `_name` is not
supported here (`f a _b c` is invalid; use `f a _ c`). The compiler reports what type is
required:

```knot
-- disconnected middle argument
result = f a _ c          -- error: _ :: SomeType (expected at this position)

-- missing function body
identity = \x -> _        -- error: _ :: a (the return type of the lambda)

-- partial literal
config = { host = "localhost", port = _ }   -- error: _ :: Int
```

### 12.3 Pattern match and Binding holes (`let _ = ...`)

`_` in a pattern or `let`-binding LHS discards the value — this is valid, not an error
(`let _ = expression` drops the result). A named variant is allowed here specifically:
`let _debugValue = expression in ...` is also valid, purely as a self-documenting mnemonic
for the reader — the name carries no compiler-checked meaning.


### 12.5 Annotation compatibility

Holes cannot carry annotations — `_ @nodeId("x")` is a parse error. Annotate
the enclosing expression or the binding instead.

---

## 13. Open Questions

1. **Metadata/annotation syntax** — TBD.
2. **Logging / observable side effects** — what's the story for debug output or
   structured logging within `IO`? Needs user stories before designing.
3. **Instance-declaration syntax** — user-defined instances of existing interfaces are
   confirmed allowed (§2.4, §7), but the actual declaration syntax isn't designed yet
   (Haskell-style `instance Eq Shape where (==) a b = ...`? something else?). Needed
   before the parser can support it.

---

## 14. Planned for v2

- **User-defined interfaces** — allow users to declare their own interfaces and implement
  them for custom types. Requires a constraint-solving/dictionary-passing subsystem;
  deferred to keep the v0 type system tractable.
- **Unit-aware numeric types** — `Length`, `Speed`, `Temperature`, etc. as distinct types
  with homogeneous `+`/`-` and explicit scaling functions. Deferred until the core
  language is stable.
