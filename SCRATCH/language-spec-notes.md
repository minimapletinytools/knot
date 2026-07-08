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

## 2. Syntax Conventions

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

## 3. Type System

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

---

## 4. Expressions

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

### 4.7 List Literals

```knot
[1, 2, 3]
```

Map literal syntax TBD.

---

## 5. Definitions

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

## 6. Interfaces (Built-in, Closed in v0)

The interface set is fixed — user-defined interfaces are not supported in v0 (see §10
for the v2 plan). Only built-in types have instances.

### 6.1 Core Interfaces

| Interface     | Key Operations                                   |
|---------------|--------------------------------------------------|
| `Eq a`        | `(==) :: a -> a -> Bool`                        |
| `Ord a`       | `compare :: a -> a -> Ordering` (implies `Eq`)  |
| `Show a`      | `show :: a -> String`                           |
| `Semigroup a` | `(<>) :: a -> a -> a`                           |
| `Monoid a`    | `empty :: a` (implies `Semigroup`)              |

### 6.2 Collection Interface 

Implemented by `List` and `Map`:

```knot
map    :: (a -> b) -> f a -> f b
foldl  :: (b -> a -> b) -> b -> f a -> b
foldr  :: (a -> b -> b) -> b -> f a -> b
filter :: (a -> Bool) -> f a -> f a
length :: f a -> Int
```

Consider doing functor, foldable, and doing interface hierarchy, but the above is fine for V1

### 6.3 Context Interface (Monadic Chaining)

```knot
pure :: a -> f a
bind :: f a -> (a -> f b) -> f b   -- also exposed as (>>=)
```

Built-in instances: `Option`, `Result`, `IO`, `List`.

---

## 7. Built-in Monads & Do-Notation

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

## 8. Metadata & Annotations

Annotations use `@name(args)` syntax and can be attached to any named binding or
sub-expression. They carry node graph layout metadata, stable IDs, documentation, and
other tooling hints. They have no effect on runtime semantics.

### 8.1 Declaration annotations

Placed on the line(s) immediately above a top-level definition or a `let` binding:

```knot
@nodeId("f1")
@position(100, 200)
@label("My Function")
myFunc :: Int -> Int
myFunc x = x + 1

-- also valid on let bindings
let
  @nodeId("n1")
  @position(150, 300)
  result = myFunc 42
in result
```

Multiple `@` lines stack and all apply to the binding that follows.

### 8.2 Inline sub-expression annotations

For annotating individual stages of a pipeline or other sub-expressions, `@annotation`
is written **postfix**, immediately after the expression atom it targets. It binds
tighter than any operator, so `f @nodeId("n1") |> g` means `(f @nodeId("n1")) |> g`.

```knot
x = f @nodeId("n1") @position(100, 200)
  |> g @nodeId("n2") @position(200, 200)
  |> h @nodeId("n3") @position(300, 200)
  |> y
```

For complex sub-expressions (not just a single identifier), wrap in parens first:

```knot
x = (f a b) @nodeId("n1") @position(100, 200)
  |> g
```

When inline annotations become unwieldy, extract to named `let` bindings and annotate
those instead — that is always the fallback:

```knot
x =
  let
    @nodeId("n1") @position(100, 200)
    step1 = f
    @nodeId("n2") @position(200, 200)
    step2 = step1 |> g
  in
    step2 |> h |> y
```

### 8.3 Standard annotation keys

| Annotation | Args | Meaning |
|---|---|---|
| `@nodeId(id)` | String | Stable unique ID for the node — persists across edits |
| `@position(x, y)` | Float, Float | Canvas position of the node |
| `@label(text)` | String | Display name override (defaults to the binding name) |
| `@doc(text)` | String | Documentation string shown in the graph UI |
| `@color(hex)` | String | Node color in the graph |
| `@group(name)` | String | Visual group/cluster the node belongs to |
| `@collapsed` | — | Node renders collapsed in the graph by default |

The annotation set is open — new keys can be added without changing the language.

---

## 9. Open Questions

1. **Map literal syntax** — TBD.
2. **Module/import syntax** — TBD.
3. **Metadata/annotation syntax** — TBD.
4. **Logging / observable side effects** — what's the story for debug output or
   structured logging within `IO`? Needs user stories before designing.

---

## 10. Planned for v2

- **User-defined interfaces** — allow users to declare their own interfaces and implement
  them for custom types. Requires a constraint-solving/dictionary-passing subsystem;
  deferred to keep the v0 type system tractable.
- **Unit-aware numeric types** — `Length`, `Speed`, `Temperature`, etc. as distinct types
  with homogeneous `+`/`-` and explicit scaling functions. Deferred until the core
  language is stable.
