# Knot Checker — Closing the Fundamental Gaps

Follow-up to `checker_impl_summary.md`. That doc covers what TM0–TM9 built;
this one plans fixes for the gaps that are actually load-bearing (not the
"superficial" ones — `Sensitivity`'s opaqueness and unchecked annotation
values stay deferred, per direction). Written before touching any code, in
the same spirit as `knot-type-checker-plan.md` — get the design right,
then implement milestone by milestone with tests at each step.

---

## 0. Is the list complete? Two more found, one re-examined and cleared.

**New: instance method bodies are never type-checked at all.**
`constrain_module` literally filters `CDecl::Instance` out:

```rust
CDecl::TypeAlias(..) | CDecl::TypeDecl(..) | CDecl::Instance(_) => None,
```

So `instance Eq Shape where (==) a b = 5` — a method returning `Int` where
`Eq`'s `(==)` must return `Bool` — is accepted with zero error. Nothing
about an instance's methods is constrained, ever. This is a real gap of
the same *kind* as the ones already found: silently accepting something
that shouldn't type-check.

**New (but really the same root cause as the Tuple/Record gap): `Unit`'s
seeded instance is dead code.** `prelude.rs` seeds
`table.insert_builtin("Eq", Ref::Builtin("Unit"))`, but `()` values are
typed as `Structure::Unit` (a dedicated `Structure` variant), never as
`Structure::App(Ref::Builtin("Unit"), [])`. `check_pending`'s `if let Some
(Structure::App(head, _))` can never match a `Structure::Unit`-typed
obligation, so that seeded entry is unreachable. Folded into fix #4 below,
not a separate fix.

**Re-examined and cleared**: I'd flagged, while building TM5, that
generalizing a mutually-recursive SCC group's members *independently*
(rather than computing one joint `ftv` set across the whole group) might
under-share a genuinely-shared free variable. Traced it through carefully
just now: because `free_vars` walks the *already-fully-resolved*
substitution, and a shared variable is the literal same `TypeVarId` in
both members' final types (linked via unification during solving, not
copied), each member's independent `generalize` call correctly discovers
it as free regardless. What differs from textbook joint generalization is
that each member gets its *own* separate quantification, instantiated
independently at each call site — which is actually the semantically
correct behavior (two independently-recursive functions are allowed to be
used at different types by their own separate callers; there's no
requirement that `isOdd`/`isEven`-style siblings share a caller's
instantiation choice). Not a bug — no fix needed.

**Not a checker bug, but worth naming**: `SchemeKey::Imported` exists but
nothing ever inserts into it — there's no multi-module compilation driver
at *any* layer yet (parsing, canonicalization, or checking), so
cross-module scheme sharing has never been exercised. Out of scope for
this plan; it's a missing *capability*, not a wrong *design*.

That's the full list: two soundness holes already discussed (structural/
parametric instance checking), the newly-found instance-method gap (a
third soundness hole, same family), the `let`-polymorphism restriction,
and the higher-kinded-types gap. Five fixes, below.

---

## 1. Real let-polymorphism for `let`-bound names

**Root cause** (recap): `Ref::Local` covers lambda/case/do params *and*
`let` bindings — `constrain::expr::constrain_name_ref` can't tell them
apart, so every `Ref::Local` resolves via an immediate, non-generalized
`LocalScope` lookup.

**Architectural decision: fix this entirely inside `knot-checker`. Do not
add a new `Ref` variant to `knot-canonical`.** It's tempting to "fix it at
the source" by splitting `Ref::Local` into `Ref::Local`/`Ref::LetLocal` —
but that would leak a type-checking concern (which bindings are eligible
for generalization) into a crate whose whole design principle is *not*
needing to know anything about types. `knot-canonical`'s own doc comment
is explicit that grouping `let` with lambda/case/do is deliberate for name
resolution's purposes. The fact that `knot-checker` needs a finer
distinction doesn't mean `knot-canonical`'s distinction was wrong — it
means `knot-checker` needs its *own* bookkeeping, which it's already
halfway to having (`LocalScope` already exists for exactly this kind of
thing).

**Design**:

```rust
// constrain/mod.rs
enum LocalBinding {
    /// Lambda/case/do param, or a group member still being solved (self/
    /// mutual reference within its own SCC group) -- always this TypeVarId,
    /// every time, forever.
    Monomorphic(TypeVarId),
    /// A `let`-bound name, keyed by its own header TypeVarId (globally
    /// unique, since it's an arena index) rather than by name -- a bare
    /// name isn't unique across different `let` blocks, but this is.
    Generalizable(TypeVarId),
}

pub struct LocalScope {
    frames: Vec<HashMap<String, LocalBinding>>,
}
```

`constrain::expr::constrain_name_ref`'s `Ref::Local` case splits:

```rust
Ref::Local(name) => match scope.lookup(name) {
    LocalBinding::Monomorphic(ty) => ty,
    LocalBinding::Generalizable(key) => {
        let ty = sub.fresh_unbound();
        constraints.push(Constraint::LookupLocal { span, key, expected: ty });
        ty
    }
}
```

`Constraint::LookupLocal { span, key: TypeVarId, expected: TypeVarId }` is
a new sibling to `Constraint::Lookup`, identical in spirit but keyed by
`TypeVarId` instead of `Ref`. `solve.rs` gains a second table, `local_env:
HashMap<TypeVarId, Scheme>`, populated exactly where `constrain::decl`'s
`Constraint::Let { top_level: false, .. }` groups finish solving (today
that branch does nothing but walk `body_con` — it gains the same
generalize-and-install step the `top_level: true` branch already has,
just installing into `local_env` keyed by each member's own `header_ty`
instead of into the global `SchemeEnv` keyed by name). `LookupLocal`
resolution is then just `instantiate` against `local_env`, identical to
how `Lookup` already works against `SchemeEnv`.

**Consequence worth calling out**: this also means a `let`-bound CAF with
a dangling constraint becomes checkable by the *same* ambiguous-CAF logic
top-level bindings already get — currently `let`-expressions are exempt
from that check too (documented in `Constraint::Let`'s own doc comment).
Fixing let-polymorphism and fixing that exemption are the same change,
which is a good sign this is the right cut.

**Steps**: (1) `LocalBinding` enum + `LocalScope` API update. (2)
`Constraint::LookupLocal` variant. (3) `constrain::decl`'s `top_level:
false` branch gains generalize + ambiguous-check + install-into-
`local_env`. (4) `solve.rs` gains `local_env`, resolves `LookupLocal`. (5)
Test: the exact `identity`-shaped example, but as a `let` instead of
top-level — `let id = \x -> x in (id 1, id True)` — now type-checks.

**Independent of every other fix below** — can be done first, in isolation.

---

## 2. Higher-kinded ("closed-set constructor") polymorphism

**Root cause** (recap): `map :: (a -> b) -> f a -> f b` needs `f` to range
over type *constructors* (`List`, `Map`, ...), not ordinary types.
`Structure::App(Ref, Vec<TypeVarId>)` always has a concrete `Ref` head —
no way to represent "unknown constructor" as a variable.

**Architectural decision: not full kind polymorphism. A narrow,
closed-set feature, matching the rest of Knot's closed-interface/
closed-operator philosophy — and reusing the *existing* interface/instance
machinery to enforce the closure, rather than building a second,
parallel enforcement mechanism.**

Concretely: introduce a genuinely new *sort* of type variable — a
"constructor variable" — that can unify with any concrete type-constructor
`Ref`, but is only usable once a `HasInstance`-shaped obligation on *it
specifically* (not on a fully-applied type) is satisfied. This means:

- `interface::table` gains two more built-in interfaces:
  `Collection` (spec §6.3 — `map`/`foldl`/`foldr`/`filter`/`length`) and
  `Context` (spec §6.4 — `pure`/`bind`). Not user-declarable (same closed
  set as the other 8), just two more named interfaces whose *instances*
  range over constructors instead of types.
- `prelude.rs` seeds `Collection` for `List`/`Map`, `Context` for
  `Option`/`Result`/`IO`/`List`, exactly like `Num Int` is seeded today —
  just keyed by a constructor instead of a type.
- A constructor variable that's never resolved to anything by the closed
  set stays a genuine "kind error," reported the same way any other
  unresolved instance is.

**Design — two sorts of `Slot`, one `Substitution`.** Rather than a
parallel arena, `var.rs`'s `Slot` gains two more variants:

```rust
enum Slot {
    Unbound { rank: u32 },
    Link(TypeVarId),
    Bound(Structure),
    Rigid { name: String },
    /// A constructor variable, unbound.
    CtorUnbound,
    /// A constructor variable, resolved to a concrete constructor.
    CtorBound(Ref),
}
```

`Structure::App`'s head becomes:

```rust
enum AppHead {
    Concrete(Ref),
    Var(TypeVarId),   // must point at a Ctor* slot
}

enum Structure {
    App(AppHead, Vec<TypeVarId>),
    ...
}
```

`unify` gains the new cross-product: `AppHead::Var` vs `AppHead::Var`
(union them, same rank-balancing idea as `union_unbound`); `AppHead::Var`
vs `AppHead::Concrete(r)` (bind the variable to `r`, occurs-check doesn't
apply the same way since a bare `Ref` has no sub-structure to recurse
into); `Concrete` vs `Concrete` (existing behavior, unchanged). A
*type*-sorted variable unifying against a *constructor*-sorted one is a
new, distinct error — a kind mismatch — caught the same place `unify_rigid`
catches its own category errors today.

`free_vars`/`generalize`/`instantiate` all need one more case each (a
`CtorUnbound`/`CtorBound` slot counts as "free"/gets copied fresh, same
shape as the existing `Unbound`/`Rigid` handling, just one sort over).

With this in place, `map`'s real signature becomes expressible:

```rust
let f = sub.fresh_ctor_unbound();          // new Substitution method
let a = sub.fresh_unbound();
let b = sub.fresh_unbound();
let fa = sub.fresh_bound(Structure::App(AppHead::Var(f), vec![a]));
let fb = sub.fresh_bound(Structure::App(AppHead::Var(f), vec![b]));
let a_to_b = sub.fresh_bound(Structure::Fn(a, b));
let ty = sub.fresh_bound(Structure::Fn(a_to_b, sub.fresh_bound(Structure::Fn(fa, fb))));
// constraints: [(f, "Collection")]   -- note: constraint is on the *constructor* var
```

`Scheme.constraints: Vec<(TypeVarId, String)>` doesn't need to change
shape at all — a constructor variable's `HasInstance`-style obligation is
recorded exactly the same way an ordinary one is; `check_ambiguous` and
`check_pending` both need one branch each to resolve a `Collection`/
`Context` obligation against a `Ctor*`-sorted `ty` (look up the resolved
`Ref` directly) instead of the `Structure::App(head, _)` pattern they use
for ordinary obligations.

**`Do`-notation** (`constrain::expr`'s remaining `todo!()`) falls out once
`pure`/`bind` have real signatures: desugar `do { x <- e1; e2 }` to
`bind e1 (\x -> e2)` exactly like the spec already says, generating
constraints the normal way once `bind :: Context f => f a -> (a -> f b) ->
f b` is a real, instantiable scheme.

**Steps**: (1) `Slot`/`AppHead` additions in `var.rs`/`ty.rs`. (2) `unify`
cross-product cases + kind-mismatch error. (3) `free_vars`/`generalize`/
`instantiate` updated for the new sort. (4) `Collection`/`Context` added
to `interface::table`, seeded in `prelude.rs`. (5) Real `map`/`foldl`/
`foldr`/`filter`/`length`/`pure`/`bind` schemes. (6) `constrain::expr`'s
`Do` case, desugaring to `bind`/`pure`. (7) Tests: `map (\x -> x + 1)
[1,2,3]` type-checks and infers `List Int`; using `map` at `Map`/`Option`
in the same module doesn't collide (real polymorphism over the
constructor, not a fixed concrete one); a constructor variable that never
resolves to `List`/`Map`/etc. is a clean kind error, not a panic.

**The single largest fix here** — genuinely new type-system machinery, not
a plumbing retrofit. Recommend doing this early (see ordering below), since
everything downstream that touches `Structure` (elaboration, instance
checking) should be built against the *final* shape of that enum, not
rebased onto it twice.

---

## 3. Full `CExpr` → `TExpr` elaboration

**Root cause** (recap): `constrain_expr`/`constrain_pattern` return a bare
`TypeVarId` per node; the `Constraint` list they populate is a flat `Vec`,
not shaped like the original tree, so there's no way to recover which
`TypeVarId` belonged to which node once generation finishes.

**Reassurance first**: the hard, correctness-sensitive part — given an
obligation and a solved type, which instance answers it — is already built
and tested (`elaborate::resolve_dictionary`). This fix is real but
*mechanical*: retrofit two functions' return types and thread the result
through, then run the existing resolution logic as a second pass over the
now-correlated tree. No new algorithm, just correlation.

**Design — two-stage elaboration**, since dictionary resolution genuinely
can't happen until solving is completely finished, but tree correlation
has to happen *during* generation (it's generation's own recursion that
knows which node is which):

**Stage A (during generation)**: `constrain_expr`/`constrain_pattern`
change from `-> TypeVarId` to `-> (TypeVarId, TExpr)` /
`-> (TypeVarId, TPattern)`. `TExpr` mirrors `CExpr`'s shape exactly, with
a `ty: TypeVarId` on every node, and — at constrained sites (`BinOp`,
`Negate`, any `Lookup`/`LookupLocal` whose resolved scheme carries
constraints) — a placeholder list of `(interface: String, ty: TypeVarId)`
pairs marking "this call site will need a dictionary for this obligation,
once solving finishes." (This is exactly `PendingInstance`'s own shape,
reused, not reinvented.)

```rust
pub enum TExpr {
    IntLit(i64, TypeVarId),
    Var(Ref, TypeVarId, Vec<(String, TypeVarId)> /* pending obligations */),
    App(Box<TExpr>, Box<TExpr>, TypeVarId),
    BinOp(BinOp, Box<TExpr>, Box<TExpr>, TypeVarId, Vec<(String, TypeVarId)>),
    ...   // one variant per CExpr variant, same shape, `ty` (and sometimes
          // pending obligations) appended
}
```

`constrain::decl` needs the equivalent threading: `LetMember` gains an
`elaborated_body: TExpr`, and `constrain_module`/`constrain_let_bindings`
return the built `TExpr`/`TModule` alongside the `Constraint` tree they
already return.

**Stage B (after `solve::solve` finishes)**: a new `elaborate::elaborate_
module` walks the `TExpr` tree built in Stage A and, for every pending
obligation recorded on a node, calls the *already-existing*
`resolve_dictionary` to fill in a real `Dictionary`, producing the final
fully-elaborated tree (`ty` fields resolved via `sub.resolve_structure`
too, if a frozen/snapshotted type is wanted rather than a live
`TypeVarId`).

**Steps**: (1) Define `TExpr`/`TPattern` fully in `ast.rs`. (2) Retrofit
`constrain::pattern::constrain_pattern` (smaller surface, do it first to
prove the pattern out). (3) Retrofit `constrain::expr::constrain_expr` —
every arm, mechanical but not small. (4) Retrofit `constrain::decl` to
thread elaborated bodies through `LetMember`/the module-level result. (5)
`elaborate::elaborate_module` (Stage B) walking the finished tree, calling
existing `resolve_dictionary` per pending obligation. (6) Tests: re-run
every existing TM3 test but assert on the *shape* of the returned `TExpr`
too, not just the `TypeVarId`; one true end-to-end test producing a fully
resolved `TExpr` for something like `f x y = x == y` called at a concrete
type, with a real `Dictionary` filled in at the `==`.

**Depends on fix #2 being done first** (or `TExpr`'s `App`/constructor-
related variants will need re-shaping once `AppHead` exists) and
**benefits from fix #4** (so the dictionaries Stage B fills in are already
correctly recursive for container types, rather than needing a second
pass later).

---

## 4. Structural + parametric instance checking (soundness fix)

**Root cause** (recap): `InstanceTable` is keyed by `(interface, Ref)` —
tuples/records have no `Ref` at all (obligations on them are silently
skipped, neither confirmed nor rejected), and a container's seeded
instance (`Eq List`) is unconditional, ignoring its element type entirely
(`List Weird` incorrectly passes `Eq` checking even though `Weird` has no
`Eq` instance).

**Design**: replace the flat existence check with a recursive one.

```rust
// InstanceEntry gains its own constraint list -- which of the target
// type's positional arguments need which interface, e.g. List's Eq entry
// says "position 0 needs Eq" (from `instance Eq a => Eq (List a)`).
struct InstanceEntry {
    kind: InstanceKind,          // BuiltIn | Declared
    requires: Vec<(usize, String)>,  // (arg position, required interface)
}

fn check_instance(sub: &mut Substitution, table: &InstanceTable, interface: &str, ty: TypeVarId) -> bool {
    match sub.resolve_structure(ty) {
        Some(Structure::App(AppHead::Concrete(head), args)) => {
            let Some(entry) = table.get(interface, &head) else { return false };
            entry.requires.iter().all(|(pos, req_interface)| {
                check_instance(sub, table, req_interface, args[*pos])
            })
        }
        // Structural rules -- not looked up in the table at all, hardcoded
        // language-level facts (spec's own "tuples given their components").
        Some(Structure::Tuple(elems)) => elems.iter().all(|e| check_instance(sub, table, interface, *e)),
        Some(Structure::Record(fields, _)) => fields.values().all(|f| check_instance(sub, table, interface, *f)),
        Some(Structure::Unit) => matches!(interface, "Eq" | "Ord" | "Show"), // one value, trivially all three
        Some(Structure::Fn(..)) => false,   // functions are never Eq/Ord/Show
        _ => false,                          // still unresolved, or a constructor-sorted var (handled by fix #2's own path)
    }
}
```

`build_instance_table` needs to populate `requires` for parametric
declared instances (walk the instance's own `constraints: Vec<CConstraint>`
against its `target: CType`'s type-variable positions — e.g. `instance Eq a
=> Eq (List a)`'s target is `List a`, single param named `a` at position
0, matched against the constraint list's `{interface: "Eq", type_var: "a"}`
to produce `requires: [(0, "Eq")]`). Built-in container instances in
`prelude.rs` get the same treatment by hand (`List`'s `Eq`/`Ord`/`Show`
entries all get `requires: [(0, that interface)]`).

`check_pending`/`elaborate::resolve_dictionary` both switch from
`has_instance` to `check_instance`. `Unit`'s now-genuinely-dead
`insert_builtin` calls in `prelude.rs` get removed (fix #4 makes the
hardcoded `Structure::Unit` case in `check_instance` the actual mechanism
instead).

`ast::Dictionary` gains recursion to match, once fix #3 exists to consume
it — a container's dictionary needs to carry its element's own dictionary:

```rust
pub struct Dictionary {
    pub interface: String,
    pub head: Ref,
    pub args: Vec<Dictionary>,   // one per `requires` entry, same order
}
```

**Steps**: (1) `InstanceEntry.requires` + populate it from both declared
and built-in instances. (2) `check_instance` (recursive) replacing
`has_instance` at both call sites. (3) Structural `Tuple`/`Record`/`Unit`
rules. (4) Remove `Unit`'s now-redundant `insert_builtin` calls. (5)
`Dictionary.args` (only meaningfully populated once fix #3 exists to call
it, but the shape can land now). (6) Tests: `List Weird`'s `==` is
correctly rejected; `(Weird, Int)`'s `==` is correctly rejected; `List
Int`, `(Int, Bool)`, `Option String` all still correctly accepted; nested
containers (`List (List Int)`) resolve two levels deep.

**Independent of fix #1**; benefits from being done before or alongside
fix #3 (see above); doesn't strictly need fix #2, but `AppHead::Concrete`
vs `AppHead::Var` is a pattern this code needs to match on either way once
fix #2 lands, so sequencing after #2 avoids touching it twice.

---

## 5. Instance method bodies need to be checked against their interface

**Root cause**: `constrain_module` filters `CDecl::Instance` out entirely
— nothing constrains a method's body against what the interface says its
signature should be.

**Design**: `interface::table` needs to know each interface's method
*shapes*, not just its name and superclasses — expressed relative to the
interface's own type variable (`Self`), since the shape has to be
instantiated against whatever concrete type a specific `instance` targets:

```rust
enum MethodShape {
    SelfTy,
    Fn(Box<MethodShape>, Box<MethodShape>),
    Bool,
    Ordering,
    StringTy,
}

const METHODS: &[(&str, &[(&str, MethodShape)])] = &[
    ("Eq", &[("==", Fn(SelfTy, Fn(SelfTy, Bool)))]),
    ("Ord", &[("compare", Fn(SelfTy, Fn(SelfTy, Ordering)))]),
    ("Show", &[("show", Fn(SelfTy, StringTy))]),
    // Semigroup/Monoid/Num/Fractional/Integral similarly -- all of these
    // are already written down as ordinary Knot signatures in spec §6/§7;
    // this table just needs to restate them once, symbolically.
];
```

`constrain::decl` stops filtering `CDecl::Instance` out: for each method,
instantiate its `MethodShape` against the instance's own `target: CType`
(substituting `SelfTy` -> the target's `Structure`), producing a real
expected `TypeVarId` — then constrain the method's `CFnDef` (params +
body) against it *exactly* like a signed top-level binding already works
(this reuses `constrain_one_group`'s signed-binding machinery almost
as-is; an instance method is really just a binding whose "signature"
was synthesized instead of user-written, including getting the instance's
own declared `constraints` — e.g. `instance Eq a => Eq (List a)`'s own `Eq
a` context — turned into `given` facts the method body gets to assume,
the same as an ordinary rigid-variable signature does).

**Steps**: (1) `MethodShape` + the `METHODS` table (one entry per
interface, straight from spec §6/§7 — mechanical, no new design). (2)
Shape-instantiation function (`SelfTy` -> target's `Structure`, walking
the same way `constrain::decl::instantiate_rigid` already walks a `CType`
— genuinely similar code, may be worth sharing). (3) `constrain_module`
stops skipping `CDecl::Instance`; builds a signed-binding-shaped
`RawBinding` per method, reusing existing group-constraining code. (4)
Tests: `instance Eq Shape where (==) a b = 5` is now a real type error;
`instance Eq Shape where (==) a b = True` (correct) still passes; a
parametric instance's method body correctly gets to *use* its own
declared context (`instance Eq a => Eq (List a) where (==) ... `'s body
using `==` on elements of type `a` is allowed, because `a`'s `Eq` is a
`given` fact here exactly like a top-level signature's constraints are).

**Independent of fixes #1–#4** — can be done any time; shares a little
machinery with fix #2 (both need "instantiate a type template against a
concrete target"), so doing it after #2 might reveal a reusable helper,
but doesn't require #2's actual kind machinery.

---

## Suggested order

1. **Fix #1** (`let`-polymorphism) — fully independent, smallest, good
   warm-up.
2. **Fix #2** (higher-kinded types) — the biggest single change to
   `Structure`/`unify`/`var.rs`; doing it before #3/#4 avoids rebasing
   their `Structure`-pattern-matching code twice.
3. **Fix #4** (structural/parametric instance checking) — needed correctly
   shaped before #3's Stage B tries to build real (possibly recursive)
   dictionaries.
4. **Fix #3** (full elaboration walk) — the mechanical retrofit, now
   sitting on top of a `Structure` that already has its final shape and an
   instance-checking layer that's actually sound.
5. **Fix #5** (instance method checking) — independent of the others,
   can slot in anywhere; listed last only because it's the least
   architecturally entangled with the rest.

## Staying deferred, per direction

- `Sensitivity`'s opaque-stub treatment (spec §9.6's real recursive
  expansion).
- Annotation *values* never being constrained against `annotation::table`'s
  derived expected type.

Both are additive — neither blocks nor is blocked by any fix above.
