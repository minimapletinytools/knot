# Proposal: record spread (`..Name`) for reusing a closed record's fields

Status: proposal / not implemented. Motivated by `knotty-drawings.knot`, where seven
primitive shape types (`Rect`, `Circle`, `Ellipse`, `Line`, `Polyline`, `Polygon`,
`Path`) each need every field of a shared `GraphicsElement` bundle plus their own
geometry, and today that means hand-duplicating ~23 fields per shape.

## 1. The gap this fills

knot's extensible records (`{ a | field : Type }`) give structural, row-polymorphic
constraints for free — any record with a superset of the required fields
automatically satisfies one, with no explicit "implements" declaration. But the
extension slot only ever accepts a bare row *variable*: both the parser
(`type_record` in `compiler/knot-syntax/src/parse/ty.rs`, which parses the
extension via `lower_ident()`) and the AST (`Type::Record(Vec<(String, Type)>,
Option<String>)` in `compiler/knot-syntax/src/ast/ty.rs`) only ever let that slot
hold a variable name, never a reference to an already-defined record type. There is
currently no way to say "this new type is everything `GraphicsElement` has, plus
these extra fields."

This is not a knot-specific gap — Elm has the identical restriction, for the
identical reason. It's the standard ceiling of the "cheap" style of row
polymorphism used across the whole ML/Elm family: field membership is resolved as
a side effect of ordinary unification (`unify_record` in
`compiler/knot-checker/src/unify.rs`), with no separate solver and nothing to
declare. Going further — true row *concatenation*, where two still-open,
not-yet-concrete rows get merged and a disjointness proof has to flow through
arbitrary polymorphic code — is what forces languages like PureScript to promote
field membership into an actual typeclass-style constraint (`Union`, `Lacks`,
resolved by the compiler's general instance-solving machinery). That's real added
complexity for a capability this proposal doesn't need — see §5.

## 2. The proposal

Add a spread member inside a record type literal: `..Name`, where `Name` is
another record type alias. It expands, at the point the containing alias is
resolved, to every field `Name` declares.

**Closed record:**
```
type alias Circle = { ..GraphicsElement, cx : Float, cy : Float, r : Float }
```
is exactly equivalent to writing out `GraphicsElement`'s fields by hand today,
plus `cx`, `cy`, `r`.

**Still-extensible record** (keeps its own open row variable, so it remains usable
as a constraint elsewhere):
```
type alias Named a = { a | ..GraphicsElement, label : String }
```

Multiple spreads in one literal are allowed:
```
type alias Combined = { ..Fills, ..Strokes, extra : Bool }
```

## 3. Semantics

- **Spread target must be closed.** `Name` must resolve, after following its own
  alias chain, to a record with `Option<String>` extension `= None`. Spreading an
  already-open alias (e.g. `IsGraphicsElement a` itself, which has its own
  unresolved `a`) is a compile error — there's nothing concrete yet to splice.
  This restriction is what keeps the feature at "eager substitution," not "row
  unification": the compiler never has to reason about two *unknown* rows.
- **Field-name collisions are always a hard error** — between a spread and an
  explicit field, or between two spreads. No merge-by-compatible-type fallback
  (unlike TypeScript's `&`), no shadowing, no "last one wins."
- **Resolution is eager and one-shot**, happening once when the containing alias
  is expanded — not deferred, not re-checked per use site. After expansion,
  `Circle` is an entirely ordinary closed record; nothing downstream (pattern
  matching, field access, record update, unification) needs to know it was ever
  written with a spread.

## 4. Where this plugs into the compiler

knot already has a dedicated whole-module alias-expansion pass —
`compiler/knot-canonical/src/resolve/alias.rs` — that substitutes every alias
reference with its resolved definition, and already treats an unexpandable case
(`type alias Bad = Bad`) as a hard error rather than looping. Spread slots
directly into this existing pass:

1. it already resolves each alias's body to concrete structure before any use;
2. when a record literal's body contains a spread entry, look up that name's own
   already-expanded body;
3. require its extension to be `None` (closed), else error;
4. union the field maps, erroring on any name collision.

This never touches `unify.rs` (row unification) or the interface/instance table
(`compiler/knot-checker/src/interface`) — it's a syntax/alias-expansion-level
feature, fully resolved before type inference starts.

### Grammar sketch

```
-- current
record : '{' [ ident '|' ] field (',' field)* '}'
field  : ident ':' type

-- proposed
record : '{' [ ident '|' ] member (',' member)* '}'
member : field | spread
field  : ident ':' type
spread : '..' UpperIdent
```

## 5. Non-goals

- **No spreading two still-open rows into each other** (no true row
  concatenation). This is exactly the case that forces PureScript into
  typeclass-based `Union`/`Lacks` solving — deliberately not chasing it.
- **No field removal / row restriction.**
- **No duplicate-label or scoped-label semantics.** A collision is always an
  error, never resolved by ordering or shadowing.
- **No spreading anything but a record type** — not an ADT, not a function type.

## 6. Operator choice: why `..Name`, not `++` or `&`

- **`++` (rejected).** In knot (as in Elm/Haskell), `++` already means list
  concatenation — an *ordered, sequential* append. A record's fields aren't a
  sequence being appended, they're an unordered set being merged; reusing `++`
  imports a "these are in order and it matters" connotation that's actively wrong
  for this operation.
- **`+` (rejected).** Too easy to misread as "or" rather than "and," especially
  right next to ADT variant syntax that already uses `|` for choice (`type Paint =
  NoPaint | SolidColor Color | ...`). `A + B` reads ambiguously close to "A or B."
- **`&` / TypeScript-style intersection (considered, not chosen).** A real
  contender — `type alias Circle = GraphicsElement & { cx : Float, ... }` is
  well-precedented and reads fine at the top level. Ranked below `..Name` for two
  reasons:
  1. It needs a brand-new infix type operator with its own precedence rules. `..`
     inside `{ }` costs nothing new grammatically — it's just one more kind of
     entry in a list the record-literal grammar already has.
  2. It doesn't compose cleanly with the *open* case — `A & { a | field : T }`
     raises a real question of how `&`'s precedence nests against the extension
     bar `|`, which needs its own new rule. `{ a | ..A, field : T }` needs none:
     spread is just another member of the same list the extension variable
     already introduces.
- **Bare `Name`, no marker (considered, not chosen).** Grammatically unambiguous
  on its own — a spread entry (capitalized, no `:`) can't collide with a field
  entry (lowercase, always `:`). Rejected anyway because a stray capitalized
  identifier sitting in a field list, with nothing marking it as intentional,
  reads as a typo on first glance. `..` is two extra characters that remove that
  ambiguity for a human reader, even though the parser never needed them.
- **`..Name` (chosen).** Directly precedented by Rust's struct-update syntax
  (`Foo { x: 1, ..base }` — "fill the rest of these fields from `base`"), applied
  at the type level instead of the value level. Same meaning, familiar to anyone
  who's used it, and visually distinct from both a field entry and the extension
  variable (which always sits immediately before `|`), so it can't be misread as
  either.

## 7. Worked example (`knotty-drawings.knot`)

Before (current state of the file — every shape restates all 23 shared fields):
```
type alias Circle =
    { id : Id
    , transform : Transform
    , fill : Paint
    , stroke : Paint
    -- ...19 more shared fields...
    , cx : Float
    , cy : Float
    , r : Float
    }
```

After, with a closed `GraphicsElement` fields record kept alongside the existing
open `IsGraphicsElement a` constraint:
```
type alias GraphicsElement =
    { id : Id
    , transform : Transform
    , fill : Paint
    , stroke : Paint
    -- ...19 more shared fields...
    }

type alias IsGraphicsElement a = { a | ..GraphicsElement }

type alias Circle = { ..GraphicsElement, cx : Float, cy : Float, r : Float }
type alias Rect    = { ..GraphicsElement, x : Float, y : Float, width : Float, height : Float, rx : Option Float, ry : Option Float }
-- ...same pattern for Ellipse, Line, Polyline, Polygon, Path
```

Note `IsGraphicsElement a` itself becomes a spread consumer too — `{ a |
..GraphicsElement }` — collapsing the current hand-maintained duplication between
the closed fields record and the open constraint down to a single source of
truth.

## 8. Open questions

- Should a spread be allowed to appear more than once for the *same* target in
  one literal (`{ ..A, ..A, x : Float }`)? Leaning toward: yes, and it's simply a
  no-op the second time — but worth deciding explicitly rather than leaving
  implicit.
- Does this want to compose with a future "closed → open" or "open → closed"
  conversion, or stay entirely separate from that question?
