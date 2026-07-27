# Knot AST Parser — Implementation Plan

Covers `knot-syntax` — the Rust crate that parses Knot source text into an AST. This is
"Step 1: language -> AST converter" from `TODO.txt`. Basis for implementation; see
`claude-ast-impl-notes-7-26-2026.md` for the Elm research this builds on and
`language-spec-notes.md` for the language being parsed.

---

## 1. Scope

**In**: lexing, layout/indentation, the full expression/pattern/type/declaration/module
grammar per the current spec (not MVP-trimmed), and operator precedence resolution. Because
Knot's operator set is closed (§2.3 — no user-defined operators), this parser fully resolves
precedence itself; there's no later canonicalization pass needed for that, unlike Elm.

**Out**: name resolution, type checking, evaluation, node-graph (Tangle) rendering, and
annotation *semantics* (merging stacked+block annotations on key conflict, running
`unravel`/`solver`) — the parser only needs to capture annotation values as parsed
expressions and attach them to the right node; later stages interpret them.

**Included as a thin post-parse layer** (structural, not semantic — see §3): tuple arity
≤ 3 and duplicate top-level bindings. Both are properties of the finished AST that don't
need type information to check, so they live here rather than being deferred to a future
semantic-analysis crate.

---

## 2. Architecture decisions (recap — already settled in prior discussion)

- **Hand-rolled recursive descent, no parser-combinator crate.** Full control over error
  messages, lowest conceptual surface area for future edits/rewrites, no dependency
  version-churn risk. Matches Elm's own choice, made deliberately given "quality +
  simplicity over performance."
- **Layout handled inline**, not via a separate tokenize-then-layout pass: a `ParseState`
  carries a reference indent column, and `check_indent`/`check_aligned` calls at each
  block-forming construct (`let`, `case`, `do`, top-level decls) enforce it. Concept
  ported from Elm's `Parse.Space`, not the code.
- **AST nodes own their data** (`String`, `Vec<T>`) rather than borrowing zero-copy slices
  of the source — no lifetimes on AST types, a bit more allocation.
- **One flat `ParseError`** (kind + span + context stack of enclosing rule names), not
  Elm's fully bespoke per-production error ADT — most of the message quality, far less code.
- **Operator precedence resolved at parse time** via precedence-climbing over the §4.8
  table, producing a properly nested `BinOp` tree directly (never a flat chain).
- **Tuple arity and multi-clause rejection are post-parse checks, not grammar limits** —
  parse permissively, reject with a good message afterward. This mirrors what Elm actually
  does for tuple arity (confirmed from source), and it's the only sane approach for
  multi-clause detection anyway, since "was this name already bound?" is inherently a
  whole-module check, not something a single production can know.

---

## 3. Crate & module layout

```
compiler/
  Cargo.toml                 (workspace root)
  knot-syntax/
    Cargo.toml
    src/
      lib.rs
      span.rs                -- Span, Spanned<T>, Cursor
      error.rs               -- ParseError, ErrorKind, context stack
      state.rs               -- ParseState, check_indent/check_aligned, whitespace+comment skipping
      lex/
        mod.rs
        ident.rs             -- lower/upper/qualified identifiers, reserved words
        literal.rs           -- int/float/string literals
      ast/
        mod.rs
        expr.rs              -- Expr, BinOp, Annotation
        pattern.rs           -- Pattern
        ty.rs                -- Type
        decl.rs              -- Decl, FnDef, InstanceDecl, Module, Import, Exposing
      parse/
        mod.rs               -- parse_module entry point
        ty.rs                -- type-expression grammar
        pattern.rs           -- pattern grammar
        expr.rs              -- expression grammar (atoms, application, operators, if/case/let/lambda/do)
        annotation.rs        -- @name(args) / @{...} grammar, prefix atom-binding
        decl.rs              -- signatures + fn defs (merged), type/type alias decls, instance decls
        module.rs            -- module header, imports/exposing
      validate.rs            -- post-parse checks: tuple arity, duplicate top-level bindings
    tests/
      corpus.rs              -- walks corpus/, parses each fixture, snapshot-compares or expects failure
```

---

## 4. AST design (sketch — shapes to nail down now, not final Rust)

```rust
pub struct Span { pub start: u32, pub end: u32 }
pub struct Spanned<T> { pub span: Span, pub node: T }

pub enum Expr {
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    BoolLit(bool),
    Unit,
    Var(Name),                     // lower, possibly qualified: Foo.bar
    Ctor(Name),                    // upper, possibly qualified: Foo.Bar
    Hole,                          // _ only — named holes (_name) are pattern/binding-only, not expression placeholders
    Lambda(Vec<Spanned<Pattern>>, Box<Spanned<Expr>>),
    App(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
    BinOp(BinOp, Box<Spanned<Expr>>, Box<Spanned<Expr>>),  // already precedence-resolved
    Negate(Box<Spanned<Expr>>),
    If(Box<Spanned<Expr>>, Box<Spanned<Expr>>, Box<Spanned<Expr>>),
    Let(Vec<(Spanned<Pattern>, Spanned<Expr>)>, Box<Spanned<Expr>>),
    Case(Box<Spanned<Expr>>, Vec<(Spanned<Pattern>, Spanned<Expr>)>),
    Do(Vec<DoStmt>, Box<Spanned<Expr>>),                   // x <- expr; ...; final expr
    List(Vec<Spanned<Expr>>),
    Tuple(Vec<Spanned<Expr>>),                             // arity checked post-parse, see validate.rs
    Record(Vec<(String, Spanned<Expr>)>),
    RecordUpdate(Box<Spanned<Expr>>, Vec<(String, Spanned<Expr>)>),
    FieldAccess(Box<Spanned<Expr>>, String),
    Annotated(Vec<Annotation>, Box<Spanned<Expr>>),        // prefix @ann atom, atom-binding scope only
}

pub enum Pattern {
    Wildcard(Option<String>),      // _ or _name
    Var(String),
    Literal(PatternLiteral),       // Int and String; no Float (unsound equality, matches Elm)
    Ctor(Name, Vec<Spanned<Pattern>>),   // covers True/False as 0-arity ctors too
    Tuple(Vec<Spanned<Pattern>>),
    Cons(Box<Spanned<Pattern>>, Box<Spanned<Pattern>>),  // x : xs
    Nil,                            // []
    As(Box<Spanned<Pattern>>, String),
}

pub enum Type {
    Named(Name, Vec<Type>),        // Int, List a, Map k v, Option a
    Var(String),
    Fn(Box<Type>, Box<Type>),
    Tuple(Vec<Type>),
    Record(Vec<(String, Type)>, Option<String>),  // fields + optional extension row var
    Unit,
}

pub struct Constraint { pub interface: String, pub type_var: String }  // e.g. `Ord a`
pub struct TypeSignature {
    pub constraints: Vec<Constraint>,  // e.g. [Ord a] for `Ord a =>`; empty if unconstrained
    pub ty: Type,
}
// Constraints only ever appear as a prefix list before `=>` at the top of a `::`
// signature — they're a property of the signature, not of `Type` itself, so `Type`
// stays unconstrained-only and this wraps it instead.

pub struct Annotation { pub key: String, pub value: Spanned<Expr> }
// @name(args) desugars into this shape at parse time (single arg -> bare value,
// multiple args -> tuple, per §10 desugaring rule) — one representation, not two.

pub struct FnDef {
    pub name: String,
    pub signature: Option<Spanned<TypeSignature>>,  // merged in from a preceding `name :: Type` line
    pub params: Vec<Spanned<Pattern>>,
    pub body: Spanned<Expr>,
    pub annotations: Vec<Annotation>,
}

pub struct InstanceDecl {
    pub interface: String,             // Eq, Ord, Show, Num, ...
    pub constraints: Vec<Constraint>,  // e.g. `Eq a =>` for `instance Eq a => Eq (Maybe a)`
    pub target: Type,                  // Shape, or Maybe a, etc.
    pub methods: Vec<FnDef>,           // each method's `signature` is always None — inherited from the interface
}
// Haskell-style: `instance Eq Shape where (==) a b = ...`. Methods reuse the same
// single-equation-plus-`case` rule as any other function (§2.3) — no special-casing.

pub enum Decl {
    Fn(FnDef),
    TypeAlias(String, Vec<String>, Type),
    TypeDecl(String, Vec<String>, Vec<(String, Vec<Type>)>),   // ADT: name, params, variants
    Instance(InstanceDecl),
}

pub enum Exposing { All, Some(Vec<ExposedItem>) }
pub enum ExposedItem { Value(String), TypeOnly(String), TypeWithVariants(String) }  // Foo vs Foo(..)
pub struct Import { pub module: Vec<String>, pub alias: Option<String>, pub exposing: Option<Exposing> }

pub struct Module {
    pub name: Vec<String>,          // dot-separated
    pub exposing: Exposing,
    pub imports: Vec<Import>,
    pub decls: Vec<Spanned<Decl>>,
}
```

**One parsing subtlety worth flagging now**: a type signature (`name :: Type`) and its
definition (`name arg = expr`) are two separate lines but *one* `FnDef` node — the
declaration parser needs to look ahead after an optional signature line and require the
next declaration to share its name, merging them (and erroring clearly if a signature is
orphaned or names don't match). Stacked/block annotations sit above the signature line but
conceptually annotate the merged binding, so they attach to the `FnDef` as a whole, not to
an intermediate "signature" node — this is why `Decl` has no separate `TypeSig` variant.

---

## 5. Build order / milestones

- **M0** — Workspace + crate scaffold; `Span`/`Cursor`/`ParseState`; `check_indent`/
  `check_aligned`; whitespace + comment (`--`, nestable `{- -}`) skipping.
- **M1** — Lexical layer: lower/upper/qualified identifiers, reserved words (now
  including `instance` — `where` graduates from meta-syntax-only to genuine user-facing
  syntax alongside it), Int/Float/String literals.
- **M2** — `Type` grammar (needed early since signatures reference it before expressions do).
- **M3** — `Pattern` grammar.
- **M4** — `Expr` grammar, in sub-stages: atoms → application → unary negation →
  precedence-climbing binary ops (§4.8 table) → layout-heavy forms (`if`/`let`/`case`/
  lambda/`do`).
- **M5** — Declarations: signature+def merging into `FnDef` (including optional
  interface constraints on the signature, e.g. `Ord a =>`), `type`/`type alias`,
  `instance` declarations (Haskell-style, reusing the constraint grammar and the
  FnDef-body grammar for methods), module header, imports/`exposing`.
- **M6** — Annotations layered on top of M4/M5: prefix `@name(args)`/`@{...}`, atom-binding
  rule, attaching to both bindings and expression atoms.
- **M7** — Post-parse validation: tuple arity ≤ 3, duplicate top-level bindings.
- **M8** — Public `parse_module` entry point + corpus test harness wired up.

---

## 6. Test corpus design

### Location & reuse

`corpus/` at the repo root — sibling to `compiler/` and `prototype/`, **not** nested inside
`knot-syntax/tests/`. This is deliberate: the type-checker and interpreter crates that come
later can point their own test suites at the same source snippets without duplicating them.
`knot-syntax/tests/corpus.rs` is just the first consumer.

```
corpus/
  valid/
    literals/
    operators/
    unary-negation/
    patterns/
    expressions/       (let, case, if, lambda, do, records, lists)
    types/             (adt, extensible-records, tuples, aliases, generic-containers)
    interfaces/        (constrained signatures, instance declarations)
    modules/           (header, imports, exposing)
    annotations/
    holes/
  invalid/             (syntax errors only — semantically-invalid-but-syntactically-fine
                         cases, e.g. type errors, belong to a future type-checker's own
                         corpus, not here)
```

Each fixture is a small `.knot` file, named for what it demonstrates
(`corpus/valid/operators/exponent-right-assoc.knot`). `invalid/` fixtures carry a leading
comment stating the expected failure reason.

### Harness

Kept deliberately simple: one `#[test]` walks `corpus/valid/**/*.knot`, parses each, and
snapshot-compares the AST via `insta` (one snapshot per fixture, generated on first run,
diffed thereafter). A second `#[test]` walks `corpus/invalid/**/*.knot` and asserts each one
fails to parse. If per-file CI granularity becomes valuable later, `datatest-stable` can
generate one named test per fixture with minimal change — not needed for v1.

### Checklist

**literals/**
- `int.knot` — `x = 42`
- `int-hex.knot` — `x = 0xFF`
- `float.knot` — `x = 3.14`
- `float-exponent.knot` — `x = 1.5e10`
- `string.knot` — `x = "hello\nworld"`
- `bool.knot` — `x = True` / `y = False`
- `unit.knot` — `x = ()`
- *invalid*: `float-no-leading-digit.knot` — `x = .5`
- *invalid*: `float-no-trailing-digit.knot` — `x = 1.`

**operators/**
- `precedence-mul-over-add.knot` — `x = 1 + 2 * 3` (expect `1 + (2*3)`)
- `precedence-exp-right-assoc.knot` — `x = 2 ^ 3 ^ 2` (expect `2 ^ (3^2)`)
- `cons-right-assoc.knot` — `xs = 1 : 2 : []`
- `pipe-chain.knot` — `x = a |> f |> g`
- `compose-chain.knot` — `h = f >> g`
- `logical-precedence.knot` — `x = a && b || c`
- *invalid*: `comparison-non-assoc.knot` — `x = a == b == c`
- *invalid*: `dot-composition-removed.knot` — `h = f . g` (no `.` operator token anymore)
- *invalid*: `dollar-not-supported.knot` — `x = f $ y`
- *invalid*: `cons-double-colon.knot` — `xs = 1 :: []` (`::` is not cons)

**unary-negation/**
- `negate-literal-arg.knot` — `x = f -1` (expect `f (-1)`)
- `negate-var-arg.knot` — `x = f -y`
- `subtraction-spaced.knot` — `x = a - 1`
- `subtraction-unspaced.knot` — `x = a-1`
- *invalid*: `negate-ambiguous-spacing.knot` — `x = f- 1`

**patterns/**
- `cons-pattern.knot`, `as-alias.knot`, `ctor-pattern.knot`, `tuple-pattern.knot`,
  `wildcard.knot`, `bool-ctor-pattern.knot` (`True`/`False` as 0-arity ctors),
  `int-literal-pattern.knot`, `string-literal-pattern.knot`
- *invalid*: `guard-not-supported.knot`
- *invalid*: `record-shorthand-pattern.knot` — `case r of { x, y } -> x`
- *invalid*: `float-literal-pattern.knot` — forbidden, unsound equality (matches Elm)

**expressions/**
- `let-multi-binding.knot`, `let-nested.knot`, `case-multi-arm.knot`, `if-then-else.knot`,
  `lambda-single-arg.knot`, `lambda-multi-arg.knot`, `do-block.knot`,
  `record-construct.knot`, `record-update.knot`, `field-access.knot`,
  `field-access-chained.knot` (`a.b.c`), `list-literal.knot`, `map-fromList.knot`
- *invalid*: `layout-violation.knot` — a `let` binding misaligned with its siblings
- *invalid*: `multiclause-fn.knot` — `f 0 = 1` then `f n = n` (post-parse check)

**types/**
- `adt-multi-variant.knot`, `type-alias-record.knot`, `extensible-record-type.knot`,
  `tuple-type-pair.knot`, `tuple-type-triple.knot`, `generic-container-types.knot`
  (signatures using `List a`, `Map k v`, `Option a`, `Result e a`)
- *invalid*: `tuple-type-arity-4.knot` (post-parse check)

**interfaces/**
- `constrained-signature.knot` — `myMax :: Ord a => a -> a -> a`
- `constrained-signature-multi.knot` — `fromIntegral :: (Integral a, Num b) => a -> b`
- `instance-declaration.knot` — `instance Eq Shape where (==) a b = ...`

**modules/**
- `module-header-explicit-exposing.knot`, `module-header-wildcard-exposing.knot`,
  `import-qualified.knot`, `import-aliased.knot`, `import-exposing-some.knot`,
  `import-exposing-wildcard.knot`

**annotations/**
- `stacked-single-key.knot`, `block-form.knot`, `mixed-stacked-and-block.knot`
- `inline-prefix-annotate-fn.knot` — `@nodeId("n1") f y` (annotates `f`, the closest
  *following* atom)
- `inline-prefix-annotate-arg.knot` — `f @nodeId("n1") y` (annotates `y`)
- `inline-parens-for-wider-scope.knot` — `@{...} (f a b)`
- `desugar-single-arg.knot` / `desugar-multi-arg-tuple.knot` — confirm `@label("x")` and
  `@position(1, 2)` desugar to the same AST as their `@{...}` equivalents (can literally
  snapshot two fixtures to the same expected output)
- *invalid*: `hole-with-annotation.knot` — `_ @nodeId("x")`

**holes/**
- `hole-anonymous-arg.knot` — `f a _ c`
- `hole-lambda-body.knot` — `\x -> _`
- `hole-let-discard.knot` — `let _ = expr in result`
- `hole-let-discard-named.knot` — `let _debugValue = expr in result` (mnemonic only, no
  compiler-checked meaning)
- `hole-record-field.knot` — `{ host = "x", port = _ }`
- *invalid*: `hole-named-in-expr.knot` — `zipWith _f _xs _ys` (named holes aren't valid
  expression placeholders)

---

## 7. Decisions made during planning (previously open questions)

| # | Question | Resolution |
|---|---|---|
| 1 | Float literal patterns in `case`? | Forbidden — matches Elm (float equality is unsound). |
| 2 | String literal patterns in `case`? | Allowed — matches Elm. |
| 3 | User-facing constrained type signatures (`Ord a => ...`)? | Allowed — only *new* interfaces are forbidden; instances of existing ones and constrained signatures using them are both fine. |
| 4 | Named holes (`_name`) as `let`-binding LHS? | Allowed there, and *only* there — not valid as an expression-position placeholder (`f a _b c` is invalid; `f a _ c` is fine). |
| 5 | Can annotations attach to `type`/`type alias` declarations? | No, not in v0 — possible V2 feature. |

One more resolved along the way, not originally on this list: **instance-declaration
syntax** follows Haskell (`instance Eq Shape where (==) a b = ...`) — see `InstanceDecl`
in §4 and the M5 note in §5.

---

## 8. Next steps

- Confirm/adjust this plan.
- Materialize `corpus/` as actual `.knot` files from the checklist above — mechanical and
  low-risk, happy to just do it next.
- Then scaffold `compiler/knot-syntax/` per §3 and start M0.
