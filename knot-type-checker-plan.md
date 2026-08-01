# Knot Type Checker — Implementation Plan

Covers the crate that turns a `knot-canonical` `CModule` into a fully type-checked,
dictionary-elaborated AST — "type analysis + dictionary passing stuff" from `TODO.txt`,
the step after `knot-canonical`. This is a **plan only**, not yet implemented — see §8 for
what's genuinely open vs. decided.

Basis for this plan: real source from `elm/compiler` (`Type/Type.hs`, `Type/UnionFind.hs`,
`Type/Unify.hs`, `Type/Solve.hs`, `Type/Constrain/Module.hs`, `Type/Constrain/Expression.hs`,
`Canonicalize/Module.hs`) fetched and read directly for this plan, the same way
`knot-ast-parser-plan.md` was grounded in real Elm parser source rather than memory. Elm
has no typeclasses at all, so the interface/instance/dictionary-passing design below is
**not** Elm-derived — it's the standard dictionary-passing translation (Wadler & Blott;
this is also how GHC compiles Haskell typeclasses) adapted to Knot's closed interface set.

---

## 1. Scope

**In**: Hindley-Milner type inference over the Canonical AST (literals through `case`/`do`/
records/extensible records), unification with an occurs check, let-polymorphism
(generalization) for both `let` expressions and top-level module bindings, checking
`Constraint`s (`Ord a =>`) against the closed interface set, building an instance table from
built-ins + `instance` declarations, superclass-existence checking, and a dictionary-passing
elaboration pass that produces a fully-typed AST with every interface method call resolved to
an explicit dictionary argument.

**Out**: `unravel`'s/`solver`'s own runtime semantics and the push-forward/constraint-
propagation solving algorithm (Twine's job, not a type-checking concern) — but *not* their
type-checking rule anymore. A full design pass on `unravel` happened since this plan was
first written (`7-29-2026_unravel_discussion.md`), settling on a concrete signature shape;
see §3.5 for what that means for this crate. Still out: user-defined interfaces (v2, spec
§14), evaluation/execution (Twine's job), and node-identity hashing (the earlier hashing
discussion's own conclusion was that it needs *this* stage's output first — see §6's
closing note).

---

## 2. What carries over from Elm, and what doesn't

Elm's type checker (`Type.Constrain.*` generates a `Constraint` tree from the Canonical AST;
`Type.Solve` walks it, calling `Type.Unify` against a mutable union-find substitution) is a
genuinely good architecture and Knot's checker follows its shape closely: **separate
constraint generation from constraint solving**, rather than a single recursive
infer-and-unify-as-you-go function. This keeps each half simple and independently testable,
exactly the reason `knot-syntax` already keeps parsing and validation separate.

What transfers directly:
- The `Constraint` AST shape (`CEqual`, `CAnd`, `CLet{rigid_vars, flex_vars, header,
  header_con, body_con}`) as the generation target — `CLet` is *the* mechanism for both
  scoping and generalization: it's how `let` and top-level bindings introduce names whose
  type the rest of the program can reference.
- Union-find-based mutable type variables with an occurs check, and extensible-record
  unification via field gathering (partition into fields both sides have vs. fields only one
  side has, unify the "only" sides against fresh extension variables) — Knot's extensible
  records (spec §3.4) are the same feature Elm's `Record1`/row-polymorphism already solves.
- Splitting top-level (and `let`-block) bindings into dependency-ordered groups via
  strongly-connected-components over the *call graph*, so each independent binding gets
  generalized (full let-polymorphism) before the next one is checked, rather than forcing
  every binding in a file into one giant simultaneously-solved group. Elm does this during
  canonicalization (`Canonicalize.Module.detectCycles`, literally `Data.Graph.stronglyConnComp`
  over each value's referenced names) — Knot does the equivalent, but as an early step of
  *this* crate instead, since `knot-canonical`'s job stops at "does every name resolve," and a
  binding's dependency graph in the *type-inference* sense (which types depend on which)
  isn't something scope resolution needs to know about at all — see §5's decision record.

What doesn't transfer:
- **Rank/pool-based generalization** (`Type.Solve`'s `generalize`/`adjustRank`, an
  OCaml/SML-style "levels" optimization that avoids a full tree walk at every `let`
  boundary by tracking each variable's binding depth incrementally). This is real
  complexity purely in service of performance at scale. Given the standing "simplicity over
  performance" instruction, §4 specifies the textbook alternative instead: generalize a
  binding's type by computing `ftv(type) \ ftv(env)` directly (free variables of the
  inferred type, minus free variables still floating around in the ambient environment)
  after each binding group is solved. Slower on huge programs, far simpler to implement and
  reason about — the right trade for Knot's expected program sizes.
- **`FlexSuper`/`SuperType`** (Elm's hardcoded, closed stand-in for exactly four
  ad-hoc-polymorphic concepts: `number`, `comparable`, `appendable`, `compappend`, since Elm
  has no real typeclasses to express them properly). Knot doesn't need this at all, for two
  independent reasons: (1) Knot's interfaces are real, dictionary-passing, closed-set
  typeclasses (§3), a strict generalization of what `FlexSuper` fakes; (2) Knot doesn't even
  have Elm's numeric-literal-defaulting problem in the first place — `knot-syntax`'s lexer
  already commits a literal to `Int` or `Float` at parse time based on the presence of a
  decimal point (`lex/literal.rs`), unlike Haskell/Elm's `fromInteger :: Integer -> a`-style
  polymorphic numeric literals. One less thing to design *and* one less thing Knot inherits
  from Elm's own workarounds.
- **The mutable-`IORef`-graph `Point`/`PointInfo` union-find representation**
  (`Type.UnionFind`). Directly portable to Rust via `Rc<RefCell<_>>`, but §4 specifies the
  more idiomatic Rust shape instead: an arena (`Vec<Slot>`) owned by the inference engine,
  with variables as plain `u32` indices and union-by-rank/path-compression done by mutating
  slots in place. Same algorithm, no reference-counted interior mutability needed.
- **Elm's "bad recursion" check** (`Canonicalize.Module.detectBadCycles`, a *second*,
  finer-grained cycle detection pass using only *direct* dependencies, which rejects a
  self-referencing non-function value like `x = x + 1` at compile time — necessary because
  Elm is strictly evaluated, so a genuinely-cyclic non-function value would loop forever the
  moment the module loads). Knot is lazy (spec §1), so this restriction simply doesn't apply
  — `x = x + 1` is exactly as fine (and exactly as likely to diverge only when *forced*) as
  it is in Haskell. Confirmed by reading the actual check, not assumed.
- **The `Map.Map Name.Name Can.Annotation` side-table output.** `Type.Solve.run` returns
  just each top-level binding's principal type scheme, not a fully re-annotated expression
  tree — sufficient for Elm, which never needs to rewrite call sites (no dictionaries to
  insert). Knot's checker needs to actually **elaborate** the tree (insert dictionary
  arguments at every constrained call site — see §3), so its output is a genuine third AST
  variant, not just a side table. A deliberate divergence, not an oversight.

---

## 3. Interfaces: dictionary-passing, since Elm has none

Knot's closed interface set (`Eq`, `Ord`, `Show`, `Semigroup`, `Monoid`, `Num`,
`Fractional`, `Integral` — spec §7) needs real ad-hoc polymorphism with user-defined
instances, which Elm's type system has no equivalent of at all. The standard technique
(Wadler & Blott 1989; this is also, in essence, how GHC compiles typeclasses) is:

- A **constraint** in a signature (`Ord a => a -> a -> a`) becomes, at elaboration time, an
  extra implicit parameter: a **dictionary** — a small record of the interface's methods,
  specialized to whatever `a` turns out to be at each call site.
- Because the interface set is *closed* (no user-declared interfaces, ever, in v0 — spec
  §2.3), every interface's method set is known statically. This can be a small hardcoded
  table exactly like `knot-canonical::prelude`'s `BUILTIN_INTERFACES`/method lists, just one
  layer richer (method name **and** type shape, plus each interface's declared superclass:
  `Ord` implies `Eq`, `Fractional`/`Integral` imply `Num`, `Monoid` implies `Semigroup`).
- A global **instance table**, keyed by `(interface, head type constructor)`, seeded with
  the built-in instances the spec already lists (`Num Int`, `Num Float`, `Integral Int`,
  `Fractional Float`, and eventually `Eq`/`Ord`/`Show` for every built-in type that needs
  them) plus every `instance` declaration `knot-canonical` already resolved (its `target`
  type and `interface` name are already validated — see `knot-canonical`'s
  `CInstanceDecl`). A parametric instance (`instance Eq a => Eq (List a)`) is stored as a
  dictionary-*constructor* rather than a concrete dictionary: building `List a`'s `Eq`
  dictionary recursively requires `a`'s own `Eq` dictionary first — precisely how GHC
  resolves `deriving`/instance context, and precisely why this needs to be a real algorithm
  and not just a lookup.
- **Coherence**: like Haskell without extensions, at most one instance per `(interface,
  head type)` pair, checked when building the table (`instance Eq Shape` declared twice ->
  error) — the same simplicity trade already made elsewhere in this project (closed
  interface set, no user operators, no multi-clause functions): fewer degrees of freedom,
  much less to get wrong.
- **Superclass existence**: declaring `instance Ord Shape` requires an `Eq Shape` instance
  to already exist in the table — checked once, here, not in `knot-canonical` (which
  deliberately deferred this exact question — see its `resolve_instance` doc comment).
- **When this actually runs**: after type inference finishes and every type variable is
  resolved (so it's known *exactly* which concrete type each constrained call site needs),
  a separate elaboration walk resolves each signature's constraints against the instance
  table and rewrites the call to pass the resolved dictionary explicitly, producing the
  Elaborated AST (§6). Constraint *checking* (does a resolvable instance exist at all) can
  happen earlier, during solving, so type errors and "no instance for `Ord Shape`" errors
  are reported together rather than in two disconnected passes.

This elaborated form — the one place where identical surface syntax (`a == b`) can
compile to genuinely different runtime behavior depending on the inferred type — is exactly
the representation the earlier node-identity-hashing discussion concluded was necessary for
a *sound* content hash (plain Canonical AST isn't enough, because it doesn't yet know which
dictionary a given `==` dispatches to). This plan's output is that representation; the
hashing scheme itself remains future work, not part of this crate.

**Deliberately deferred, not designed**: whether a constrained top-level binding gets a
monomorphism-restriction-style single evaluation shared across all its uses, or is free to
re-elaborate (and re-evaluate) per instantiation. Haskell's monomorphism restriction exists
specifically to avoid silently repeating expensive work at each use of a polymorphic CAF —
given Knot's node-graph/lazy-sharing model, this has real runtime implications beyond type
checking, so it belongs with the runtime/partial-reevaluation design, not decided here.

---

## 3.5 Annotation type-checking (`unravel` and friends)

Not identified as a gap until the `unravel` design pass (`7-29-2026_unravel_discussion.md`)
worked through a concrete signature — worth folding in now while it's fresh, even though
full implementation still waits on spec §11 stabilizing further.

**The general gap this surfaces**: nothing in §§1–3 covers type-checking an *annotation's*
value against an expected type. `knot-canonical` deliberately resolves annotation values as
ordinary expressions regardless of key (name resolution doesn't need to know a key's
expected type) — but *this* crate does, once it's checking rather than just resolving.
Most standard keys (spec §10.4 — `nodeId :: String`, `position :: (Float, Float)`, `label
:: String`, `collapsed :: Bool`, ...) have a fixed expected type, trivial to check. `unravel`
is the hard case: its expected type isn't fixed, it's *derived* from the annotated binding's
own signature.

**The derivation rule**, given the settled signature shape: for
`f :: A -> B -> C -> Out`, the `unravel` key's value must have type

```
Sensitivity Out -> UnravelInput A -> UnravelInput B -> UnravelInput C -> Option (A, B, C)
```

mechanically substituting `f`'s own param/return types into a fixed template. This needs a
small, closed table — structurally the same shape as §3's interface-method table, just
keyed by annotation name instead of interface name — mapping a handful of special keys to
"how to derive this key's expected type from context" (a constant for most keys; a
signature-parameterized template for `unravel`, and presumably `solver` once its own shape
gets pinned down per the discussion doc's §8 proposal). Once the expected type is known,
checking the annotation's value against it is ordinary type-checking, *except* for the
`Sensitivity Out` slot in that template, which is not an ordinary type application — see
below.

**Correction, 2026-08-01 — `Sensitivity` is a recursive type-level function, not an
ordinary ADT.** The original resolution below (kept struck-through context intentionally,
so the reasoning that got revisited stays visible) assumed wrapping `Out` wholesale was
enough. It isn't: per spec §9.6, `Sensitivity` must recurse into `Out`'s own record/tuple
*shape* — `Sensitivity { x : Float, y : Float }` reduces to `{ x : Sensitivity Float, y :
Sensitivity Float }`, all the way to scalar leaves, before the leaf-level constraint
vocabulary (`Exact a | Range a a | Tolerance a a | Free`, still TBD — spec §13) ever
applies. A flat `Exact Out | Range Out Out | ...` over the whole `Out` can't express "let
me constrain `x` and leave `y` free" at all, which is the entire point of §9.6 — so this
crate *does* need one genuinely new piece of type-system machinery: a small structural
function, `sensitivity_of` (§4), that pattern-matches a resolved `Structure` and recurses
through `Record`/`Tuple` rather than doing a nominal-type lookup. `UnravelInput` is
unaffected by this correction — it's still an ordinary hand-written generic type alias
(`{ orig : a, hints : List a }`); only `Sensitivity` needed the flat-ADT plan reopened.

Superseded text, kept for the record of what changed and why:

~~**The reassuring part**: `Sensitivity`/`UnravelInput` were deliberately settled on as
ordinary, hand-written ADTs/records (`type Sensitivity a = Exact a | Range a a | Tolerance
a a | Free`; `type alias UnravelInput a = { orig : a, hints : List a }`) rather than an
auto-derived "deep partial" transform — see the discussion doc §6 for why a real generic/
type-family feature was rejected. That means this crate needs *zero* new type-system
machinery for it: no type-level functions that recurse over a type's own field structure,
nothing beyond the HM + closed-interface architecture already planned. The only genuinely
new piece is the small annotation-key → expected-type table itself.~~

**Concrete, not-yet-done follow-ups, deliberately not done as part of this edit**:
- `UnravelInput` and the eventual leaf vocabulary (`Exact`/`Range`/`Tolerance`/`Free`, spec
  §13) need to be added to `knot-canonical::prelude`'s `BUILTIN_TYPES`/`BUILTIN_CONSTRUCTORS`
  once this is actually implemented, same as any other built-in generic type — premature
  before this crate (and the annotation-key table) exist to consume them, and while spec
  §11/§13 are still marked as needing more work. `Sensitivity` itself is different: it still
  needs an arity-1 entry in `BUILTIN_TYPES` so `knot-canonical` can resolve the bare name in
  a signature like `Sensitivity out` (spec §9.1) — but unlike every other built-in type it
  has no data constructors and no fixed stored shape; `knot-canonical` resolves it as an
  opaque nominal head only, and *this* crate never unifies against a stored definition for
  it, special-casing it in `sensitivity_of` (§4) once the concrete type it's applied to is
  known instead.
- Tuple-arity-≤3 interacts with multi-argument `unravel`: `Option (a, b, c)` is fine, but a
  4+-parameter function's unravel needs `Option {a: A, b: B, c: C, d: D}` (a record) instead
  of a bare tuple — the same existing rule (`knot-syntax::validate`) already applies
  uniformly here, not a new restriction, just worth knowing as an authoring convention.
- Annotation *values* elaborating correctly (dictionary-insertion reaching into an
  `unravel`'s own body if it uses interface methods) needs no special-casing — the Elaborated
  AST already mirrors `CAnnotation`'s `value: Spanned<CExpr>` shape, so it falls out for free
  from elaborating the whole tree uniformly (§6/§7).

No new milestone yet — folding this into TM6 once `interface/table.rs` exists, as a sibling
`annotation/table.rs` with the same shape, is the likely spot (see §7).

---

## 4. Core data design

```rust
// -- variables: an arena of indices, not a mutable pointer graph (see §2) --

pub struct TypeVarId(u32);

enum Slot {
    Unbound { rank: u32 },
    Link(TypeVarId),           // union-find redirect (path-compressed on lookup)
    Bound(Structure),          // resolved to a concrete shape
    Rigid { name: String },    // from an explicit signature -- must NOT unify with a
                                // concrete type, only with itself; see §5 on skolemization
}

pub struct Substitution { slots: Vec<Slot> }   // owned by the inference engine

enum Structure {
    App(Ref, Vec<TypeVarId>),                  // Int, List a, Map k v -- Ref from knot-canonical
    Fn(TypeVarId, TypeVarId),
    Tuple(Vec<TypeVarId>),                     // arity already <= 3, checked upstream
    Record(BTreeMap<String, TypeVarId>, Option<TypeVarId>),  // fields + row extension var
    Unit,
}

// -- constraint generation target, mirroring Type.Constraint --

enum Constraint {
    True,
    Equal { span: Span, expected: TypeVarId, actual: TypeVarId },
    /// A signature's `Ord a =>` obligation on a concrete, now-inferred type.
    HasInstance { span: Span, interface: String, ty: TypeVarId },
    And(Vec<Constraint>),
    Let {
        rigid_vars: Vec<TypeVarId>,
        flex_vars: Vec<TypeVarId>,
        header: BTreeMap<String, TypeVarId>,   // names this binding group introduces
        header_con: Box<Constraint>,
        body_con: Box<Constraint>,
    },
}

// -- the instance table (§3) --

struct InstanceTable {
    // (interface name, head type constructor) -> how to build/find the dictionary
    entries: HashMap<(String, Ref), InstanceEntry>,
}
enum InstanceEntry {
    BuiltIn { dict: DictionaryValue },
    Declared { constraints: Vec<CConstraint>, methods: HashMap<String, /* elaborated body */> },
}

// -- Sensitivity: structural, not nominal (§3.5 correction) --
// Only valid once `ty` is fully resolved (post-unification) -- this is why deriving
// `unravel`'s expected type has to happen after solving, not during constraint generation.

fn sensitivity_of(sub: &Substitution, ty: TypeVarId) -> TypeVarId {
    match sub.resolve_structure(ty) {
        Structure::Record(fields, ext) =>
            sub.fresh_bound(Structure::Record(
                fields.iter().map(|(name, field_ty)| (name.clone(), sensitivity_of(sub, *field_ty))).collect(),
                ext,
            )),
        Structure::Tuple(elems) =>
            sub.fresh_bound(Structure::Tuple(elems.iter().map(|e| sensitivity_of(sub, *e)).collect())),
        _ => leaf_sensitivity(sub, ty),   // Exact a | Range a a | Tolerance a a | Free -- §13, TBD
    }
}
```

Generalization (the simplified, non-rank-based alternative to §2's Elm mechanism):

```rust
fn generalize(sub: &Substitution, env: &TypeEnv, ty: TypeVarId) -> Scheme {
    let ftv_ty = free_vars(sub, ty);
    let ftv_env = env.free_vars(sub);           // vars still free in *enclosing* bindings
    let quantified = ftv_ty.difference(&ftv_env);
    Scheme { vars: quantified.collect(), ty }
}
```

Extensible-record unification follows Elm's field-gathering approach directly (concept,
not code): split both sides' fields into shared vs. side-only, unify the shared pairwise,
unify each side's "only" fields against a fresh extension variable representing "whatever
the other side's row turns out to also have."

---

## 5. Architecture decisions

- **Constraint generation separate from solving**, per §2 — matches Elm, matches this
  project's own established preference for small, single-purpose passes over one big
  recursive function.
- **Arena-indexed union-find, not `Rc<RefCell<_>>`** — see §2/§4; more idiomatic Rust,
  avoids reference-counting overhead and the borrow-checker friction of a mutable pointer
  graph, no behavior difference.
- **`ftv`-difference generalization, not rank/pool tracking** — see §2; explicit
  simplicity-over-performance trade.
- **Dependency-group (SCC) splitting lives in *this* crate, not `knot-canonical`** — a
  binding's type-inference dependency graph (which bindings' *types* it needs) is a
  different question from name *scope* (which `knot-canonical` already answers
  completely: every top-level binding is mutually visible regardless of source order,
  correctly, for scope purposes). Elm's own pipeline reaches the same conclusion by
  construction — `Canonicalize.Module` computes the SCC split, but purely so
  `Type.Constrain.Module` has `Declare`/`DeclareRec` to consume; the split's only
  consumer is type inference. Knot's checker computes it itself from the already-resolved
  `Ref::TopLevel`/`Ref::Local` edges in the `CModule` it's given, rather than asking
  `knot-canonical` to grow a type-inference-flavored concern it doesn't otherwise need.
- **No "bad recursion" check** — see §2; laziness makes it unnecessary, confirmed against
  Elm's actual (strictness-motivated) implementation rather than assumed by analogy.
- **Coherent instances only** (§3) — no overlapping instances in v0, matching the
  project's repeated preference for closing off degrees of freedom it doesn't need yet
  (closed interfaces, closed operators, no multi-clause functions).
- **Rigid (skolem) type variables for signatures** — a signature's own type variables
  (`Ord a => a -> a -> a`) must be treated as opaque/rigid during that binding's own body
  check (they can unify with themselves but not silently specialize to a concrete type),
  the same guarantee Elm's `nameToRigid`/`RigidVar` provides and genuine let-polymorphism
  requires; without it, a signature would be a lie the checker doesn't actually enforce.
- **Crate name: `knot-checker`**, continuing the naming pattern (`knot-syntax` parses text
  into syntax; `knot-canonical` resolves names into the canonical form; `knot-checker` checks
  and elaborates types). Workspace member alongside the other two, depending on
  `knot-canonical`.

---

## 6. Crate & module layout

```
compiler/knot-checker/
  Cargo.toml                    (depends on knot-canonical)
  src/
    lib.rs                      -- check_module entry point
    var.rs                      -- TypeVarId, Substitution/Slot arena, union-find ops
    ty.rs                       -- Structure, Scheme, generalize/instantiate
    unify.rs                    -- unify() incl. occurs check + record field-gathering
    constrain/
      mod.rs
      expr.rs                   -- Constraint generation over CExpr (mirrors Constrain/Expression.hs)
      pattern.rs                -- Constraint generation over CPattern
      decl.rs                   -- per-binding-group CLet wrapping, SCC dependency split (§5)
    solve.rs                    -- walks Constraint, calls unify, builds the Env of schemes
    interface/
      mod.rs
      table.rs                  -- closed interface method table (mirrors prelude.rs's shape)
      instance.rs                -- InstanceTable construction + coherence + superclass checks
    annotation/
      mod.rs
      table.rs                  -- annotation-key -> expected-type derivation (§3.5): fixed
                                    type for most keys, signature-parameterized template for
                                    `unravel`/`solver`
      sensitivity.rs             -- sensitivity_of(): structural recursion of `Sensitivity`
                                    into Record/Tuple shape (§3.5/§4) -- the one genuinely new
                                    piece of type-system machinery this crate needs beyond HM
                                    + closed interfaces
    elaborate.rs                -- post-solve pass: resolves HasInstance obligations against
                                    the InstanceTable, inserts explicit dictionary args
    ast.rs                      -- the Elaborated AST (TExpr/TPattern/... with a `ty` on every
                                    node and dictionaries threaded through constrained calls)
    error.rs                    -- TypeError (mismatch, occurs-check/infinite type, no
                                    instance, ambiguous instance, ...), collected not fatal
                                    (matches knot-canonical's CanonError, not knot-syntax's
                                    fail-fast ParseError -- there's no backtracking concept
                                    here either)
```

---

## 7. Build order / milestones

- **TM0** — Crate scaffold; `var.rs` (arena + union-find: fresh/find/union with
  rank + path compression); `ty.rs`'s `Structure`/`Scheme` shapes.
- **TM1** — `unify.rs`: structural unification (`App`/`Fn`/`Tuple`/`Unit`) + occurs check.
  No records yet — get the core algorithm right and tested first.
- **TM2** — Record/row-polymorphism unification (field-gathering, per §4).
- **TM3** — `constrain/expr.rs` + `constrain/pattern.rs`: literals, `Var`/`Ctor` (via
  `CLocal`-equivalent instantiate-a-known-scheme), application, lambda, `if`, `case`,
  tuples/lists/records — everything except `let` and top-level bindings, which need §5's
  SCC machinery first.
- **TM4** — `constrain/decl.rs`: SCC dependency splitting (§5) + `Let` constraint wrapping
  for both `let`-expressions and whole-module top-level bindings; rigid-var handling for
  signatures.
- **TM5** — `solve.rs`: the driver that ties generation to `unify.rs`, builds the
  scheme environment, runs `generalize` at each binding group's boundary, collects
  `TypeError`s instead of stopping at the first (matching `knot-canonical`'s error-collection
  stance, for the same reason).
- **TM6** — `interface/table.rs`: the closed interface/method/superclass table (hardcoded,
  small). `interface/instance.rs`: table construction from built-ins + `knot-canonical`'s
  already-validated `CInstanceDecl`s, coherence + superclass-existence checks. Sibling
  `annotation/table.rs` + `annotation/sensitivity.rs` (§3.5/§4/§6) once `unravel`'s design is
  stable enough to implement — `sensitivity_of`'s Record/Tuple recursion can be built and
  unit-tested against synthetic types before the leaf vocabulary (spec §13) is pinned down,
  so it needn't block the rest of this milestone.
- **TM7** — `elaborate.rs` + `ast.rs`: the post-solve dictionary-insertion pass producing
  the Elaborated AST — the crate's actual deliverable.
- **TM8** — Built-in instance wiring (`Num Int`, `Num Float`, `Integral Int`,
  `Fractional Float`, `Eq`/`Ord`/`Show` for primitives and built-in containers) +
  `fromIntegral`.
- **TM9** *(stretch, lower priority — spec explicitly only wants a warning, not an error)*
  — pattern-match exhaustiveness/redundancy checking, algorithmically Maranget's
  usefulness-checking approach (Elm's `Nitpick.PatternMatches` implements the same
  algorithm) — a warning pass, not part of the core type-checking path, safe to defer past
  TM0–TM8 without blocking anything else.

---

## 8. Test strategy

New self-contained fixtures, not a reuse of the existing `corpus/` — the same reasoning
`knot-canonical`'s own tests landed on: the existing `corpus/valid` fixtures are
deliberately small syntax-focused snippets with free variables (`x = a |> f |> g`), correct
for exercising grammar but meaningless to type-check as-is. Primary vehicle: inline
`#[cfg(test)]` unit tests per module (matching every other crate in this workspace so far),
plus a handful of small self-contained `.knot`-source string tests per milestone (e.g. TM6's
`instance Eq Shape where (==) a b = ...` end-to-end through elaboration, confirming the
dictionary actually gets threaded).

---

## 9. Open questions for you

1. **Monomorphism-restriction-style sharing** (§3's closing note) — does a polymorphic,
   constrained top-level binding need to evaluate once and share the result across
   instantiations, or is per-instantiation re-elaboration/re-evaluation fine? This has real
   runtime/caching implications, not just type-checking ones.
2. **Built-in `Eq`/`Ord`/`Show` coverage** — which built-in types need these out of the box
   beyond the numeric ones spec §6.2 already lists explicitly (presumably `String`, `Bool`,
   `List a` given `Eq a`/`Ord a`, tuples given their components, `Unit`)? Worth a short
   explicit list in the spec rather than the checker silently deciding.
3. **`List.map`-style qualified access** — `knot-canonical`'s prelude docs already flagged
   this as unresolved (is `List.map` the same polymorphic collection-interface `map`
   written with a qualifier, or a distinct concrete function belonging to a real `List`
   stdlib module?). Doesn't block TM0–TM8, but does block designing the built-in
   `List`/`Map`/`String` module *interfaces* properly rather than ad hoc.
4. **Confirm the crate name** (`knot-checker`) and this plan's scope split before I start
   TM0 — same checkpoint pattern as the parser plan.
5. **When does `annotation/table.rs` (§3.5) actually get implemented?** — the mechanism is
   now clear enough to build, but it depends on spec §11 (unravel/solver) settling further
   first, per the discussion doc's own "open threads." Gate it on that, or start it alongside
   TM6 with just the fixed-expected-type keys (`nodeId`, `position`, ...) and leave `unravel`
   itself stubbed until its design is less in flux?
