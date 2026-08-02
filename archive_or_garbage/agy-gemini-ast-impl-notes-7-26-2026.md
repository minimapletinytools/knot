# Knot Language AST Parser Specification & Implementation Plan (`knot-parser`)
**Date:** July 26, 2026  
**Target Crate:** `knot-parser` (Rust)

---

## 1. Language Spec Review: Gaps & Clarifications for AST Parsing

Before building the Rust `Code -> AST` parser module, several syntax edge cases in the Knot language specification must be formally specified:

### A. Annotation Grammar & Precedence
* **Target Nodes**: Annotations (`@name(args)` or `@{ ... }`) attach to **Named Value Bindings** (top-level functions, `let` bindings) and **Expression Atoms**.
* **Inline Precedence**: Sub-expression annotations bind tightly to the expression atom immediately preceding them. In `f x @{ nodeId = "1" }`, the annotation applies to atom `x`. To annotate `f x`, explicit parens are required: `(f x) @{ nodeId = "1" }`.
* **Annotation Value Expressions**: Annotation values can be full Knot expressions (e.g. `unravel = \inputs sensitivity -> ...`). The parser recursively invokes `parse_expr` inside annotation record values.

### B. Dot Token Lexing Rules
* In Knot:
  * `record.field` (no space around `.`) is field access.
  * `f . g` (spaces around `.`) is function composition.
* **Lexer Resolution**: The lexer distinguishes between `Token::Dot` (spaced) and `Token::FieldAccess(Identifier)` (unspaced `.field`).

### C. Pattern Aliases vs Annotations
* Knot uses `as` for pattern aliases (`(x : xs) as fullList`) to avoid syntactic collisions with layout annotations (`@`).
* **Parser Resolution**: The pattern parser disallows `@` in patterns and treats `as` as a contextual keyword at precedence lower than cons (`:`).

### D. Typed Holes & Loose Bindings
* Holes `_` and `_name` can appear in **expression positions** (`result = f a _ c`).
* `let _ = expr in ...` is a valid binding representing loose/unconnected outputs.
* **AST Resolution**: `Expr::Hole(Option<String>)` is a first-class AST expression variant, distinct from `Pattern::Wildcard`.

---

## 2. Lessons from Elm Compiler (`elm/compiler`) Architecture

The official Haskell implementation of Elm (`elm/compiler`) structures its frontend parser into a clean, zero-allocation pipeline:

```
Source Text (UTF-8) 
   │
   ▼
Parse.Primitives / Parse.Space (Tracks Line, Column, Indentation Stack, Comments)
   │
   ├──► Parse.Module       ──► Module Header & Imports
   ├──► Parse.Declaration  ──► Top-level Types, Signatures, Bindings
   ├──► Parse.Type         ──► ADTs, Records, Function Type Signatures
   ├──► Parse.Pattern      ──► Constructors, Cons, Tuples, Wildcards
   └──► Parse.Expression   ──► Pratt Parsing for Operators, Let-In, Case-Of, Lambdas
   │
   ▼
AST.Source (Pure syntactic representation preserving source spans & raw names)
```

### Key Architecture Principles Adopted from Elm:
1. **Separation of `AST.Source` and `AST.Canonical`**:
   * **`AST.Source`**: Direct representation of parsed text. Preserves original source regions (`Span`), raw unqualified names, and unresolved operator infix chains.
   * **`AST.Canonical`**: Formed *after* parsing during semantic analysis (resolving imports, variable scopes, type-checking, and operator fixity).
2. **Indentation State (`Parse.Space`)**:
   * Elm uses off-side layout rules instead of curly braces. The parser state maintains an **Indentation Stack** (`Row`, `Column`). Every block (`let...in`, `case...of`, `type`) pushes its starting column to the stack to validate nested indentation.
3. **No Automatic Backtracking**:
   * Elm avoids arbitrary backtracking combinators. A branch is committed as soon as a key token is consumed, yielding precise compiler error locations.

---

## 3. Full Comparison: Knot vs. Elm AST Syntax Differences

| Feature | Elm Syntax | Knot Syntax | AST Parser Impact |
| :--- | :--- | :--- | :--- |
| **Annotations / Layout Metadata** | None | `@name(args)`, `@{ key = val }`, postfix `expr @{...}` | Node definitions & expressions are wrapped in `Annotated<T>` containing `AnnotationSet`. |
| **Typed Expression Holes** | Syntax Error | `_` or `_name` allowed in expressions | First-class `Expr::Hole(Option<String>)` AST node. |
| **Pattern Match Discard** | `let _ = expr` is a warning/error | `let _ = expr` is explicitly allowed | Allowed in `Pattern::Wildcard` for let-bindings. |
| **Pattern Aliases** | `(x :: xs) @ list` | `(x : xs) as list` | Pattern parser uses `as` keyword instead of `@`. |
| **List Cons Operator** | `::` for cons, `:` for type sig | `:` for cons, `::` for type sig | Swapped token meanings matching Haskell style. |
| **Custom Operators** | User-defined infix allowed (`<\|>`, `=>`) | **Forbidden**. Fixed set (Precedence 0–9) | Lexer/Parser rejects custom operator symbols. |
| **Pipe / Composition Operators** | Supports `<\|`, `<<`, `=<<`, `\|>`, `>>` | Supports **only** `\|>` (forward pipe) and `>>` (forward composition) | Simplifies operator precedence table. |
| **Typeclass / Interface Signatures** | Constrained type vars (`number`) | Explicit `interface` decls & constraints `(Num a) => ...` | `Parse.Type` handles constraint contexts. |
| **Record Pattern Shorthand** | `{ x, y }` pattern destructuring | **Forbidden**. Must use `r as pt -> pt.x` | Parser rejects shorthand record patterns. |
| **`where` clauses** | Not allowed | Not allowed | Matches Elm's `let...in` only scoping. |

---

## 4. Implementation Plan for `knot-parser` (Rust Crate)

`knot-parser` will be implemented as a Rust crate using **`winnow`** (a fast, zero-allocation parser combinator library in Rust that mirrors Elm's `Parse.Primitives`).

### Crate Directory Layout
```
knot-parser/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── span.rs              // Position (line, col), Span (start, end), Located<T>
    ├── ast/
    │   ├── mod.rs
    │   ├── module.rs        // ModuleHeader, Import, Exposing
    │   ├── declaration.rs   // Declaration, ValueDecl, TypeDecl, TypeAlias
    │   ├── expression.rs    // Expr, Pattern, Hole, BinOp, AnnotationSet
    │   └── types.rs         // TypeExpr, InterfaceConstraint
    ├── lexer/
    │   ├── mod.rs           // Tokens, Identifiers, Keywords, Literals
    │   └── state.rs         // Indentation stack & layout state
    ├── parser/
    │   ├── mod.rs           // parse_module entry point
    │   ├── space.rs         // Whitespace, comments (-- and {- -}), column layout
    │   ├── annotation.rs    // @name(...) and @{...} parser
    │   ├── decl.rs          // Declarations & signatures
    │   ├── type_expr.rs     // Type signatures and ADTs
    │   ├── pattern.rs       // Pattern matching expressions
    │   └── expression.rs    // Pratt Parser for expressions, let-in, case-of
    └── error.rs             // Rich compiler diagnostics & error spans
```

### Execution Phasing for `knot-parser`:

1. **Phase 1 — Lexer & Layout Engine (`lexer/`, `space.rs`)**:
   * Lex keywords, identifiers, symbols (`::`, `:`, `|>`, `>>`, `@`).
   * Implement comment handling (`--` single line, `{- -}` nested block comments).
   * Implement column layout tracker for indentation enforcement (`let`, `case`, `type`).
2. **Phase 2 — AST Data Structures (`ast/`)**:
   * Define `AST.Source` types (`Module`, `Declaration`, `Expr`, `Pattern`, `TypeExpr`, `AnnotationSet`).
3. **Phase 3 — Annotations & Declarations Parser (`annotation.rs`, `decl.rs`)**:
   * Parse `@name(args)` and `@{ key = val }` block annotations.
   * Parse top-level signatures `name :: Type` and function bindings `name arg1 arg2 = expr`.
   * Parse ADT definitions (`type Shape = Circle Float | ...`).
4. **Phase 4 — Expression Pratt Parser (`expression.rs`, `pattern.rs`)**:
   * Parse literals, identifiers, tuples, lists, records.
   * Parse typed holes (`_`, `_name`).
   * Implement Pratt Parsing / Shunting Yard for operators (precedence levels 0–9).
   * Parse control expressions (`let...in`, `case...of`, `if...then...else`, `\x -> ...`).
5. **Phase 5 — Test Suite & Roundtrip Validation**:
   * Unit tests for each syntax construct.
   * AST snapshot tests against sample `.knot` source files.
