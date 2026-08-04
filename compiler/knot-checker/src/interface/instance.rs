//! The instance table: which `(interface, head type)` pairs actually have
//! an instance. Built from a module's own `CInstanceDecl`s here (plan §6's
//! `interface/instance.rs`); seeding the *built-in* instances (`Num Int`,
//! ...) is `insert_builtin`'s job, called from wherever TM8 ends up living.
//!
//! **Coherence and superclass existence** (plan §3) are checked by
//! `build_instance_table` itself, across the *whole* module in two passes
//! (see that function's own doc comment on why one pass isn't enough) — at
//! most one instance per `(interface, head)` pair, and a superclass's own
//! instance must exist *somewhere* in the module, regardless of which one
//! was declared textually first. Built-ins are trusted unconditionally
//! (`insert_builtin` never checks either): they're hand-written by this
//! compiler, not user input to validate.
//!
//! **`check_instance` is the real, recursive answer** (Fix #4,
//! `knot-checker-gaps-plan.md`) to "does `ty` have `interface`" — a
//! parametric instance's own `requires` (e.g. `instance Eq a => Eq (List
//! a)`'s `Eq a`) is checked recursively against each corresponding
//! argument, and `Tuple`/`Unit` (and an *open* `Record`, and any `Record`
//! with no matching custom instance) get hardcoded structural rules (spec:
//! these derive `Eq`/`Ord`/`Show` from their own parts) rather than a table
//! lookup at all — no `Ref` to key them by in the first place. `has_
//! instance` stays a flat existence check underneath (used for coherence:
//! does *this exact* head have *some* instance, full stop, ignoring what
//! its own parameters might need) — `insert_declared_unchecked`'s caller
//! (`build_instance_table`'s own superclass/duplicate passes) only ever
//! needs that shallow question, not the recursive one.
//!
//! **A user can't declare an instance *for* a `Var`/`Fn`/`Unit` shape, or
//! an *open* `Record`, directly** (`head_ref` returns `None` for those
//! `CType`s — no `Ref` to key an instance-table entry by at all) — no
//! interface at all can be given to a bare `Fn`/`Var` target or an open
//! row. This used to be silent (the declaration just vanished with no
//! diagnostic, surfacing only as a confusing `NoInstance` wherever a
//! caller tried to use it) — `build_instance_table` now reports a real
//! `InstanceTargetNotNominal` at the declaration site instead. A *closed*
//! `Record` or a `Tuple` target are the two exceptions: `record_key`/
//! `tuple_key` give `InstanceTable`'s own separate `record_entries`/
//! `tuple_entries` a `RecordKey`/`TupleKey` to key by instead of a `Ref` —
//! since `knot-canonical::resolve::alias::expand_aliases` erases every
//! `type alias` reference before this ever runs, a record's field names
//! (paired with each field's own *type*, not just its name — see
//! `CanonicalType`) or a tuple's own element types are the only handle
//! left to declare (and later look up) a custom instance by, letting
//! either override the structural `Eq`/`Ord`/`Show` fallback and, more
//! importantly, giving both access to interfaces (`Num`, `Semigroup`,
//! anything user-defined) that have no structural fallback at all. A
//! `VarApp` whose head is still unresolved (a genuine kind error, spec
//! §10.6) is never confirmed by anything — see `check_instance`'s own
//! doc comment.

use std::collections::{HashMap, HashSet};

use knot_canonical::ast::{CConstraint, CDecl, CType, Ref};
use knot_syntax::span::Spanned;

use crate::error::{TypeError, TypeErrorKind};
use crate::interface::table::{is_known_interface, superclasses};
use crate::solve::PendingInstance;
use crate::ty::Structure;
use crate::var::{Substitution, TypeVarId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceKind {
    BuiltIn,
    Declared,
}

/// One `(interface, head)` table entry: not just "does it exist" but
/// "which of the head's own positional type arguments need which interface
/// too" — e.g. `List`'s `Eq` entry says "position 0 needs `Eq`" (from
/// `instance Eq a => Eq (List a)`, or the equivalent hand-written fact for
/// a built-in in `prelude.rs`). `requires` is empty for a non-parametric
/// instance (`instance Eq Shape`, or any of the numeric primitives) — never
/// having to consult anything further is just the zero-argument case of
/// the same rule. `field_requires` is the record-target counterpart —
/// keyed by field *name* rather than positional index, since a record has
/// no positions, only names — populated only for a `record_entries` entry;
/// always empty for an ordinary `entries` one (a single entry only ever
/// has one shape of obligation, never both, so this is simpler than an
/// enum for what's mutually exclusive anyway).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceEntry {
    pub kind: InstanceKind,
    pub requires: Vec<(usize, String)>,
    pub field_requires: Vec<(String, String)>,
}

/// A `CType`/`Structure`'s own shape with every type *variable* replaced by
/// a canonical, first-occurrence-order index — what makes `RecordKey`/
/// `TupleKey` meaningfully *type*-aware (not just name/arity-aware) without
/// accidentally distinguishing `{ value : a }` from `{ value : b }` (same
/// shape, arbitrary variable spelling) while still correctly distinguishing
/// `{ value : Int }` from `{ value : String }` (genuinely different, both
/// fully concrete) and `{ x : a, y : a }` from `{ x : a, y : b }` (whether
/// two positions share the *same* variable is real structure, not just
/// naming). Two separate walks build one of these — `canonicalize_ctype`
/// (a fresh, not-yet-inferred instance *declaration*'s own `CType`, keyed
/// by each variable's own name) and `canonicalize_structure` (a real call
/// site's own resolved `Structure`, post type-inference, keyed by
/// `TypeVarId` instead since a use site's variables have no name at all)
/// — but both produce this same target shape, which is all `RecordKey`/
/// `TupleKey` equality/hashing ever needs to agree on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CanonicalType {
    Var(usize),
    Named(Ref, Vec<CanonicalType>),
    Fn(Box<CanonicalType>, Box<CanonicalType>),
    Tuple(Vec<CanonicalType>),
    /// Sorted by field name — only ever built from a *closed* record (no
    /// row-extension case at all, unlike `CType::Record`/`Structure::
    /// Record` themselves), since only a closed record can be a
    /// well-defined instance target in the first place (`record_key`'s own
    /// doc comment).
    Record(Vec<(String, CanonicalType)>),
    Unit,
}

/// A closed record's own canonical shape (sorted field names, each paired
/// with its own alpha-equivalence-normalized type — see `CanonicalType`),
/// standing in for `Ref` as the key for a record-shaped instance target
/// (`instance Num Vector2` where `Vector2` is a `type alias` for `{ x :
/// Float, y : Float }`) -- see `record_key`'s own doc comment for why
/// records need a wholly separate table from `entries` rather than a new
/// `Ref` variant.
type RecordKey = Vec<(String, CanonicalType)>;

/// The tuple-target counterpart of `RecordKey` — positional (a tuple has
/// no field names to sort by), each element's own canonical shape in
/// order.
type TupleKey = Vec<CanonicalType>;

#[derive(Debug, Default)]
pub struct InstanceTable {
    entries: HashMap<(String, Ref), InstanceEntry>,
    record_entries: HashMap<(String, RecordKey), InstanceEntry>,
    tuple_entries: HashMap<(String, TupleKey), InstanceEntry>,
}

impl InstanceTable {
    pub fn new() -> Self {
        InstanceTable::default()
    }

    pub fn has_instance(&self, interface: &str, head: &Ref) -> bool {
        self.entries
            .contains_key(&(interface.to_string(), head.clone()))
    }

    fn has_record_instance(&self, interface: &str, key: &RecordKey) -> bool {
        self.record_entries
            .contains_key(&(interface.to_string(), key.clone()))
    }

    fn record_entry(&self, interface: &str, key: &RecordKey) -> Option<&InstanceEntry> {
        self.record_entries
            .get(&(interface.to_string(), key.clone()))
    }

    fn has_tuple_instance(&self, interface: &str, key: &TupleKey) -> bool {
        self.tuple_entries
            .contains_key(&(interface.to_string(), key.clone()))
    }

    fn tuple_entry(&self, interface: &str, key: &TupleKey) -> Option<&InstanceEntry> {
        self.tuple_entries
            .get(&(interface.to_string(), key.clone()))
    }

    fn entry(&self, interface: &str, head: &Ref) -> Option<&InstanceEntry> {
        self.entries.get(&(interface.to_string(), head.clone()))
    }

    /// TM8's seam — built-in instances are trusted, not checked. `requires`
    /// is this instance's own positional-argument obligations, same shape
    /// and meaning as a declared instance's (see `InstanceEntry`) — e.g.
    /// `List`'s `Eq` entry passes `vec![(0, "Eq".to_string())]`.
    pub fn insert_builtin(&mut self, interface: &str, head: Ref, requires: Vec<(usize, String)>) {
        self.entries.insert(
            (interface.to_string(), head),
            InstanceEntry {
                kind: InstanceKind::BuiltIn,
                requires,
                field_requires: Vec::new(),
            },
        );
    }

    /// Merges `other`'s own entries into `self`, keeping `self`'s own entry
    /// on a `(interface, head)` collision — `check::check_module`'s own
    /// glue for combining `prelude::seed`'s builtin table with a module's
    /// own `build_instance_table` result. Called with the *declared* table
    /// as `self` so a builtin entry never silently shadows a real user
    /// declaration. By the time this runs, a user "redeclaring" an
    /// interface a builtin type already has (`instance Eq Int where ...`)
    /// has already been reported as a `DuplicateInstance` by
    /// `build_instance_table`'s own pass 1 (given the same `builtins`
    /// table as this call's own `other`) and left out of `self` entirely
    /// — so this merge itself never actually needs to resolve that
    /// collision; it only ever sees genuinely-non-overlapping heads in
    /// practice.
    pub fn merge_from(&mut self, other: InstanceTable) {
        for (key, entry) in other.entries {
            self.entries.entry(key).or_insert(entry);
        }
        for (key, entry) in other.record_entries {
            self.record_entries.entry(key).or_insert(entry);
        }
        for (key, entry) in other.tuple_entries {
            self.tuple_entries.entry(key).or_insert(entry);
        }
    }

    /// Registers a declared instance's own existence with no checking at
    /// all — `build_instance_table`'s own two passes do the duplicate and
    /// superclass checks themselves, across *every* declared instance
    /// first, so a superclass declared later in the same module (`instance
    /// Ord Shape` followed by `instance Eq Shape`) is visible by the time
    /// either check runs. See `build_instance_table`'s own doc comment.
    fn insert_declared_unchecked(
        &mut self,
        interface: &str,
        head: Ref,
        requires: Vec<(usize, String)>,
    ) {
        self.entries.insert(
            (interface.to_string(), head),
            InstanceEntry {
                kind: InstanceKind::Declared,
                requires,
                field_requires: Vec::new(),
            },
        );
    }

    /// The record-keyed counterpart of `insert_declared_unchecked` — same
    /// no-checking-here contract, same two-pass caller. `field_requires`
    /// is this record target's own declared context, if any (`instance Eq
    /// a => Eq { value : a }`'s `Eq a` becomes `[("value", "Eq")]`) — see
    /// `instance_field_requires`.
    fn insert_declared_record_unchecked(
        &mut self,
        interface: &str,
        key: RecordKey,
        field_requires: Vec<(String, String)>,
    ) {
        self.record_entries.insert(
            (interface.to_string(), key),
            InstanceEntry {
                kind: InstanceKind::Declared,
                requires: Vec::new(),
                field_requires,
            },
        );
    }

    /// The tuple-keyed counterpart of `insert_declared_unchecked` — same
    /// no-checking-here contract, same two-pass caller. `requires` here is
    /// positional, exactly like an ordinary `Ref`-keyed entry's own (a
    /// tuple's elements already have positions, unlike a record's named
    /// fields) — see `tuple_requires`.
    fn insert_declared_tuple_unchecked(
        &mut self,
        interface: &str,
        key: TupleKey,
        requires: Vec<(usize, String)>,
    ) {
        self.tuple_entries.insert(
            (interface.to_string(), key),
            InstanceEntry {
                kind: InstanceKind::Declared,
                requires,
                field_requires: Vec::new(),
            },
        );
    }
}

/// The head type constructor an instance's `target` names —
/// `instance Eq Shape` -> `Shape`'s `Ref`; `instance Eq (List a)` ->
/// `List`'s. `None` for a target this can't key by `Ref` at all
/// (`Var`/`Fn`/`Unit`, or a `Record`/`Tuple` -- see `record_key`/
/// `tuple_key` for those two's own separate paths) — see module docs.
/// `VarApp` is here for exhaustiveness only, never actually reachable: an
/// instance's own target is parsed via `type_atom`, which can never
/// produce one (only `type_app`'s own trailing-argument loop can) — there's
/// no `instance Eq f` syntax for a constructor variable to begin with.
fn head_ref(target: &CType) -> Option<&Ref> {
    match target {
        CType::Named(r, _) => Some(r),
        CType::Var(_)
        | CType::VarApp(..)
        | CType::Fn(..)
        | CType::Tuple(_)
        | CType::Record(..)
        | CType::Unit => None,
    }
}

/// `canonicalize_ctype`'s own state: which type-variable *name* has
/// already been assigned which canonical index, in first-occurrence order
/// across the whole walk (so a shared name across two positions — `{ x :
/// a, y : a }` — correctly gets the *same* index both times, and
/// `instance_requires`/`instance_field_requires`/`tuple_requires`'s own
/// name-matching stays meaningful alongside it).
fn canonicalize_ctype(ty: &CType, seen: &mut HashMap<String, usize>) -> CanonicalType {
    match ty {
        CType::Var(name) => {
            let next = seen.len();
            CanonicalType::Var(*seen.entry(name.clone()).or_insert(next))
        }
        // A constructor-variable application (spec §10.6) inside a record/
        // tuple field's own type -- never actually reachable as a real
        // instance-target field in practice (same reasoning as
        // `canonicalize_structure`'s own `Ctor`/`VarApp` case below): opaque
        // and "generic, not concrete" is all this needs to answer, so it's
        // keyed the same way a bare `Var` is, by name alone, dropping its
        // own arguments from consideration.
        CType::VarApp(name, _args) => {
            let next = seen.len();
            CanonicalType::Var(*seen.entry(name.clone()).or_insert(next))
        }
        CType::Named(r, args) => CanonicalType::Named(
            r.clone(),
            args.iter().map(|a| canonicalize_ctype(a, seen)).collect(),
        ),
        CType::Fn(a, b) => CanonicalType::Fn(
            Box::new(canonicalize_ctype(a, seen)),
            Box::new(canonicalize_ctype(b, seen)),
        ),
        CType::Tuple(elems) => {
            CanonicalType::Tuple(elems.iter().map(|e| canonicalize_ctype(e, seen)).collect())
        }
        // Only ever reached for a *nested* record (e.g. one of a `Tuple`'s
        // own elements) -- the top-level-target case has its own direct
        // handling in `record_key`, which needs the closedness check
        // `record_key`'s own doc comment already explains applies only at
        // the target's own top level, never nested inside it.
        CType::Record(fields, _, _) => {
            let mut sorted: Vec<&(String, CType)> = fields.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            CanonicalType::Record(
                sorted
                    .into_iter()
                    .map(|(name, t)| (name.clone(), canonicalize_ctype(t, seen)))
                    .collect(),
            )
        }
        CType::Unit => CanonicalType::Unit,
    }
}

/// `canonicalize_structure`'s own state — keyed by `TypeVarId` rather than
/// a name (a real call site's own variables have none), same
/// first-occurrence-order rule as `canonicalize_ctype`.
fn canonicalize_structure(
    sub: &mut Substitution,
    ty: TypeVarId,
    seen: &mut HashMap<TypeVarId, usize>,
) -> CanonicalType {
    let root = sub.find(ty);
    match sub.resolve_structure(root) {
        Some(Structure::App(r, args)) => CanonicalType::Named(
            r,
            args.iter()
                .map(|a| canonicalize_structure(sub, *a, seen))
                .collect(),
        ),
        Some(Structure::Fn(a, b)) => CanonicalType::Fn(
            Box::new(canonicalize_structure(sub, a, seen)),
            Box::new(canonicalize_structure(sub, b, seen)),
        ),
        Some(Structure::Tuple(elems)) => CanonicalType::Tuple(
            elems
                .iter()
                .map(|e| canonicalize_structure(sub, *e, seen))
                .collect(),
        ),
        Some(Structure::Record(fields, _)) => CanonicalType::Record(
            fields
                .iter()
                .map(|(name, t)| (name.clone(), canonicalize_structure(sub, *t, seen)))
                .collect(),
        ),
        Some(Structure::Unit) => CanonicalType::Unit,
        // A still-unresolved variable (`Unbound`/`Rigid`), or a
        // constructor-sorted position (`Ctor`/`VarApp` — never actually
        // reachable as a real tuple/record element in practice, since
        // those only ever arise from `Collection`/`Context`'s own
        // machinery, not an ordinary structural position) — all "generic,
        // not a concrete named type" for this purpose, exactly like a bare
        // `CType::Var`.
        None | Some(Structure::Ctor(..)) | Some(Structure::VarApp(..)) => {
            let next = seen.len();
            CanonicalType::Var(*seen.entry(root).or_insert(next))
        }
    }
}

/// The `RecordKey` a *closed* record target keys `InstanceTable`'s own
/// `record_entries` by — `None` for an open/row-polymorphic one (`{ a | x :
/// Int }`, spelled with a leading `a |`), which can never be a well-defined
/// instance target: unlike `Eq`/`Ord`/`Show`'s own structural derivation
/// (which only ever needs to recurse into whatever fields happen to be
/// *visible* at a given call site, `ext` or not), a genuinely custom
/// instance is declared once for one exact, fixed shape, so there must be
/// no further fields a use site could still gain through unification. By
/// the time this ever runs, `knot-canonical::resolve::alias::expand_
/// aliases` has already substituted away every `type alias` reference
/// (`Vector2` in `instance Num Vector2` is already this literal `CType::
/// Record`, not a nominal name any more) — a `RecordKey` is genuinely the
/// *only* handle left for "which declaration is this."
fn record_key(target: &CType) -> Option<RecordKey> {
    let CType::Record(fields, _, None) = target else {
        return None;
    };
    let mut seen = HashMap::new();
    let mut sorted: Vec<&(String, CType)> = fields.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    Some(
        sorted
            .into_iter()
            .map(|(name, t)| (name.clone(), canonicalize_ctype(t, &mut seen)))
            .collect(),
    )
}

/// The `TupleKey` counterpart of `record_key` — every `CType::Tuple` is
/// already "closed" by construction (no row-polymorphism analog exists for
/// tuples at all), so there's no openness check to make here.
fn tuple_key(target: &CType) -> Option<TupleKey> {
    let CType::Tuple(elems) = target else {
        return None;
    };
    let mut seen = HashMap::new();
    Some(
        elems
            .iter()
            .map(|e| canonicalize_ctype(e, &mut seen))
            .collect(),
    )
}

/// Maps each of `target`'s own type-variable-named positional arguments to
/// whichever interface `constraints` requires of that name — e.g.
/// `instance Eq a => Eq (List a)`'s target `List a` has `a` at position 0,
/// matched against `{interface: "Eq", type_var: "a"}` to produce
/// `[(0, "Eq")]`. A position whose argument isn't a bare type variable (a
/// concrete nested type) simply needs nothing extra, so it's just absent
/// from the result.
fn instance_requires(target: &CType, constraints: &[CConstraint]) -> Vec<(usize, String)> {
    let CType::Named(_, args) = target else {
        return Vec::new();
    };
    let mut requires = Vec::new();
    for (pos, arg) in args.iter().enumerate() {
        if let CType::Var(name) = arg {
            for c in constraints {
                if c.type_var == *name {
                    requires.push((pos, c.interface.clone()));
                }
            }
        }
    }
    requires
}

/// The record-target counterpart of `instance_requires` — maps each of a
/// closed record target's own field type-variable names to whichever
/// interface `constraints` requires of that name, e.g. `instance Eq a =>
/// Eq { value : a }`'s `value : a` matched against `{interface: "Eq",
/// type_var: "a"}` produces `[("value", "Eq")]`. A field whose own type
/// isn't a bare type variable (a concrete nested type) simply needs
/// nothing extra, exactly like `instance_requires`'s own positional case
/// — this is the fix for the gap that function's own doc comment used to
/// leave silent: a record target's declared context was accepted into the
/// table but never actually recorded anywhere, so `check_instance` could
/// never re-check it against a real call site's own field type.
fn instance_field_requires(target: &CType, constraints: &[CConstraint]) -> Vec<(String, String)> {
    let CType::Record(fields, _, None) = target else {
        return Vec::new();
    };
    let mut requires = Vec::new();
    for (field_name, field_ty) in fields {
        if let CType::Var(name) = field_ty {
            for c in constraints {
                if c.type_var == *name {
                    requires.push((field_name.clone(), c.interface.clone()));
                }
            }
        }
    }
    requires
}

/// The tuple-target counterpart of `instance_requires` — positional, and
/// mirrors that function almost exactly since a tuple's own elements are
/// already positional, same as a nominal target's own type arguments (no
/// field-name indirection like `instance_field_requires` needs).
fn tuple_requires(target: &CType, constraints: &[CConstraint]) -> Vec<(usize, String)> {
    let CType::Tuple(elems) = target else {
        return Vec::new();
    };
    let mut requires = Vec::new();
    for (pos, elem) in elems.iter().enumerate() {
        if let CType::Var(name) = elem {
            for c in constraints {
                if c.type_var == *name {
                    requires.push((pos, c.interface.clone()));
                }
            }
        }
    }
    requires
}

/// Which of `InstanceTable`'s three parallel key spaces one declared
/// instance's own target resolved to — see `head_ref`/`record_key`/
/// `tuple_key` — paired with that target's own `requires` in whichever shape
/// matches (positional for `Nominal`/`Tuple`, field-keyed for `Record`), so
/// the two can never end up mismatched the way carrying them as two
/// separate values would risk.
#[derive(Debug, Clone)]
enum DeclaredHead {
    Nominal(Ref, Vec<(usize, String)>),
    Record(RecordKey, Vec<(String, String)>),
    Tuple(TupleKey, Vec<(usize, String)>),
}

/// `(interface, head, span)` for one declared instance this table can key
/// by either `Ref` or `RecordKey` — `build_instance_table`'s own
/// intermediate shape between collecting every such declaration and its two
/// checking passes.
type DeclaredInstance<'a> = (&'a str, DeclaredHead, knot_syntax::span::Span);

/// Builds the table from every `CDecl::Instance` in `decls`. Unknown
/// interfaces are skipped (`knot-canonical` already reported that error);
/// `insert_builtin` for the actual built-in seeding happens separately —
/// `builtins` is that already-seeded table (`prelude::seed`'s own, or
/// `&InstanceTable::new()` for a caller with none in play), consulted only
/// by pass 1's own duplicate check (see below for why pass 2 never needs
/// it too) so re-declaring an instance a builtin type already has
/// (`instance Eq Int where ...`) is caught here rather than merged in
/// silently later (`InstanceTable::merge_from`'s own doc comment used to
/// call this out as untested, unusual territory before this fix — a real,
/// user-visible gap, not just an edge case).
///
/// **Two passes, deliberately not one.** A single forward pass checking
/// each instance's own superclasses as it's inserted would make coherence
/// depend on declaration order *within the same module* — `instance Ord
/// Shape` followed later by `instance Eq Shape` would wrongly report a
/// missing `Eq` superclass, purely because `Eq` hadn't been registered yet
/// at the point `Ord` was processed, even though both are declared in this
/// same module. Elm/Haskell have no such ordering requirement, so neither
/// should this. Pass 1 registers every instance's own existence first
/// (first occurrence per `(interface, head)` wins, checked against both
/// `table`-so-far *and* `builtins`; a later or builtin-shadowing duplicate
/// is reported via `DuplicateInstance` and otherwise ignored — not
/// inserted, not superclass-checked in pass 2 either, matching what a
/// single early-return pass would have done for it). Pass 2 then checks
/// every *accepted* instance's own superclasses against the
/// now-fully-populated `table` alone, so order within the module no
/// longer matters — `builtins` is never consulted here too, since every
/// interface this crate currently seeds already seeds its own superclass
/// alongside it (e.g. `Ord`/`Eq` together for `Int`/`Float`/`String`/
/// `Bool`), so a *newly* declared instance (one `builtins` doesn't already
/// have, or pass 1 would have rejected it as a duplicate) can never have a
/// superclass obligation only `builtins` could satisfy.
pub fn build_instance_table(
    decls: &[Spanned<CDecl>],
    builtins: &InstanceTable,
) -> (InstanceTable, Vec<TypeError>) {
    let mut table = InstanceTable::new();
    let mut errors = Vec::new();

    let mut declared: Vec<DeclaredInstance> = Vec::new();
    for d in decls {
        let CDecl::Instance(inst) = &d.node else {
            continue;
        };
        if !is_known_interface(&inst.interface) {
            continue;
        }
        let head = head_ref(&inst.target)
            .map(|r| {
                DeclaredHead::Nominal(
                    r.clone(),
                    instance_requires(&inst.target, &inst.constraints),
                )
            })
            .or_else(|| {
                record_key(&inst.target).map(|key| {
                    DeclaredHead::Record(
                        key,
                        instance_field_requires(&inst.target, &inst.constraints),
                    )
                })
            })
            .or_else(|| {
                tuple_key(&inst.target).map(|key| {
                    DeclaredHead::Tuple(key, tuple_requires(&inst.target, &inst.constraints))
                })
            });
        match head {
            Some(head) => declared.push((inst.interface.as_str(), head, d.span)),
            None => errors.push(TypeError {
                span: d.span,
                kind: TypeErrorKind::InstanceTargetNotNominal {
                    interface: inst.interface.clone(),
                },
            }),
        }
    }

    let mut accepted = vec![false; declared.len()];
    for (i, (interface, head, span)) in declared.iter().enumerate() {
        let already_exists = match head {
            DeclaredHead::Nominal(r, _) => {
                table.has_instance(interface, r) || builtins.has_instance(interface, r)
            }
            DeclaredHead::Record(key, _) => {
                table.has_record_instance(interface, key)
                    || builtins.has_record_instance(interface, key)
            }
            DeclaredHead::Tuple(key, _) => {
                table.has_tuple_instance(interface, key)
                    || builtins.has_tuple_instance(interface, key)
            }
        };
        if already_exists {
            errors.push(TypeError {
                span: *span,
                kind: TypeErrorKind::DuplicateInstance {
                    interface: interface.to_string(),
                },
            });
        } else {
            match head {
                DeclaredHead::Nominal(r, requires) => {
                    table.insert_declared_unchecked(interface, r.clone(), requires.clone())
                }
                DeclaredHead::Record(key, field_requires) => table
                    .insert_declared_record_unchecked(
                        interface,
                        key.clone(),
                        field_requires.clone(),
                    ),
                DeclaredHead::Tuple(key, requires) => {
                    table.insert_declared_tuple_unchecked(interface, key.clone(), requires.clone())
                }
            }
            accepted[i] = true;
        }
    }

    for (i, (interface, head, span)) in declared.iter().enumerate() {
        if !accepted[i] {
            continue;
        }
        for superclass in superclasses(interface) {
            let has_superclass = match head {
                DeclaredHead::Nominal(r, _) => table.has_instance(superclass, r),
                DeclaredHead::Record(key, _) => table.has_record_instance(superclass, key),
                DeclaredHead::Tuple(key, _) => table.has_tuple_instance(superclass, key),
            };
            if !has_superclass {
                errors.push(TypeError {
                    span: *span,
                    kind: TypeErrorKind::MissingSuperclassInstance {
                        interface: interface.to_string(),
                        superclass: superclass.to_string(),
                    },
                });
            }
        }
    }

    (table, errors)
}

/// The real, recursive answer to "does `ty` have `interface`" — see module
/// docs. `Fn` is never `Eq`/`Ord`/`Show`/anything else (no meaningful
/// equality, order, or display for a function value); a still-unresolved
/// type (or a `VarApp` whose head hasn't been pinned to a concrete
/// constructor yet) answers `false` here too, but see `check_pending`'s own
/// doc comment on why *reporting* an error still needs to tell that case
/// apart from a genuine "no."
///
/// **A bare rigid variable defers to `given` first** (Fix #13): a *concrete*
/// obligation like `Semigroup (Max a)` recurses, via `Max`'s own `requires`,
/// into a check of `Ord` against `a` itself -- and `a`, being a signature's
/// own rigid variable, has no `Structure` for `sub.resolve_structure` to
/// ever return (by design, see `var.rs`'s own doc comment), so without this
/// check every such recursive step answered `false` unconditionally, no
/// matter how thoroughly the enclosing signature/instance context already
/// established `Ord a`. `given`'s own values are already closed over
/// superclasses at insertion time (`insert_given_with_superclasses`), so a
/// plain `contains` here is enough -- no separate superclass walk needed.
pub fn check_instance(
    sub: &mut Substitution,
    table: &InstanceTable,
    given: &HashMap<TypeVarId, HashSet<String>>,
    interface: &str,
    ty: TypeVarId,
) -> bool {
    if sub.is_rigid(ty) {
        let root = sub.find(ty);
        return given
            .get(&root)
            .is_some_and(|interfaces| interfaces.contains(interface));
    }
    match sub.resolve_structure(ty) {
        Some(Structure::App(head, args)) => match table.entry(interface, &head) {
            Some(entry) => entry.requires.iter().all(|(pos, req_interface)| {
                check_instance(sub, table, given, req_interface, args[*pos])
            }),
            None => false,
        },
        // A Collection/Context instance is registered by name alone
        // (`prelude::seed_instances`), regardless of how many of the
        // constructor's own arguments are already applied -- the leading
        // ones (`ty::Structure::Ctor`'s own doc comment) don't change
        // *which* instance answers, only what `map`/`bind`/... themselves
        // thread through separately.
        Some(Structure::Ctor(head, _leading_args)) => table.has_instance(interface, &head),
        // A tuple first checks for a matching custom instance (Task #39:
        // `TupleKey`-keyed, mirroring the `Record` arm below exactly) --
        // letting a real declaration (`instance Num (Int, Int)`, say) give
        // a tuple target access to an interface with no structural fallback
        // at all, not just override `Eq`/`Ord`/`Show`'s own derivation. No
        // custom instance falls back to the old structural rule, gated to
        // exactly those three (same as `Unit` below), so a dangling `Num
        // (Int, Int)` obligation (nonsensical -- Knot has no tuple
        // arithmetic) correctly still fails instead of wrongly inheriting
        // `Int`'s own `Num` instance through the recursion.
        Some(Structure::Tuple(elems)) => {
            let mut seen = HashMap::new();
            let key: TupleKey = elems
                .iter()
                .map(|e| canonicalize_structure(sub, *e, &mut seen))
                .collect();
            match table.tuple_entry(interface, &key) {
                Some(entry) => entry.requires.iter().all(|(pos, req_interface)| {
                    check_instance(sub, table, given, req_interface, elems[*pos])
                }),
                None => {
                    matches!(interface, "Eq" | "Ord" | "Show")
                        && elems
                            .iter()
                            .all(|e| check_instance(sub, table, given, interface, *e))
                }
            }
        }
        // A *closed* record first checks for a matching custom instance,
        // keyed by `canonicalize_structure`'s own type-aware `RecordKey`
        // (Task #39 -- previously keyed by field *name* alone, so `{ value
        // : Int }` and `{ value : String }` wrongly collided into the same
        // table slot; see `CanonicalType`'s own doc comment) -- letting a
        // real declaration override the structural fallback below, exactly
        // as a user would expect. Its own `field_requires` (the instance's
        // declared context, e.g. `Eq a` on `instance Eq a => Eq { value : a
        // }`) gets checked recursively against each named field's own real
        // argument type here, exactly like `Structure::App`'s own
        // positional `requires` above. An open/row-polymorphic record
        // (`ext.is_some()`) skips the whole lookup: a use site that hasn't
        // pinned down every field yet can't possibly match one exact,
        // fixed instance target soundly.
        Some(Structure::Record(fields, ext)) => {
            let matched_custom_instance = if ext.is_some() {
                None
            } else {
                let mut seen = HashMap::new();
                let key: RecordKey = fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), canonicalize_structure(sub, *ty, &mut seen)))
                    .collect();
                table.record_entry(interface, &key).map(|entry| {
                    entry
                        .field_requires
                        .iter()
                        .all(|(field_name, req_interface)| {
                            fields.get(field_name).is_some_and(|field_ty| {
                                check_instance(sub, table, given, req_interface, *field_ty)
                            })
                        })
                })
            };
            match matched_custom_instance {
                Some(satisfied) => satisfied,
                None => {
                    matches!(interface, "Eq" | "Ord" | "Show")
                        && fields
                            .values()
                            .all(|f| check_instance(sub, table, given, interface, *f))
                }
            }
        }
        // One value, trivially all three.
        Some(Structure::Unit) => matches!(interface, "Eq" | "Ord" | "Show"),
        _ => false,
    }
}

/// Checks every still-unresolved (concrete-typed) obligation `solve::solve`
/// returned against `table` via `check_instance`, appending a
/// `TypeErrorKind::NoInstance` for each one that fails. A genuinely
/// still-*unresolved* type (`sub.resolve_structure` returns `None` — an
/// unbound flexible variable, or a constructor variable never pinned to a
/// concrete head) is left alone instead: that's not this obligation's
/// fault to report, and for the constructor-variable case specifically
/// it's a kind error this crate doesn't report yet (see `ty::Structure::
/// VarApp`'s own doc comment), not a missing-instance one. `given` is
/// `solve::solve_with_obligations`'s own final map, needed so `check_instance`
/// can resolve a rigid variable buried inside an otherwise-concrete `ty`
/// (Fix #13) -- pass `&HashMap::new()` for a caller with no rigid variables
/// in play at all.
pub fn check_pending(
    sub: &mut Substitution,
    table: &InstanceTable,
    given: &HashMap<TypeVarId, HashSet<String>>,
    pending: Vec<PendingInstance>,
    errors: &mut Vec<TypeError>,
) {
    for p in pending {
        let is_resolved = sub.resolve_structure(p.ty).is_some();
        if is_resolved && !check_instance(sub, table, given, &p.interface, p.ty) {
            errors.push(TypeError {
                span: p.span,
                kind: TypeErrorKind::NoInstance {
                    interface: p.interface,
                },
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_instances_are_trusted_unconditionally() {
        let mut table = InstanceTable::new();
        table.insert_builtin("Num", Ref::Builtin("Int".to_string()), vec![]);
        assert!(table.has_instance("Num", &Ref::Builtin("Int".to_string())));
        assert!(!table.has_instance("Num", &Ref::Builtin("String".to_string())));
    }

    fn decls(src: &str) -> Vec<Spanned<CDecl>> {
        let mut state = knot_syntax::ParseState::new(src);
        let raw = state.parse_decls().unwrap();
        assert!(state.is_eof(), "leftover input: {src}");
        knot_canonical::canonicalize_decls(&raw).unwrap_or_else(|errs| panic!("{errs:?}"))
    }

    #[test]
    fn a_declared_instance_with_its_superclass_already_present_is_accepted() {
        let cs = decls(
            "type Shape = Circle Float\n\
             instance Eq Shape where\n  (==) a b = True\n\
             instance Ord Shape where\n  compare a b = EQ\n",
        );
        let (table, errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(errors.is_empty(), "{errors:?}");
        assert!(table.has_instance("Eq", &Ref::TopLevel("Shape".to_string())));
        assert!(table.has_instance("Ord", &Ref::TopLevel("Shape".to_string())));
    }

    #[test]
    fn declaring_ord_without_eq_first_is_a_missing_superclass_error() {
        let cs = decls("type Shape = Circle Float\ninstance Ord Shape where\n  compare a b = EQ\n");
        let (_table, errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            TypeErrorKind::MissingSuperclassInstance { interface, superclass }
                if interface == "Ord" && superclass == "Eq"
        )));
    }

    #[test]
    fn a_superclass_declared_after_its_own_subclass_in_the_same_module_is_still_accepted() {
        // Same two instances as
        // `a_declared_instance_with_its_superclass_already_present_is_accepted`,
        // just in the opposite declaration order -- coherence must not
        // depend on where in the module each instance happens to be
        // written.
        let cs = decls(
            "type Shape = Circle Float\n\
             instance Ord Shape where\n  compare a b = EQ\n\
             instance Eq Shape where\n  (==) a b = True\n",
        );
        let (table, errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(errors.is_empty(), "{errors:?}");
        assert!(table.has_instance("Eq", &Ref::TopLevel("Shape".to_string())));
        assert!(table.has_instance("Ord", &Ref::TopLevel("Shape".to_string())));
    }

    #[test]
    fn a_duplicate_instance_with_a_genuinely_missing_superclass_reports_each_error_once() {
        // Two `Ord Shape`s (a duplicate) and no `Eq` at all -- the second,
        // rejected `Ord` shouldn't also get its own independent
        // MissingSuperclassInstance check (pass 2 only checks accepted
        // instances), so this should be exactly one DuplicateInstance and
        // exactly one MissingSuperclassInstance, not two of the latter.
        let cs = decls(
            "type Shape = Circle Float\n\
             instance Ord Shape where\n  compare a b = EQ\n\
             instance Ord Shape where\n  compare a b = EQ\n",
        );
        let (_table, errors) = build_instance_table(&cs, &InstanceTable::new());
        let duplicates = errors
            .iter()
            .filter(|e| matches!(&e.kind, TypeErrorKind::DuplicateInstance { .. }))
            .count();
        let missing_superclass = errors
            .iter()
            .filter(|e| matches!(&e.kind, TypeErrorKind::MissingSuperclassInstance { .. }))
            .count();
        assert_eq!(duplicates, 1, "{errors:?}");
        assert_eq!(missing_superclass, 1, "{errors:?}");
    }

    #[test]
    fn declaring_the_same_instance_twice_is_a_duplicate_error() {
        let cs = decls(
            "type Shape = Circle Float\n\
             instance Eq Shape where\n  (==) a b = True\n\
             instance Eq Shape where\n  (==) a b = False\n",
        );
        let (_table, errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(errors
            .iter()
            .any(|e| matches!(&e.kind, TypeErrorKind::DuplicateInstance { interface } if interface == "Eq")));
    }

    #[test]
    fn redeclaring_an_instance_a_builtin_type_already_has_is_a_duplicate_error() {
        // Task #40: build_instance_table's own coherence pass used to only
        // ever see a module's own declared instances, not the builtin
        // table it gets merged with only afterward (InstanceTable::
        // merge_from) -- so `instance Eq Int where ...` type-checked as if
        // Int had no Eq instance yet at all. Now `builtins` is consulted
        // too.
        let mut builtins = InstanceTable::new();
        builtins.insert_builtin("Eq", Ref::Builtin("Int".to_string()), vec![]);
        let cs = decls("instance Eq Int where\n  (==) a b = False\n");
        let (table, errors) = build_instance_table(&cs, &builtins);
        assert!(errors
            .iter()
            .any(|e| matches!(&e.kind, TypeErrorKind::DuplicateInstance { interface } if interface == "Eq")));
        // Rejected, not inserted -- table itself stays empty for this head.
        assert!(!table.has_instance("Eq", &Ref::Builtin("Int".to_string())));
    }

    #[test]
    fn declaring_a_new_instance_for_a_builtin_type_is_still_accepted() {
        // The fix above must not become overly conservative -- a builtin
        // type genuinely getting a *new* interface it doesn't already have
        // (Semigroup has no builtin Int instance) is still fine.
        let mut builtins = InstanceTable::new();
        builtins.insert_builtin("Eq", Ref::Builtin("Int".to_string()), vec![]);
        let cs = decls("instance Semigroup Int where\n  (<>) a b = a\n");
        let (table, errors) = build_instance_table(&cs, &builtins);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(table.has_instance("Semigroup", &Ref::Builtin("Int".to_string())));
    }

    #[test]
    fn a_non_eq_ord_show_instance_targeting_a_closed_record_is_now_accepted() {
        // Semigroup has no structural fallback the way Eq/Ord/Show do, but
        // that's exactly the case a real user most wants a *custom*
        // instance for -- rejecting it as InstanceTargetNotNominal used to
        // make this "unreachable dead code" reasoning (right for Eq/Ord/
        // Show, which already derive automatically) wrongly block a
        // genuinely new capability instead. Now accepted into `record_
        // entries` (keyed by field-name set, `Ref`-keyed `entries` stays
        // untouched either way) rather than `entries` itself.
        let cs = decls("instance Semigroup { x : Float } where\n  (<>) a b = { x = a.x }\n");
        let (table, errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(errors.is_empty(), "{errors:?}");
        assert!(table.entries.is_empty());
        let key = record_key(&CType::Record(
            vec![(
                "x".to_string(),
                CType::Named(Ref::Builtin("Float".to_string()), vec![]),
            )],
            vec![],
            None,
        ))
        .unwrap();
        assert!(table.has_record_instance("Semigroup", &key));
    }

    #[test]
    fn a_record_instances_own_declared_context_is_recorded_as_field_requires() {
        let cs =
            decls("instance Eq a => Eq { value : a } where\n  (==) x y = x.value == y.value\n");
        let (table, errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(errors.is_empty(), "{errors:?}");
        let key = record_key(&CType::Record(
            vec![("value".to_string(), CType::Var("a".to_string()))],
            vec![],
            None,
        ))
        .unwrap();
        let entry = table
            .record_entry("Eq", &key)
            .expect("Eq { value : a } should be registered");
        assert_eq!(
            entry.field_requires,
            vec![("value".to_string(), "Eq".to_string())]
        );
        assert!(entry.requires.is_empty());
    }

    #[test]
    fn check_instance_rejects_a_record_instance_whose_own_context_isnt_satisfied() {
        // The exact unsoundness this fix closes: Eq { value : a } declares
        // `Eq a =>`, but NoEqType has no Eq instance at all -- check_instance
        // must now recurse into `value`'s own real argument type and find
        // it lacking, not just confirm the record *shape* has an instance.
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_declared_record_unchecked(
            "Eq",
            vec![(
                "value".to_string(),
                CanonicalType::Named(Ref::TopLevel("NoEqType".to_string()), vec![]),
            )],
            vec![("value".to_string(), "Eq".to_string())],
        );
        let no_eq_ty = sub.fresh_bound(Structure::App(
            Ref::TopLevel("NoEqType".to_string()),
            vec![],
        ));
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("value".to_string(), no_eq_ty);
        let record_ty = sub.fresh_bound(Structure::Record(fields, None));
        assert!(!check_instance(
            &mut sub,
            &table,
            &HashMap::new(),
            "Eq",
            record_ty
        ));
    }

    #[test]
    fn check_instance_accepts_a_record_instance_whose_own_context_is_satisfied() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Eq", Ref::Builtin("Int".to_string()), vec![]);
        table.insert_declared_record_unchecked(
            "Eq",
            vec![(
                "value".to_string(),
                CanonicalType::Named(Ref::Builtin("Int".to_string()), vec![]),
            )],
            vec![("value".to_string(), "Eq".to_string())],
        );
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("value".to_string(), int_ty);
        let record_ty = sub.fresh_bound(Structure::Record(fields, None));
        assert!(check_instance(
            &mut sub,
            &table,
            &HashMap::new(),
            "Eq",
            record_ty
        ));
    }

    #[test]
    fn two_record_instances_with_the_same_field_name_but_different_types_dont_collide() {
        // The unsoundness/over-rejection Task #39 fixes: RecordKey used to
        // be keyed by field *name* alone, so `{ value : Int }` and `{
        // value : String }` wrongly shared one table slot -- whichever was
        // declared second would either collide as a spurious
        // DuplicateInstance or silently shadow the first.
        let cs = decls(
            "instance Show { value : Int } where\n  show x = \"int\"\n\
             instance Show { value : String } where\n  show x = \"string\"\n",
        );
        let (table, errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(errors.is_empty(), "{errors:?}");
        let int_key = record_key(&CType::Record(
            vec![(
                "value".to_string(),
                CType::Named(Ref::Builtin("Int".to_string()), vec![]),
            )],
            vec![],
            None,
        ))
        .unwrap();
        let string_key = record_key(&CType::Record(
            vec![(
                "value".to_string(),
                CType::Named(Ref::Builtin("String".to_string()), vec![]),
            )],
            vec![],
            None,
        ))
        .unwrap();
        assert!(table.has_record_instance("Show", &int_key));
        assert!(table.has_record_instance("Show", &string_key));
    }

    #[test]
    fn a_custom_instance_can_now_target_a_tuple() {
        // Task #39: a Tuple target used to be unconditionally rejected as
        // InstanceTargetNotNominal, even for an interface with no
        // structural fallback at all (Num has no derived tuple instance
        // the way Eq/Ord/Show do) -- now accepted into `tuple_entries`.
        let cs = decls("instance Num (Int, Int) where\n  (+) a b = a\n");
        let (table, errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(errors.is_empty(), "{errors:?}");
        let key = tuple_key(&CType::Tuple(vec![
            CType::Named(Ref::Builtin("Int".to_string()), vec![]),
            CType::Named(Ref::Builtin("Int".to_string()), vec![]),
        ]))
        .unwrap();
        assert!(table.has_tuple_instance("Num", &key));
    }

    #[test]
    fn an_eq_instance_targeting_a_tuple_now_registers_as_a_custom_instance() {
        // Eq/Ord/Show already derive automatically for Tuple/Record/Unit
        // via check_instance's own hardcoded structural rule -- an
        // explicit `instance Eq (Int, Int)` used to be rejected outright
        // as InstanceTargetNotNominal (unreachable-dead-code reasoning);
        // now that Tuple is a valid target shape like Record, it's simply
        // accepted as a (redundant but harmless) custom instance instead.
        let cs = decls("instance Eq (Int, Int) where\n  (==) a b = True\n");
        let (_table, errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn check_pending_flags_a_concrete_type_with_no_instance() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_bound(Structure::App(Ref::Builtin("String".to_string()), vec![]));
        let pending = vec![PendingInstance {
            span: knot_syntax::span::Span::new(0, 0),
            interface: "Num".to_string(),
            ty,
        }];
        let mut errors = Vec::new();
        check_pending(&mut sub, &table, &HashMap::new(), pending, &mut errors);
        assert!(errors.iter().any(
            |e| matches!(&e.kind, TypeErrorKind::NoInstance { interface } if interface == "Num")
        ));
    }

    #[test]
    fn check_pending_accepts_a_seeded_builtin_instance() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Num", Ref::Builtin("Int".to_string()), vec![]);
        let ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let pending = vec![PendingInstance {
            span: knot_syntax::span::Span::new(0, 0),
            interface: "Num".to_string(),
            ty,
        }];
        let mut errors = Vec::new();
        check_pending(&mut sub, &table, &HashMap::new(), pending, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn check_pending_resolves_a_ctor_headed_obligation_against_the_table() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Collection", Ref::Builtin("List".to_string()), vec![]);
        let ty = sub.fresh_bound(Structure::Ctor(Ref::Builtin("List".to_string()), vec![]));
        let pending = vec![PendingInstance {
            span: knot_syntax::span::Span::new(0, 0),
            interface: "Collection".to_string(),
            ty,
        }];
        let mut errors = Vec::new();
        check_pending(&mut sub, &table, &HashMap::new(), pending, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn check_pending_flags_a_ctor_headed_obligation_with_no_instance() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_bound(Structure::Ctor(Ref::Builtin("IO".to_string()), vec![]));
        let pending = vec![PendingInstance {
            span: knot_syntax::span::Span::new(0, 0),
            interface: "Collection".to_string(),
            ty,
        }];
        let mut errors = Vec::new();
        check_pending(&mut sub, &table, &HashMap::new(), pending, &mut errors);
        assert!(errors.iter().any(
            |e| matches!(&e.kind, TypeErrorKind::NoInstance { interface } if interface == "Collection")
        ));
    }

    #[test]
    fn check_pending_leaves_an_unresolved_ctor_var_unflagged() {
        // A still-unbound constructor variable (never pinned to a concrete
        // head) is a kind error, not a missing-instance one -- left alone
        // here, same as any other still-unresolved obligation.
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_ctor_unbound();
        let pending = vec![PendingInstance {
            span: knot_syntax::span::Span::new(0, 0),
            interface: "Collection".to_string(),
            ty,
        }];
        let mut errors = Vec::new();
        check_pending(&mut sub, &table, &HashMap::new(), pending, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn check_pending_leaves_a_still_unresolved_type_variable_unflagged() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_unbound();
        let pending = vec![PendingInstance {
            span: knot_syntax::span::Span::new(0, 0),
            interface: "Eq".to_string(),
            ty,
        }];
        let mut errors = Vec::new();
        check_pending(&mut sub, &table, &HashMap::new(), pending, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn empty_tuple_trivially_satisfies_eq_ord_show() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_bound(Structure::Tuple(vec![]));
        for interface in ["Eq", "Ord", "Show"] {
            assert!(check_instance(
                &mut sub,
                &table,
                &HashMap::new(),
                interface,
                ty
            ));
        }
    }

    #[test]
    fn a_function_is_never_eq_ord_or_show() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let a = sub.fresh_unbound();
        let fn_ty = sub.fresh_bound(Structure::Fn(a, a));
        assert!(!check_instance(
            &mut sub,
            &table,
            &HashMap::new(),
            "Eq",
            fn_ty
        ));
    }

    #[test]
    fn tuple_is_eq_only_if_every_element_is() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Eq", Ref::Builtin("Int".to_string()), vec![]);
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let weird_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Weird".to_string()), vec![]));

        let ok_pair = sub.fresh_bound(Structure::Tuple(vec![int_ty, int_ty]));
        assert!(check_instance(
            &mut sub,
            &table,
            &HashMap::new(),
            "Eq",
            ok_pair
        ));

        let bad_pair = sub.fresh_bound(Structure::Tuple(vec![int_ty, weird_ty]));
        assert!(!check_instance(
            &mut sub,
            &table,
            &HashMap::new(),
            "Eq",
            bad_pair
        ));
    }

    #[test]
    fn tuple_never_structurally_satisfies_a_non_derivable_interface() {
        // Knot has no tuple arithmetic -- (Int, Int) must not inherit Int's
        // own Num instance just because every element happens to have one.
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Num", Ref::Builtin("Int".to_string()), vec![]);
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let pair = sub.fresh_bound(Structure::Tuple(vec![int_ty, int_ty]));
        assert!(!check_instance(
            &mut sub,
            &table,
            &HashMap::new(),
            "Num",
            pair
        ));
    }

    #[test]
    fn record_is_eq_only_if_every_field_is() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Eq", Ref::Builtin("Int".to_string()), vec![]);
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let weird_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Weird".to_string()), vec![]));

        let mut ok_fields = std::collections::BTreeMap::new();
        ok_fields.insert("x".to_string(), int_ty);
        let ok_record = sub.fresh_bound(Structure::Record(ok_fields, None));
        assert!(check_instance(
            &mut sub,
            &table,
            &HashMap::new(),
            "Eq",
            ok_record
        ));

        let mut bad_fields = std::collections::BTreeMap::new();
        bad_fields.insert("x".to_string(), weird_ty);
        let bad_record = sub.fresh_bound(Structure::Record(bad_fields, None));
        assert!(!check_instance(
            &mut sub,
            &table,
            &HashMap::new(),
            "Eq",
            bad_record
        ));
    }

    #[test]
    fn a_parametric_builtin_instance_recurses_into_its_own_element_type() {
        // List Weird's Eq must fail even though List itself has a seeded Eq
        // entry -- the whole point of `requires` (Fix #4).
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Eq", Ref::Builtin("Int".to_string()), vec![]);
        table.insert_builtin(
            "Eq",
            Ref::Builtin("List".to_string()),
            vec![(0, "Eq".to_string())],
        );
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let weird_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Weird".to_string()), vec![]));
        let list_int = sub.fresh_bound(Structure::App(
            Ref::Builtin("List".to_string()),
            vec![int_ty],
        ));
        let list_weird = sub.fresh_bound(Structure::App(
            Ref::Builtin("List".to_string()),
            vec![weird_ty],
        ));

        assert!(check_instance(
            &mut sub,
            &table,
            &HashMap::new(),
            "Eq",
            list_int
        ));
        assert!(!check_instance(
            &mut sub,
            &table,
            &HashMap::new(),
            "Eq",
            list_weird
        ));
    }

    #[test]
    fn nested_containers_resolve_two_levels_deep() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Eq", Ref::Builtin("Int".to_string()), vec![]);
        table.insert_builtin(
            "Eq",
            Ref::Builtin("List".to_string()),
            vec![(0, "Eq".to_string())],
        );
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let list_int = sub.fresh_bound(Structure::App(
            Ref::Builtin("List".to_string()),
            vec![int_ty],
        ));
        let list_list_int = sub.fresh_bound(Structure::App(
            Ref::Builtin("List".to_string()),
            vec![list_int],
        ));
        assert!(check_instance(
            &mut sub,
            &table,
            &HashMap::new(),
            "Eq",
            list_list_int
        ));
    }

    #[test]
    fn a_declared_parametric_instance_populates_requires_from_its_own_constraints() {
        let cs = decls(
            "type Box a = Box a\n\
             instance Eq a => Eq (Box a) where\n  (==) x y = True\n",
        );
        let (table, errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(errors.is_empty(), "{errors:?}");

        let mut sub = Substitution::new();
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let weird_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Weird".to_string()), vec![]));
        let box_ref = Ref::TopLevel("Box".to_string());
        let box_int = sub.fresh_bound(Structure::App(box_ref.clone(), vec![int_ty]));
        let box_weird = sub.fresh_bound(Structure::App(box_ref, vec![weird_ty]));

        // Int itself needs its own seeded Eq for the recursion to succeed.
        let mut table_with_int = table;
        table_with_int.insert_builtin("Eq", Ref::Builtin("Int".to_string()), vec![]);
        assert!(check_instance(
            &mut sub,
            &table_with_int,
            &HashMap::new(),
            "Eq",
            box_int
        ));
        assert!(!check_instance(
            &mut sub,
            &table_with_int,
            &HashMap::new(),
            "Eq",
            box_weird
        ));
    }

    // -- end-to-end: TM3 (constrain) + TM5 (solve) + TM6 (this table) --

    #[test]
    fn a_user_declared_instance_satisfies_a_concrete_use_end_to_end() {
        let cs = decls(
            "type Shape = Circle Float\n\
             instance Eq Shape where\n  (==) a b = True\n\
             compareShapes :: Shape -> Shape -> Bool\n\
             compareShapes a b = a == b\n",
        );
        let (table, table_errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(table_errors.is_empty(), "{table_errors:?}");

        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        // Fix #5 now actually type-checks `(==) a b = True`'s own body, so
        // `True` needs to resolve -- TM8 hasn't wired up the real prelude in
        // this test module, so seed it directly (same as solve.rs's own
        // tests do).
        let bool_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Bool".to_string()), vec![]));
        env.insert(
            crate::solve::SchemeKey::Builtin("True".to_string()),
            crate::ty::Scheme::monomorphic(bool_ty),
        );
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");

        check_pending(&mut sub, &table, &HashMap::new(), pending, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_concrete_use_with_no_declared_instance_is_a_no_instance_error() {
        let cs = decls(
            "type Shape = Circle Float\n\
             compareShapes :: Shape -> Shape -> Bool\n\
             compareShapes a b = a == b\n",
        );
        let (table, table_errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(table_errors.is_empty(), "{table_errors:?}");

        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");

        check_pending(&mut sub, &table, &HashMap::new(), pending, &mut errors);
        assert!(errors.iter().any(
            |e| matches!(&e.kind, TypeErrorKind::NoInstance { interface } if interface == "Eq")
        ));
    }

    // -- Fix #5: instance method bodies are checked against their interface --

    #[test]
    fn an_instance_methods_body_returning_the_wrong_type_is_a_real_error() {
        // `5`'s own type is `Num a => a`, not hard-wired to `Int` (see
        // `constrain::expr`'s own doc comment on `CExpr::IntLit`) -- an
        // unbound flexible variable unifies with *any* concrete structure,
        // `Bool` included, so this no longer fails at the raw `unify`
        // level. The real error now only surfaces once `check_pending`
        // finds `Bool` has no `Num` instance -- exactly mirroring what a
        // real Haskell `f :: Bool; f = 5` reports (`No instance for (Num
        // Bool)`), not a unification mismatch.
        let cs = decls("type Shape = Circle Float\ninstance Eq Shape where\n  (==) a b = 5\n");
        let (table, table_errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(table_errors.is_empty(), "{table_errors:?}");

        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");

        check_pending(&mut sub, &table, &HashMap::new(), pending, &mut errors);
        assert!(
            errors.iter().any(
                |e| matches!(&e.kind, TypeErrorKind::NoInstance { interface } if interface == "Num")
            ),
            "expected a NoInstance(\"Num\") error, got {errors:?}"
        );
    }

    #[test]
    fn an_instance_methods_correct_body_type_checks_with_no_errors() {
        let cs = decls("type Shape = Circle Float\ninstance Eq Shape where\n  (==) a b = True\n");
        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        let bool_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Bool".to_string()), vec![]));
        env.insert(
            crate::solve::SchemeKey::Builtin("True".to_string()),
            crate::ty::Scheme::monomorphic(bool_ty),
        );
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// `type Box a = Box a`'s own constructor scheme -- user-defined ADT
    /// constructors (`Ref::TopLevel`, per `knot-canonical::env::
    /// declare_ctor`/`resolve_ctor`) are never auto-seeded by this crate's
    /// own machinery (a real, separate, pre-existing gap unrelated to Fix
    /// #5 -- `prelude.rs`'s own doc comment only ever covers *built-in*
    /// constructors like `Just`/`Ok`), so a test that actually constructs
    /// or pattern-matches one has to seed it by hand, same as `prelude.rs`
    /// itself does for `Just`/`Nothing`/`Ok`/`Err`.
    fn seed_box_ctor(sub: &mut Substitution, env: &mut crate::solve::SchemeEnv) {
        let a = sub.fresh_unbound();
        let box_ref = Ref::TopLevel("Box".to_string());
        let box_a = sub.fresh_bound(Structure::App(box_ref, vec![a]));
        let ctor_ty = sub.fresh_bound(Structure::Fn(a, box_a));
        env.insert(
            crate::solve::SchemeKey::TopLevel("Box".to_string()),
            crate::ty::Scheme {
                vars: vec![a],
                constraints: vec![],
                ty: ctor_ty,
            },
        );
    }

    #[test]
    fn a_parametric_instance_methods_body_may_use_its_own_given_context() {
        // instance Eq a => Eq (Box a) where (==) (Box x) (Box y) = x == y --
        // the body's own `x == y` needs `Eq a`, which the instance's own
        // declared context grants as a `given` fact (Constraint::Given),
        // not something this method's body has to prove itself.
        let cs = decls(
            "type Box a = Box a\n\
             instance Eq a => Eq (Box a) where\n  (==) (Box x) (Box y) = x == y\n",
        );
        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        seed_box_ctor(&mut sub, &mut env);
        let (pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        // The inner `x == y`'s Eq obligation should be on the rigid `a`,
        // satisfied by the instance's own given context -- so nothing
        // should be left dangling in `pending` either.
        assert!(pending.is_empty(), "{pending:?}");
    }

    #[test]
    fn a_parametric_instance_body_using_more_than_its_context_grants_is_a_rigid_instance_error() {
        // Same shape as above, but the instance declares no Ord context for
        // `a`, even though the body needs it.
        let cs = decls(
            "type Box a = Box a\n\
             instance Eq a => Eq (Box a) where\n  (==) (Box x) (Box y) = x < y\n",
        );
        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        seed_box_ctor(&mut sub, &mut env);
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(
            errors.iter().any(|e| matches!(
                &e.kind,
                TypeErrorKind::NoInstanceForRigid { interface } if interface == "Ord"
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn an_unknown_method_name_is_silently_skipped_not_type_checked() {
        // `bogus` isn't one of Eq's own methods -- constrain_instance skips
        // it rather than erroring (see its own doc comment); the *known*
        // `==` method alongside it still gets checked normally.
        let cs = decls(
            "type Shape = Circle Float\n\
             instance Eq Shape where\n  (==) a b = True\n  bogus x = x + 1\n",
        );
        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        let bool_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Bool".to_string()), vec![]));
        env.insert(
            crate::solve::SchemeKey::Builtin("True".to_string()),
            crate::ty::Scheme::monomorphic(bool_ty),
        );
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
    }

    // -- Collection/Context instance methods (closing Fix #5's own gap) --

    fn seed_box_ctor_local(sub: &mut Substitution, env: &mut crate::solve::SchemeEnv) {
        crate::constrain::decl::seed_user_constructors(sub, env, &decls("type Box a = Box a\n"));
    }

    #[test]
    fn a_collection_instances_methods_are_checked_against_their_real_shapes() {
        let cs = decls(concat!(
            "type Box a = Box a\n",
            "instance Collection Box where\n",
            "  map f b =\n",
            "    case b of\n",
            "      Box x -> Box (f x)\n",
            "  foldl f z b =\n",
            "    case b of\n",
            "      Box x -> f z x\n",
            "  foldr f z b =\n",
            "    case b of\n",
            "      Box x -> f x z\n",
            "  filter f b = b\n",
            "  length b = 1\n",
        ));
        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        seed_box_ctor_local(&mut sub, &mut env);
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_context_instances_methods_are_checked_against_their_real_shapes() {
        let cs = decls(concat!(
            "type Box a = Box a\n",
            "instance Context Box where\n",
            "  pure x = Box x\n",
            "  bind b f =\n",
            "    case b of\n",
            "      Box x -> f x\n",
        ));
        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        seed_box_ctor_local(&mut sub, &mut env);
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_collection_instances_wrong_shaped_method_body_is_a_real_error() {
        // map must return `Box b`, not an Int -- but `5`'s own type is
        // `Num a => a` (see `constrain::expr`'s own doc comment on
        // `CExpr::IntLit`), so it unifies with `Box b`'s own structure
        // just fine at the raw `unify` level (an unbound variable unifies
        // with anything). The real error only surfaces once `check_
        // pending` finds `Box` has no `Num` instance -- same story as
        // `an_instance_methods_body_returning_the_wrong_type_is_a_real_
        // error` above, just against a compound `Box b` shape instead of
        // a plain nominal `Bool`.
        let cs = decls("type Box a = Box a\ninstance Collection Box where\n  map f b = 5\n");
        let (table, table_errors) = build_instance_table(&cs, &InstanceTable::new());
        assert!(table_errors.is_empty(), "{table_errors:?}");

        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        seed_box_ctor_local(&mut sub, &mut env);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");

        check_pending(&mut sub, &table, &HashMap::new(), pending, &mut errors);
        assert!(
            errors.iter().any(
                |e| matches!(&e.kind, TypeErrorKind::NoInstance { interface } if interface == "Num")
            ),
            "expected a NoInstance(\"Num\") error, got {errors:?}"
        );
    }

    #[test]
    fn an_unknown_collection_method_name_is_silently_skipped() {
        let cs = decls("type Box a = Box a\ninstance Collection Box where\n  bogus x = x\n");
        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        seed_box_ctor_local(&mut sub, &mut env);
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
    }
}
