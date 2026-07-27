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

**Out**: the `unravel`/`solver` annotation keys' own type-checking rules (spec §11 is
explicitly unreviewed — "TODO review this section, you never reviewed this lol" — designing
their expected-type rule now would be building on sand; see §7's extension point instead),
user-defined interfaces (v2, spec §14), evaluation/execution (Twine's job), and node-identity
hashing (the earlier hashing discussion's own conclusion was that it needs *this* stage's
output first — see §6's closing note).

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
- **Crate name: `knot-types`**, continuing the naming pattern (`knot-syntax` parses text
  into syntax; `knot-canonical` resolves names into the canonical form; `knot-types` checks
  and elaborates types). Workspace member alongside the other two, depending on
  `knot-canonical`.

---

## 6. Crate & module layout

```
compiler/knot-types/
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
  already-validated `CInstanceDecl`s, coherence + superclass-existence checks.
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
4. **Confirm the crate name** (`knot-types`) and this plan's scope split before I start
   TM0 — same checkpoint pattern as the parser plan.
