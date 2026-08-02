# Knot — Code → AST Module: Research & Implementation Notes (Claude, 2026-07-26)

Companion to `agy-gemini-ast-impl-notes-7-26-2026.md` (produced concurrently by a Gemini
session on the same task). Where the two disagree, it's called out explicitly below rather
than silently resolved — see §5.

Spec fixes from this pass already landed in `language-spec-notes.md` itself (§2.2/§2.3,
§3.8, §4.8, §4.9, §9, §10). This file holds the research and reasoning that don't belong
in the spec: Elm's actual internals, the full Knot/Elm diff, and the Rust build plan.

**Addendum (later same day):** §10.3's inline annotations were changed from postfix to
prefix after this doc was written — `@ann` now precedes the atom it binds to
(`@nodeId("n1") f` rather than `f @nodeId("n1")`), for consistency with the
already-prefix §10.1/§10.2 forms and because "closest *following* atom" is a more
intuitive binding rule than "closest *preceding* atom" was. Every other decision in this
file is unaffected — this only touches the annotation grammar's direction, not its
existence or its atom-binding-only scope. The build-order note in §4 below reflects this.

---

## 1. Spec fixes applied, and why

| # | Issue | Resolution | Where |
|---|---|---|---|
| 1 | Cons shown as `::` in the §4.8 table, contradicting `:` used everywhere else (§2.1, §4.6) | Table fixed to `:`. Confirmed against real Elm source: Elm itself uses `:` for signatures / `::` for cons — Knot deliberately inverts that, so the two tokens must not collide. | §4.8 |
| 2 | `.` and `$` claimed absent (§2.2) but `.` composition was defined in three other places (§2.1, §3, §4.2) and `$` sat unexplained in the §4.8 table | Both `.` and `$` are omitted, full stop. Removed the "Spaced Dot Composition" bullet from §2.1, the dot-access composition clause in §3, the "`.` still available" sentence in §4.2, and the `$` row from §4.8. `.` survives only as unspaced record field access. | §2.1, §2.2, §2.3, §3, §4.2, §4.8 |
| 5 | No unary-minus/negative-literal rule | Follows Elm exactly: whitespace-before-but-not-after `-` is negation (`f -1` = `f (-1)`), otherwise subtraction or a parse error. One paragraph, no elaboration — this is a "matches Elm" item, not a departure. | §4.9 |
| 7 | `@name(args)` desugaring unstated | One line: single arg → bare value, multiple args → tuple. | §10 (8.1) |
| 8 | Tuple arity unbounded in an earlier draft | Reverted to capped at 3 (pairs/triples only), matching Elm. | §3.8 |
| 10 | Multi-clause function definitions (Haskell `f 0 = ...`/`f n = ...`) never addressed | Not supported — one bullet added to §2.3 "Omitted from Haskell/Elm" (not duplicated elsewhere). | §2.3 |
| 11 | Module-name/file-path convention, wildcard `import X exposing (..)` | Both stated to match Elm, one line each, placed next to the relevant syntax in §9. | §9 |

**Deliberately left out of the spec, kept here instead:** a full lexical grammar (identifier
charset, reserved-word list, numeric-literal grammar, string escapes, layout/indentation
algorithm) and the annotation atom-binding precedence rule beyond what's already implicit in
the examples. None of these are contradictions or departures from Elm — they're places where
Knot straightforwardly follows Elm's own lexer, so per the "only elaborate on genuine
departures" principle they belong in the *implementation* notes (below and in the Rust plan),
not in the language spec itself.

---

## 2. How Elm's compiler actually parses (verified against `elm/compiler` source)

Read directly from `github.com/elm/compiler`, `master` branch, `compiler/src/`: `Parse/Primitives.hs`,
`Parse/Space.hs`, `Parse/Variable.hs`, `Parse/Number.hs`, `Parse/String.hs`, `Parse/Expression.hs`,
`Parse/Pattern.hs`, `Parse/Type.hs`, `Parse/Declaration.hs`, `Parse/Module.hs`, `Parse/Symbol.hs`,
`AST/Source.hs`, `Reporting/Annotation.hs`, `Canonicalize/Expression.hs`.

**No separate lexer, no separate layout pass.** Elm is one hand-rolled recursive-descent parser
over raw bytes. Indentation-sensitivity is enforced inline, at the exact grammar point that
needs it, via two primitives threaded through parser state (`Parse/Space.hs`):

```haskell
checkIndent  :: ... -> Parser x ()   -- current column > reference indent (continuation line)
checkAligned :: ... -> Parser x ()   -- current column == reference indent (next block item)
```

A `let` block snapshots the column of its first binding as the reference indent, then calls
`checkAligned` before each subsequent binding and `checkIndent` for continuation lines.
`case...of` and top-level declarations work identically. There's no global layout algorithm to
port — it's a small, local, directly-portable primitive. This is the mechanism the Rust
`ParseState`/`check_indent`/`check_aligned` design below is modeled on.

**Binary operator resolution — the most consequential finding for Knot.** Elm does *not*
Pratt-parse operators at parse time, despite what you'd guess from the grammar looking
expression-precedence-shaped. `Parse/Expression.hs` builds a flat, unassociated chain:

```haskell
data Expr_ = ... | Binops [(Expr, A.Located Name)] Expr | ...   -- AST/Source.hs
```

Precedence/associativity gets resolved **later**, in `Canonicalize/Expression.hs`, via a
shunting-yard pass that looks up each operator's fixity from the modules that were imported
(`infix left 6 (+) = add` declarations can live in any package). Elm has to defer this because
operators are ordinary functions and their fixity isn't knowable until you know what's
imported — the parser genuinely cannot resolve `a + b * c` into a tree without first
consulting import info.

**Knot doesn't have this problem**, since §2.3 closes off user-defined operators entirely and
§4.8 fixes the whole table in the spec. The Rust parser can skip Elm's two-phase dance and run
an ordinary precedence-climbing/Pratt parser directly at parse time, producing a properly
nested tree immediately. This is a genuine simplification, not just a style choice, and it's
worth stating plainly since Gemini's parallel plan describes Elm's `Parse.Expression` as doing
"Pratt Parsing for Operators" directly — it doesn't; the Pratt-style resolution only becomes
possible for Knot, precisely *because* Knot removed the feature (importable operator fixity)
that forced Elm's two-phase design. Good news either way, just want the reasoning on record.

**Other confirmed details:**
- Int vs. Float is decided lexically at scan time (fork on `.`/`e`/`E`), never deferred to the
  type checker.
- Elm forbids **Float literal patterns** in `case` — unsound equality. Knot doesn't currently
  say this anywhere; not in the punch list for this pass, but worth a one-line addition
  whenever pattern-matching semantics get revisited (it's a "matches Elm" item, not a spec
  contradiction, so it was left out of this pass's spec edits per the minimalism instruction).
- Reserved words are a plain fixed set checked after scanning a lowercase identifier;
  identifier continuation chars are ASCII alnum + `_` + Unicode alphabetics — no trailing `'`
  (unlike Haskell).
- AST nodes are plain enums wrapped in a `Located`/region-tagged wrapper — no name resolution,
  no operator resolution yet. This "Source AST" (not Elm's later Canonical AST) is the right
  reference point for Knot's code→AST module, since that's the same scope.
- Error handling is one bespoke ADT variant per grammar production carrying full context — the
  source of Elm's well-regarded error messages, at the cost of a lot of enum surface area.
- `elm/compiler` is BSD-3-Clause; Knot is MPL-2.0. Only matters if code gets transcribed rather
  than reimplemented from understanding — worth a mental note, not a blocker.

---

## 3. Knot vs. Elm — full spec (not trimmed to MVP)

| Category | Items | Notes |
|---|---|---|
| Reusable ~1:1 | records/updates, extensible records, ADTs, tuples (now capped at 3, matching Elm exactly), lists, `let`/`case`/`if`/lambda layout, qualified imports + `exposing` (+ wildcard), module header shape, `as`-aliases | Elm's `Parse.Type`/`Pattern`/`Declaration`/`Module` are close templates |
| Reusable mechanism, swapped token | type signatures (`:`→`::`), cons (`::`→`:`) | Same logic, tokens inverted on purpose |
| Simpler than Elm | operator resolution (Pratt at parse time vs. Elm's parse-flat-then-resolve-with-imported-fixities), no `port`/effect-module/`Cmd`/`Sub` machinery, no user `infix` declarations, no `.` composition to disambiguate from field access at the operator-token level | Direct payoff of Knot's closed operator set |
| New grammar, no Elm prior art | `do`-notation, `@name(...)`/`@{...}` annotations (incl. atom-binding precedence rule), `unravel`/`solver` semantics, typed holes as *required-invalid* expression-position placeholders | Can still reuse Elm's layout *mechanics* (e.g. `do` borrows `let`'s indent rules) even though the construct is new |
| Omitted vs. Elm | user operators, ports/effects, record-shorthand destructuring, list comprehensions | Less surface area, not more |
| Reintroduced from Haskell (absent in Elm) | bare-word infix `div`/`mod`, closed typeclass-style interfaces | Elm has none of these; grammar is simple but net-new. (`.` composition was *also* considered for reintroduction but ended up rejected — see §1 above.) |

---

## 4. Plan for the Rust `code → AST` module

**Scope**: produces a Source-AST equivalent — spans attached, nothing resolved (no name
resolution, no type checking; operator precedence *is* resolved here, unlike Elm, since the
table is fixed).

**Crate layout**: no Rust exists in this repo yet. New top-level `compiler/` dir (sibling to
`prototype/`) with a Cargo workspace root, starting with one crate — e.g. `compiler/knot-syntax`
— so a type-checker/interpreter crate can be added later without restructuring.

**Core primitives** (Rust-idiomatic version of `Parse.Primitives` + `Reporting.Annotation`, not
a literal port):
- `Span { start: u32, end: u32 }` + `Spanned<T>` wrapper (stands in for `Located`)
- `Cursor { offset, line, col }`, updated incrementally
- `ParseState<'a> { src: &'a [u8], pos: Cursor, indent: u32 }` with `check_indent`/`check_aligned`
  as near-direct ports of `Parse.Space`'s primitives — the one piece of Elm architecture worth
  copying closely, since it's the entire trick for layout-sensitivity without a separate pass
- Recursive descent, hand-rolled vs. a combinator crate: **open decision, see §5** — Gemini's
  plan picks `winnow`; my read is that the indent-threading is bespoke enough that a generic
  combinator library doesn't buy much over hand-rolled, and full control matters for error
  quality (matching why Elm itself didn't use one either). Worth deciding deliberately rather
  than defaulting either way.

**AST**: `Expr`/`Pattern`/`Type`/`Decl`/`Module` enums shaped like `AST.Source`, plus Knot-only
additions (`DoBlock`, `Annotation` fields), with a properly-nested binary-op tree from day one
(not Elm's flat `Binops` chain — Knot can afford to skip that intermediate representation
entirely, per §2 above).

**Build order** (bottom-up):
1. Span/Cursor/indent primitives
2. Identifiers, keywords, literals (int/float/string; no `Char` — not in the primitive-type
   table)
3. Type-expression grammar
4. Pattern grammar
5. Expression grammar: atoms → application → binary ops (precedence-climbing over the §4.8
   table) → layout-heavy forms (`if`/`case`/`let`/lambda/`do`)
6. Declarations, module header, imports/`exposing`
7. Annotation grammar layered on last (atom-attachment rule from §10 governs where prefix
   `@ann` binds — see the addendum at the top of this doc)
8. `parse_module` entry point — bail on first error for v0, matching Elm's own behavior

**Testing**: the spec's own code blocks are a ready-made golden-file corpus; add targeted cases
for every ambiguous spot resolved above (negative numbers, dot-access-vs-nothing-else now that
composition is gone, annotation attachment scope, tuple-arity rejection past 3).

---

## 5. Open decisions / disagreements with the parallel Gemini plan

Not resolved here on purpose — flagging for you to pick:

1. **Hand-rolled recursive descent vs. a combinator crate (`winnow`).** Gemini's plan picks
   `winnow`. I lean hand-rolled, matching Elm's own choice, mainly for error-message quality and
   because the indent-checking state doesn't compose cleanly through generic combinators. Not a
   strong objection — `winnow` is a fine crate — just don't want the choice made by default via
   whichever notes file gets read last.
2. **Elm's operator-parsing architecture** — Gemini's doc describes `Parse.Expression` as doing
   "Pratt Parsing for Operators" directly, matching Elm. Based on actually reading
   `AST/Source.hs` and `Canonicalize/Expression.hs`, that's not what Elm does (see §2) — Elm
   defers precedence resolution to a post-parse canonicalization pass because operator fixity
   can come from imports. Doesn't change the Rust plan (Knot can Pratt-parse directly either
   way, since its operators are closed), but worth knowing the Elm comparison itself was
   slightly off in that doc.
