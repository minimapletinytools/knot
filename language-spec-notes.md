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

### 2.1 From Elm
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
* **no Partial Sections**: `(+)` supported but not `(+2)` `(2+)`

### 2.2 From Haskell
* **Lazy evaluation (call-by-need)**: Values are computed only when demanded (unlike Elm's strict evaluation). This allows for infinite structures and deferred graph computation.
* **Built-in typeclasses**: Overloaded operations use a typeclass-like interface system — see §2.4 for the full, closed interface list.
* **Type Signatures**: Standard `::` syntax.
* **List cons operator**: list cons (`:`) preserved.
* **Monadic/List Operators**: `do` syntax and bind operator (`>>=`) preserved — `bind`/`pure` are the `Context` interface's own methods (§2.4).
* **Holes**: `_` and `_somename` can be used in place of expressions as holes

### 2.3 Omitted from Haskell/Elm
* **No user-defined typeclasses**: Unlike Haskell's typeclasses (or Elm's extensible record polymorphism), the *set* of interfaces in v0 is closed — fixed, see §2.4 for the full list — to keep compile-time dictionary passing and type checking simple; users cannot declare a brand-new interface. Instances of these existing interfaces are open, though — see §2.4.
* **No custom symbolic operators**: Unlike both Haskell and Elm, users cannot define new operators (e.g., `+++`), ensuring 1-to-1 parsing and node-graph mapping remain clean.
* **No List Comprehensions**: Haskell's list comprehension syntax (`[x | x <- xs]`) is omitted in favor of standard map/filter functions.
* **No Record Shorthand Destructuring**: Elm's shorthand record pattern matching (`{ x, y }`) is omitted.
* **Right to Left functor operators `<|` `<<` `=<<`**: not supported
* **No `.` function composition operator**
* **No multi-clause function definitions**: Unlike Haskell's `f 0 = ...` / `f n = ...` style, a function name may be bound by only one equation; branch on argument patterns using `case`, matching Elm.
* **Basically anything else that's in Haskell but not Elm that's not listed in 2.2**

### 2.4 Different / Custom
* **Open Instances, Closed Interfaces**: The *set* of interfaces is closed (no new ones), but instances of the existing interfaces are open — users may implement any of the below for their own types, not just built-in primitives. The full, closed interface list:

  | Interface    | Key Operations                                | Haskell Analog                                                                      |
  |--------------|------------------------------------------------|---------------------------------------------------------------------------------------|
  | `Eq`         | `(==)`                                          | `Eq`                                                                                   |
  | `Ord`        | `compare` (implies `Eq`)                        | `Ord`                                                                                  |
  | `Show`       | `show`                                          | `Show`                                                                                 |
  | `Semigroup`  | `(<>)`                                          | `Semigroup`                                                                            |
  | `Monoid`     | `empty` (implies `Semigroup`)                   | `Monoid`                                                                               |
  | `Num`        | `(+)`, `(-)`, `(*)`, `negate`, `abs`, `signum`   | `Num`                                                                                   |
  | `Fractional` | `(/)`, `recip` (implies `Num`)                  | `Fractional`                                                                           |
  | `Integral`   | `div`, `mod` (implies `Num`, `Ord`)              | `Integral`                                                                             |
  | `Collection` | `map`, `foldl`, `foldr`, `filter`, `length`      | `Functor` + `Foldable`, merged into one interface                                      |
  | `Context`    | `pure`, `bind`                                  | `Applicative` + `Monad`, merged into one interface — replaces `Monad` and its `>>=`    |
* **Metadata & Annotations (`@name(...)` / `@{...}`)**: A compiler-checked layout and tool annotation system designed for graph coordinates, documentation, and metadata.
* **Unravel System (Reverse Execution)**: Backwards execution solving rule annotations (`unravel`), allowing changes to flow in reverse through the graph.
* **Holes in LHS of let bindings**: `let _ = expression` is allowed and dropped by the compiler.
* **Record Spreads**: A record can be defined to contain all fields of  `A` with `..A` syntax e.g. `type B = { ..A, someField : Int }`. works with row polymorphic constraints to e.g. `{a | ..A}`

---

## 3. Node Graph Model

TODO brief outline that all lanugage features must map to node spec, (reverse mapping need not be complete)
TODO mention something about reverse mapping enforces canonical linter

---

## 4. Lexical Syntax & Layout

### 4.1 Comments
**Matching Haskell** Line comments start with `--` and run to end of line. Block comments are `{- ... -}`
and nest (`{- outer {- inner -} still outer -}` is one comment).

### 4.2 Identifiers & Keywords
**Matching Haskell/Elm** Identifiers are ASCII-only: a leading letter, then letters/digits/`_`. Case
distinguishes the two identifier namespaces — lowercase-leading
for values, function names, type variables, and record fields; uppercase-leading for
types, constructors, and modules. 

**TODO** trailing `'` (not valid in elm, valid in haskell, choose)

Reserved words (never usable as an identifier): `module`, `exposing`, `import`, `as`,
`type`, `alias`, `let`, `in`, `if`, `then`, `else`, `case`, `of`, `do`, `True`, `False`,
`interface`, `where`, `instance`. (`interface`/`where` are reserved even though a user
never writes an `interface ... where` block themselves — see §10 — since `where` is
real, user-facing syntax via `instance ... where`.)

### 4.3 Layout / Indentation Rule
**Matching Haskell/Elm** Knot is layout-sensitive, in the tradition of Haskell/Elm: a block-forming construct
(`let`, `case`/`of` arms, `do`, and top-level declarations) establishes a reference
indent column, and every subsequent item at that block's level must align to it —
there's no explicit block-closing token. 

### 4.4 File & Module Structure
Every `.knot` file is exactly one module (§14). A module's declared name should match
its file path, dot-separated (`Geometry.Shapes` lives at `Geometry/Shapes.knot`).

---

## 5. Type System

### 5.1 Primitive Types

| Type     | Description           |
|----------|-----------------------|
| `Bool`   | `True` / `False`      |
| `Int`    | Integer               |
| `Float`  | 64-bit floating point |
| `String` | UTF-8 text            |
| `Unit`   | `()` — the unit type  |

`String` is fully opaque, like Elm's — there is no `Char` type at all, so a
`String` can't be decomposed into or built up from a list of characters the
way Haskell's `String = [Char]` can.

### 5.2 Type Aliases
**Matching Elm** `type alias Name = Type` gives an existing type a new name — a naming convenience,
not a new nominal type. Aliases are expanded away entirely before type checking, so
two different aliases naming the same underlying shape (e.g. two record aliases with
identical fields) are the same type as far as the checker is concerned.

### 5.3 Algebraic Data Types

**Matching Elm** 

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

using the `type` keyword. A limited form of Haskell's `deriving` *is*
supported — see §10.7.

### 5.4 Records

**Matching Elm** Recorld construction, update, and access all use Elm's syntax (§2.1):

```knot
p  = { x = 1.0, y = 2.0 }
p2 = { p | y = 3.0 }
p.x
```

A record type is **closed** by default (exactly these fields, no more) — a type
variable in record position instead means "any record with at least these fields"
(**open**, a.k.a. row-polymorphic):

```knot
distance :: { r | x : Float, y : Float } -> Float
```

Note that Elm's record pattern matching syntax e.g. `{ x, y }` is not supported.

**Record Spreads**: a record type can be defined to contain all of another's fields via
`..Name`, and this composes with row polymorphism:

```knot
type alias B = { ..A, someField : Int }     -- B has every field of A, plus someField
distance2 :: { a | ..GraphicsElement, label : String } -> Float
```

### 5.5 Tuples

**Matching Elm** 

```knot
pair :: (Int, String)
pair = (42, "hello")

triple :: (Float, Float, Float)
triple = (1.0, 2.0, 3.0)
```

Tuple arity is capped at 3 elements (pairs and triples only), matching Elm — larger
fixed-size groupings should use a record instead.

### 5.6 Built-in Generic Containers

TODO move this section to builtin section or prelude seciton or whatever

| Type         | Description                               |
|--------------|--------------------------------------------|
| `List a`     | Ordered sequence                          |
| `Map k v`    | Key-value map (`k` must implement `Ord`)  |
| `Maybe a`    | Nullable / missing value                  |
| `Result e a` | Success or failure with a typed error     |
| `IO a`       | Side effects (file I/O, printer comms, etc.) |

### 5.7 `Ordering`

TODO move this section to builtin section or prelude seciton or whatever

```knot
type Ordering
  = LT
  | EQ
  | GT
```

The result type of `compare` (§10). `Ordering` has its own `Eq`/`Ord`/`Show`
instances, same as any other primitive-shaped built-in type.

---

## 6. Type Inference & Checking

### 6.1 Let-Polymorphism / Generalization
Standard Hindley-Milner: a top-level or `let`-bound binding is generalized and may be
used at multiple types; a lambda- or `case`-bound parameter is not generalized within
its own scope (matching Haskell/Elm).

### 6.2 Numeric-Literal Polymorphism & Defaulting
An integer literal isn't hard-wired to `Int` — it starts as `Num a => a` and unifies
with whatever the surrounding context demands (`Float`, a signature, a sibling
operand, or even a user's own custom `Num` instance). If nothing else ever pins its
type down, it defaults to `Int` (Haskell-style defaulting, simplified to this one
closed-world case).

### 6.3 Ambiguous Constraints
A binding that ends up generalized over an interface obligation with no way to ever
resolve it — a constraint that never becomes part of any concrete use, and never
attaches to a function's own parameter — is a compile error, not a silently-ambiguous
runtime dictionary.

---

## 7. Expressions

### 7.1 Literals

**Matching Haskell/Elm**
- `Int` e.g. `5`
- `Float` e.g. `3.14`
- `Char` e.g. `'a'`
- `String` e.g. `"meow meow meow"`

### 7.2 Records, Lists, Tuples

**Matching Haskell/Elm**

```knot
[1, 2, 3]

myMap :: Map String Int
myMap = Map.fromList [ ("apple", 1), ("banana", 2) ]

pair = (42, "hello")
```

**Matching Elm**
```knot
point = { x = 1.0, y = 2.0 }
```

knot records can remain anonymous like Elm

### 7.3 Let, If/Else, Case

**Matching Haskell/Elm**

```knot
let
  radius = 5.0
  area   = 3.14159 * radius * radius
in
  area

if radius > 0.0 then area else 0.0

case shape of
  Circle r      -> 3.14159 * r * r
  Rectangle w h -> w * h
```

See §8 for the full pattern-matching story.

### 7.4 Operators & Precedence

**Differs from Haskell/Elm**

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
| 1 | `\|>`, `>>`, `>>=` | Left | Forward pipe, forward composition, monadic bind |

```knot
x |> f |> g              -- forward pipe
f >> g                    -- forward composition, equivalent to \x -> g (f x)
m >>= f                   -- monadic bind, equivalent to bind m f (§10.6, §11)
```

Because Knot's operator set is closed (§2.3 — no user-defined operators), this table is exhaustive and fixed at parse time; unlike Elm, a parser never needs to consult import declarations to learn an operator's fixity.

#### Boolean Operators

### 7.5 Unary Negation

**Matching Elm**
`-` is overloaded between subtraction and negation; Knot resolves this exactly like Elm
does — a `-` with whitespace before but none after is unary negation binding tighter than
application (`f -1` is `f (-1)`, not `f - 1`). Symmetric spacing (`a - 1` or `a-1`) is
ordinary subtraction. The remaining case — whitespace *after* `-` but not before it
(`f- 1`) — is a parse error: it's ambiguous between the two and must be disambiguated
with parentheses or by fixing the spacing.

### 7.6 Operator Sections
**Matching Elm** A bare operator in parens is a first-class function: `(+)` is `\x y -> x + y`. Unlike Haskell, there are no *partial* sections — `(+2)`/`(2+)` are both unsupported; write
the lambda out (`\x -> x + 2`) instead.

---

## 8. Pattern Matching

### 8.1 Supported Patterns
**Matching Haskell/Elm**

e.g.
```knot
case shape of
  Circle r       -> 3.14159 * r * r
  Rectangle w h  -> w * h
  Triangle a b c -> ...
```
```knot
case list of
  []     -> 0
  x : xs -> x + sum xs
```
```knot
  case pair of
    (x, y) -> x
```

**Matching Elm**
- **Pattern aliases** (`as`, see §2.1):
  ```knot
  case list of
    (x : xs) as fullList -> fullList
  ```
- **No Float literal patterns** (matches Elm — float equality is unsound): compare explicitly with `if` inside the match arm instead. `Int` and `String` literal patterns are both fine (also matching Elm).

**Differs from Haskell/Elm**

- **No record shorthand** (see §2.3): match via alias + dot notation instead: `r as point -> point.x`.
- **No `@` aliases** use `as` instead


### 8.2 No Guards
**Matching Elm**
Pattern matching does not support guards (§2.1) — use nested `if...then...else` inside a match arm instead.

### 8.3 Exhaustiveness Checking
**Matching Haskell**
The compiler checks case patterns are all reachable and exhaustive and reports as warnings if not.

---

## 9. Function & Value Definitions
**Matching Hasell/Elm**
```knot
name :: Type
name arg1 arg2 = expr
```

**Matching Elm**
A name is bound by exactly one equation (§2.3 — no multi-clause definitions);
branch on argument shape with `case` (§8) inside the body instead.

---

## 10. Interfaces (Typeclasses)

### 10.1 The Closed Interface Set
**Unique**
The interface set is fixed (§2.4) — user-defined interfaces are not supported in v0
(planned for v2, §20). Instances of these interfaces are open, though: both built-in
and user-defined types may implement them. Users may also write their own function
signatures constrained by these interfaces, e.g. `myMax :: Ord a => a -> a -> a`,
usable with any type that has an `Ord` instance.

See §2.4 for the full, authoritative table of all ten interfaces and their Haskell
analogs — not repeated here to avoid the two copies drifting apart.

### 10.2 Instance Declarations & Coherence
**Matching Haskell**
```knot
instance Eq Shape where
  (==) a b = ...
```
An instance's target is a nominal type, a *closed* record, or a tuple — never a bare
type variable, a function type, or an *open* (row-polymorphic) record, since there's
no fixed, exact shape to key an instance by in those cases.

At most one instance may exist per `(interface, type)` pair, across the whole program
— this includes builtin types.

Note `instance` can be used on record types by alias or by its full declaration

`type alias MyRecord = { field: Int }`
both `instance Num { field: Int } where` and `instance Num MyRecord where` are allowed

Orphan instances are allowed and conflicts are always an error
 
### 10.3 Superclasses
**Matching Haskell**
Some interfaces imply another: `Ord` implies `Eq`, `Monoid` implies `Semigroup`,
`Fractional` and `Integral` both imply `Num`, and `Integral` additionally implies
`Ord` (see §2.4's table for methods). Declaring an instance for an interface without
its superclass instance already existing *somewhere* in the same program is a
compile error — order of declaration within a module doesn't matter.

### 10.4 Structural Automatic Derivation

**Matching Elm** `Eq`, `Ord`, and `Show` derive automatically, field-by-field/element-by-
element, for any `Tuple`, `Record` (closed or open), or `Unit` value with
no declared instance — no `deriving` clause, and no instance declaration
at all, needed for the common case. A custom instance for one of these
three targets *overrides* the automatic derivation rather than
conflicting with it. 

### 10.5 Numeric Interfaces: Exponentiation & Conversion
**Similar to Haskell**
The three numeric interfaces, shown here in interface-block notation purely to
document their built-in signatures (users never write `interface ... where`
themselves — §4.2, §20):

```knot
interface Num a where
  (+)    :: a -> a -> a
  (-)    :: a -> a -> a
  (*)    :: a -> a -> a
  negate :: a -> a
  abs    :: a -> a
  signum :: a -> a

interface Num a => Fractional a where
  (/)    :: a -> a -> a
  recip  :: a -> a

interface (Num a, Ord a) => Integral a where
  div    :: a -> a -> a
  mod    :: a -> a -> a
```

Built-in instances: `Num Int`, `Num Float`, `Integral Int`, `Fractional Float`.

`^` (§7.4) is not a method of any single interface above — its signature constrains
two different type variables via two different interfaces at once, so it's a
standalone built-in signature, matching Haskell exactly:

```knot
(^) :: (Num a, Integral b) => a -> b -> a
```

Conversion: `fromIntegral :: (Integral a, Num b) => a -> b` converts an integral
value (e.g. `Int`) to any other numeric type (e.g. `Float`).

### 10.6 `Collection` & `Context`: Signatures and Restrictions

```knot
-- Collection
map    :: (a -> b) -> f a -> f b
foldl  :: (b -> a -> b) -> b -> f a -> b
foldr  :: (a -> b -> b) -> b -> f a -> b
filter :: (a -> Bool) -> f a -> f a
length :: f a -> Int

-- Context
pure :: a -> f a
bind :: f a -> (a -> f b) -> f b   -- also exposed as (>>=)
```

Built-in `Collection` instances: `List`, `Map`. Built-in `Context` instances:
`Maybe`, `Result`, `IO`, `List`.

A user's own function signature can be generic over `Collection`/`Context`
too, the same way `map`/`bind`/etc. themselves are — `f` applied to an
argument (`f a`) is valid anywhere in an ordinary `::` signature, not just
in these two interfaces' own built-in method shapes:

```knot
countIt :: Collection f => f Int -> Int
countIt xs = length xs

countIt (Box 5)     -- works for any type with its own Collection instance
countIt [1, 2, 3]   -- and for the builtins, through the same signature
```

### 10.7 `deriving` for User ADTs

**Different / Custom**
A `type` declaration (never `type alias` — see §10.4 above for why
records/tuples don't need this) can opt into `Eq`, `Ord`, `Show`,
`Semigroup`, or `Monoid` without writing an instance body at all:

```knot
type Shape
  = Circle Float
  | Rectangle Float Float
  deriving (Eq, Ord, Show)
```

A single interface needs no parens (`deriving Eq`). `deriving` only
covers field shapes that are "structurally easy" to derive automatically
— every field of every constructor must be one of:
- a bare use of one of the type's own declared parameters (`Box a`'s own
  `a` field needs the derived interface, exactly like a hand-written
  `instance Eq a => Eq (List a)`'s own `Eq a`);
- a canonical self-reference — the type's own name applied to its own
  parameters, in the same order (`Tree a`'s own `Tree a` fields);
- a concrete, zero-argument builtin type that already has the interface
  (`Float`, `Int`, ...);
- (`Eq`/`Ord`/`Show` only) `Unit`, which already derives these three
  automatically (§10.4).

Anything else — a field with its own nested generic argument (`List a`),
a tuple, a record, a function type, a differently-ordered self-reference,
or a field referencing another declared or derived type in the same
module — is rejected rather than silently accepted or left to surface as
a confusing missing-instance error somewhere else. `Semigroup`/`Monoid`
are further restricted to a single-constructor type: there's no sane
`<>` between two different constructors of a sum type. `Num`/
`Fractional`/`Integral` and `Collection`/`Context` can't be derived at
all — the former have no single obviously-correct pointwise meaning
(especially `*`), and the latter need real traversal logic, not a field
scan. A derived instance participates in ordinary coherence and
superclass checking exactly like a hand-written one: `deriving (Ord)`
without `Eq` is a missing-superclass error, and `deriving (Eq)` alongside
a hand-written `instance Eq` on the same type is a duplicate.

---

## 11. Do-Notation & Built-in Contexts

**Matching Haskell**

`do` notation sequences chained computations over any `Context` instance (§10.6),
desugaring to `bind`/`pure`:

```knot
do
  x <- someOption
  y <- anotherOption
  pure (x + y)
```

---

## 12. Prelude & Built-ins

TODO this section needs some reorg rewrite

**Different / Custom** *(planned — describes the intended split, not the
current implementation, which today hand-builds every entry below in
Rust regardless of which column it belongs in)*

Knot's own prelude is not entirely one thing. Some of it is a genuine
compiler primitive with no possible Knot-level definition; a lot of it is
better expressed as ordinary Knot source — a bundled `prelude.knot`,
parsed, canonicalized, and type-checked through the exact same pipeline
as user code, its resulting types and instances merged into the
"always in scope" environment every module starts with. Nothing about
this split is visible to a user: a name resolves the same way, and has
the same type, regardless of which side it lives on. The main benefit
isn't purity for its own sake — it's that anything with a real Knot body
can eventually be stepped through by a debugger like any other function,
rather than needing a special "this one's opaque" exception.

### 12.1 What Stays a Compiler Primitive

| What | Why it can't move |
|---|---|
| Raw `Int`/`Float`/`String` arithmetic, comparison, and formatting | Nothing lower-level in Knot to bottom out to — this is where the recursion has to stop. |
| The interface dispatch mechanism (which instance answers a given `show`/`compare`/`+`/...) | Compiler machinery, not user-writable Knot — the same sense in which Haskell's own dictionary passing isn't user-writable Haskell. |
| The closed interface *set* itself, and each interface's own method shapes | A restatement of "no user-defined interfaces" (§2.3) — applies equally to `Collection`/`Context` as to any other interface. |
| The `Collection`/`Context` kind-polymorphism machinery (`f` as a constructor variable, §10.6) | The type-system feature itself. *Using* it in an ordinary signature (`Collection f => f a -> f b`) is already fully open to users, though — see §10.6. This row is only about the underlying mechanism. |
| Operator tokens, their precedence, and what each one desugars to | Grammar, not privilege — see §13. Teaching the parser a new token from Knot source isn't something to build, and the operator set is closed (§2.3) regardless of where anything else lives. |
| `IO`'s own primitive actions | Side effects need a real runtime underneath; there's nothing to pattern-match. |

### 12.2 What Moves to `prelude.knot`

Everything below is expressible in ordinary, already-existing Knot syntax
— no new grammar, no privileged constructs. Each one is just an ordinary
`type` declaration or an ordinary `instance` declaration, exactly the
shape a user could already write for their own type:

- **`Bool`** — `type Bool = True | False deriving (Eq, Ord, Show)`
  (§10.7). `True`/`False` are already ordinary constructor names, not
  reserved syntax (§4.2).
- **`not`** — `not b = case b of True -> False; False -> True`. No
  interface dispatch involved at all.
- **`Ordering`** — `type Ordering = LT | EQ | GT deriving (Eq, Ord, Show)`.
- **`Maybe`, `Result`** — ordinary two-constructor ADTs; their
  `Eq`/`Ord`/`Show`/`Context` instances (`pure`/`bind` via
  pattern-matching) are the same shape a user's own `Context` instance
  already is (§10.6).
- **`List`** — structurally `Nil | Cons a (List a)`; its
  `Eq`/`Ord`/`Show`/`Collection`/`Context`/`Semigroup`/`Monoid` instances
  are all ordinary recursive pattern-matches. Surface syntax (`[]`,
  `x : xs`, `[1, 2, 3]`) stays exactly as today — pure grammar sugar
  (§13) desugaring to real `Cons`/`Nil` applications underneath.
- **`Map`**, if wanted — a real (if naive, association-list-backed)
  reference implementation, rather than an opaque stub.

Moving something from the left column to the right doesn't change its
type or behavior at all — only where its definition physically lives.

---

## 13. Syntax Sugar & Desugaring

**Different / Custom** *(planned — describes the intended pipeline; the
desugaring described here is currently scattered across parsing and
constraint generation rather than living in one dedicated stage)*

Compilation has a dedicated **desugaring** stage, sitting between
parsing and canonicalization:

```
source text --parse--> surface AST --desugar--> reduced AST --canonicalize--> ...
```

This exists for a reason specific to Knot: the *surface*, pre-desugar
syntax has its own meaning in the node-graph mapping (§3) that the
*reduced* form doesn't — a pipe reads as a distinct "pipe" node, a `do`
block as its own sequencing shape, a list literal as its own
construction node, not as a generic function application
indistinguishable from any other. If desugaring happened inline during
parsing (or were skipped entirely and left for the type checker to sort
out later, as `do` is today), there would be no artifact left anywhere
for the graph mapping to work from. So parsing produces and keeps the
sugared form; a separate pass reduces it to the smaller vocabulary
canonicalization and type-checking actually need to understand, before
either ever runs.

Not everything that could be called "sugar" belongs in this stage,
though — only sugar with its own distinct graph-node identity. Something
that's purely a textual authoring convenience with no separate graph
meaning (the stacked `@name(args)` vs. block `@{ ... }` annotation forms,
§15.1/§15.2 — both just mean "here are some key-value pairs," and the
editor presumably only cares about the resolved pairs, not which
spelling produced them) can keep desugaring wherever's convenient, as it
does today.

### 13.1 What Desugars Here

| Surface form | Reduces to |
|---|---|
| `[1, 2, 3]` | `Cons 1 (Cons 2 (Cons 3 Nil))` |
| `x : xs` | `Cons x xs` |
| `a \|> f` | `f a` |
| `f >> g` | `\x -> g (f x)` |
| `a >>= f` | `bind a f` |
| `do { x <- e1; rest }` | `bind e1 (\x -> rest)` (§11) |
| `(+)` (bare operator section, §7.6) | `\x y -> x + y` |

`(+)` is included here — not resolved at parse time, despite the parser
being what first recognizes the shape — specifically because it has its
own graph-node reading too: a first-class reference to an operation,
distinct from an ordinary two-argument lambda that happens to compute
the same thing.

### 13.2 What Doesn't

**Interface-dispatched operators** (`+`, `-`, `*`, `==`, `<>`, `<`, ...)
aren't sugar for a single fixed definition at all, so they don't desugar
here — `a + b` stays a `BinOp` all the way to type-checking, which turns
it into a `HasInstance("Num", ...)` obligation checked against whichever
instance actually matches (§10.2). *Which* code runs — a compiler
primitive for `Int`, or real Knot source for anything else, including a
`prelude.knot`-defined type (§12) — is a completely separate question
from desugaring, decided per call site by ordinary instance resolution,
not by this pass.



**`&&`/`||`** are a third case, distinct from both of the above: neither
sugar for something else nor interface-dispatched — a fixed, concrete
`Bool -> Bool -> Bool` operation with no instance lookup needed at all
(§7.4). TODO this is a problem becasue we don't have syntax to declare operators in prelude.knot

### 13.3 Representation

Desugaring rewrites in place, using the same surface AST type parsing
already produces, rather than introducing a separate, smaller "core"
grammar. By the time canonicalization runs, certain shapes (`Do`, the
sugar-only `BinOp` cases, bare operator sections, list literals) are
guaranteed to no longer occur — rewritten into ordinary function- and
constructor-application nodes — but the type itself doesn't yet
*enforce* that; a stray sugar node reaching canonicalization would just
be unreachable in practice, not a compile-time impossibility. Revisiting
this once the node-graph mapping (§3) is designed in more depth, with a
real "core" type replacing the surface one at this boundary, is a
plausible future hardening pass, not needed to ship the desugaring stage
itself.

---

## 14. Modules & Imports

Knot adopts Elm-style module and import syntax. Every file defines a single module (§4.4).

### 14.1 Module Declaration

```knot
module Geometry exposing (Point, distance, Shape(..))

module Geometry exposing (..)      -- expose everything
```

### 14.2 Imports

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

## 15. Metadata & Annotations

Annotations can be attached to any named binding. They carry node graph layout metadata,
stable IDs, reverse execution logic, documentation, and other tooling hints. They have
no effect on forward runtime semantics.

Annotations are evaluated at **graph construction time** — after type-checking but
before the graph runs. This means annotation values can be full Knot expressions
(including function references and conditionals), and all annotation fields are
type-checked like any other Knot code.

Two syntaxes are supported and can be freely mixed:

### 15.1 Stacked single-key form: `@name(args)`

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

### 15.2 Block form: `@{ ... }`

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

### 15.3 Inline sub-expression annotations

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

### 15.4 Standard annotation keys

| Key | Type | Meaning |
|---|---|---|
| `nodeId` | `String` | Stable unique ID — persists across edits |
| `position` | `(Float, Float)` | Canvas position |
| `label` | `String` | Display name override |
| `doc` | `String` | Documentation string shown in graph UI |
| `color` | `String` | Node color (hex) |
| `group` | `String` | Visual group/cluster |
| `collapsed` | `Bool` | Whether node renders collapsed by default |
| `unravel` | `Unraveler` | Reverse execution function — see §16 |

The annotation set is open — new keys can be added without changing the language.

### 15.5 Annotations Are Typed, Prefix Functions

An annotation key is not a special grammatical category — it's an ordinary, prefix-
position Knot function with a real type signature, and `@key(args)` is checked exactly
like an application of that function: `args` is type-checked against `key`'s declared
parameter type, then the whole thing behaves as `identity` on the value it's attached to
at runtime (per this section's "no effect on forward runtime semantics"). Standard keys'
signatures:

- `unravel :: Unraveler -> ...` — takes a **function type** (see §16; `Unraveler`'s exact
  shape mirrors the annotated binding's own signature).
- `nodeId`, `label`, `doc`, `color`, `group` — take `String`.
- `position` — takes `(Float, Float)`.
- `collapsed` — takes `Bool`.

New keys (the set is open, per above) are added the same way: give the key a type.

**Scoping**: when an annotation's value is itself a function — as with `unravel` — that
function may reference anything in scope at the point the annotation is written: the
same `let`-bound names, function parameters, and imports visible to the binding it
annotates. This is what lets an `unravel` body close over its own function's parameters
(§16.1's `\inputs sensitivity -> ...`).

**Placement — the one special rule**: as ordinary prefix functions, annotations follow
the expression-atom binding rule of §15.3. The one addition to that rule is that an
annotation may also be stacked immediately above three things that are *not* expression
atoms: the right-hand side of a `let` binding, a function definition, and a `name ::
Type` signature line. The first two are really just the atom rule applied with the whole
bound value (or function body) as the atom; the signature-line case is the genuine
exception, since a bare `:: Type` line isn't an expression at all — §15.1's stacked form
is this rule combined with the AST-level merging of a signature and its definition into
one binding.

---

## 16. Unravel (Reverse Execution)

TODO review this section ,you never reviewed this lol.
TODO you also need to add the special "unraveller" output node that attaches both to an output value to unravel, and an application output object for positioning in the UI

Every binding in the node graph can optionally carry an **unravel function** — a
reverse execution rule that, given a desired change in a node's output, computes the
corresponding desired changes to its inputs. This is what enables the graph to be
driven backwards: change a visualized output, propagate the change upstream to find
which input values (typically literals) to modify.

### 16.1 What an unravel function is

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

### 16.2 Default unravelers

Built-in operations have sensible default unravelers — no annotation needed for simple
cases. For example, addition splits the output sensitivity evenly across its inputs by
default. The annotation only needs to appear when overriding the default behavior.

### 16.3 Unravel on higher-order functions

When a node takes a function as input (e.g. `foldl`, `map`), its unravel receives not
just the function value but also that function's own unravel, and calls it during the
backward pass. Function strands in the graph carry their unravel bundled alongside
their forward implementation — passing a function to a higher-order node automatically
makes its unravel available for the backward pass.

### 16.4 Conflict resolution

When multiple downstream paths converge on the same node during a backward pass, that
node may receive conflicting desired values from different paths. The default strategy
is to average them; this is configurable per-node via a `solver` annotation key
(distinct from `unravel`). If the system cannot find a stable solution within a set
number of iterations, it surfaces a warning rather than silently producing a wrong
result.

### 16.5 Annotation examples

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

### 16.6 `Sensitivity` Is a Recursive Type Function

`Sensitivity` (the output-change type threaded through §16.1's unravel signatures) is not
an ordinary parametric type applied wholesale to a binding's output — it's a type-level
function that recurses into that output type's *shape*, mirroring structure rather than
wrapping it:

```
Sensitivity(record { f1 : T1, f2 : T2, ... }) = record { f1 : Sensitivity(T1), f2 : Sensitivity(T2), ... }
Sensitivity(tuple(T1, T2, T3))                = tuple(Sensitivity(T1), Sensitivity(T2), Sensitivity(T3))
Sensitivity(scalar T)                         = <leaf sensitivity vocabulary for T>  -- TBD, §19
```

This recursion applies uniformly to **user-defined record types**, not just built-in
tuples/records: if a binding's output is `type alias Point = { x : Float, y : Float }`,
its `Sensitivity Point` is `{ x : Sensitivity Float, y : Sensitivity Float }`. That lets
an unravel caller constrain `x` while leaving `y` free, rather than being forced to
specify (or leave entirely unconstrained) the whole record at once. The leaf case —
`Sensitivity` of a scalar type — is where the actual constraint vocabulary lives; still
open, see §19.

**Scope of the recursion — product shapes only, so far.** The rule above only covers
*product* shapes: record and tuple, where the set of fields is fixed and statically known.
It does not (yet) cover *sum* shapes — any multi-constructor `type` (§5.3), including
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

## 17. Typed Holes

Knot uses `_` as a **typed hole** in expression position — a placeholder for an expression
that has not yet been supplied. Holes are intentionally invalid there: programs containing
an expression-position hole do not compile, but the compiler reports the expected type at
each hole site, making incomplete programs informative rather than silent. Named holes
(`_name`) are a separate, narrower feature restricted to pattern/binding discard position
(§17.2) — they are **not** valid as expression placeholders.

### 17.1 Expression holes

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

### 17.2 Pattern match and Binding holes (`let _ = ...`)

`_` in a pattern or `let`-binding LHS discards the value — this is valid, not an error
(`let _ = expression` drops the result). A named variant is allowed here specifically:
`let _debugValue = expression in ...` is also valid, purely as a self-documenting mnemonic
for the reader — the name carries no compiler-checked meaning.

### 17.3 Annotation compatibility

Holes cannot carry annotations — `_ @nodeId("x")` is a parse error. Annotate
the enclosing expression or the binding instead.

---

## 18. Diagnostics: Errors & Warnings

A hard type error rejects the program; a warning does not. What falls in each bucket:

**Errors** — ordinary type mismatches; referencing an undefined name; a binding
generalized over an interface obligation that never resolves (§6.3, "ambiguous
constraint"); a concrete type missing a required interface instance; two instances
declared for the same `(interface, type)` pair (§10.2); an interface instance
declared without its required superclass instance (§10.3); an instance declared for
a target shape that can't be given one at all — a bare type variable, a function
type, or an open record (§10.2).

**Warnings** (never errors — §1) — a `case` that doesn't cover every possible value
of its scrutinee's type; a `case` arm that can never be reached (§8.3).

---

## 19. Open Questions

1. **Logging / observable side effects** — what's the story for debug output or
   structured logging within `IO`? Needs user stories before designing.
2. **`Sensitivity`'s leaf scalar vocabulary** — the constraint vocabulary for a
   scalar's own sensitivity (candidates seen elsewhere: `Exact`/`Range`/`Tolerance`/
   `Free`) is still undesigned (§16.6).
3. **`Sensitivity` over sum types** — letting an unravel constraint change *which
   constructor* is active, not just a fixed shape's fields, is a genuinely harder,
   unsolved problem (§16.6).

---

## 20. Planned for v2

- **User-defined interfaces** — allow users to declare their own interfaces and implement
  them for custom types. Requires a constraint-solving/dictionary-passing subsystem;
  deferred to keep the v0 type system tractable.
- **Unit-aware numeric types** — `Length`, `Speed`, `Temperature`, etc. as distinct types
  with homogeneous `+`/`-` and explicit scaling functions. Deferred until the core
  language is stable.
