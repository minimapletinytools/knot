//! `unify()` (see `unify.rs`) has no notion of *why* two types were being
//! compared — it only ever sees `TypeVarId`s, never a span. `solve.rs` is
//! what has a `Constraint`'s span on hand when it calls `unify`, so it's the
//! one that wraps a bare `UnifyError` into a spanned, module-level
//! `TypeError` — kept as two separate types rather than threading a span
//! through `unify` itself.

use knot_syntax::span::Span;

use crate::var::TypeVarId;

#[derive(Debug, Clone, PartialEq)]
pub enum UnifyError {
    Mismatch {
        expected: TypeVarId,
        actual: TypeVarId,
    },
    /// `var` occurs within `in_ty`'s own structure — binding would build an
    /// infinite type.
    Occurs { var: TypeVarId, in_ty: TypeVarId },
}

/// Collected, not fatal — `solve.rs` gathers every one of these across a
/// whole module instead of stopping at the first, matching
/// `knot-canonical`'s own `CanonError`/`Vec<CanonError>` stance.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeError {
    pub span: Span,
    pub kind: TypeErrorKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeErrorKind {
    Unify(UnifyError),
    /// A `Constraint::Lookup` reference never made it into the scheme
    /// environment — in a fully-wired checker (once TM6/TM8 seed the
    /// prelude) this should never actually fire for a real program, since
    /// `knot-canonical` already rejects genuinely unbound names; it exists
    /// as a defensive/diagnostic case for this crate's own current gaps
    /// (e.g. a builtin not seeded yet).
    UnboundValue(String),
    /// A zero-argument binding generalized over a variable that still
    /// carries an unresolved interface obligation — the arity-based
    /// replacement for Haskell's monomorphism restriction (this session's
    /// design decision, folded into `knot-type-checker-plan.md` §3).
    AmbiguousConstraint {
        interface: String,
    },
    /// A rigid (signature-bound) type variable was asked to satisfy an
    /// interface its own signature never granted — checkable without any
    /// instance table at all, unlike an ordinary `HasInstance` obligation on
    /// a concrete type (that one needs `interface::instance`'s table, right
    /// below).
    NoInstanceForRigid {
        interface: String,
    },
    /// A concrete type's `HasInstance` obligation, checked against
    /// `interface::instance::InstanceTable` (TM6) once solving resolves it —
    /// no built-in or user `instance` declaration provides `interface` for
    /// this type's own head.
    NoInstance {
        interface: String,
    },
    /// Two `instance` declarations for the same `(interface, head type)`
    /// pair — coherence (plan §3): at most one instance per pair, matching
    /// Haskell without extensions.
    DuplicateInstance {
        interface: String,
    },
    /// `instance Ord Shape` declared without an `Eq Shape` instance already
    /// existing — a superclass obligation the closed interface table
    /// (`interface::table`) says this interface requires.
    MissingSuperclassInstance {
        interface: String,
        superclass: String,
    },
    /// An `instance` declaration's own target isn't a type
    /// `interface::instance::InstanceTable` can key an instance by at all —
    /// a bare type variable, function type, tuple, record, or `()` (see
    /// that module's own `head_ref`). `Eq`/`Ord`/`Show` derive
    /// automatically for `Tuple`/`Record`/`Unit` via `check_instance`'s own
    /// hardcoded structural rule, with no declaration needed — an explicit
    /// one there would be silently unreachable dead code, not a real
    /// instance. No other interface has any such fallback, so previously
    /// an explicit instance for one of these shapes just vanished with no
    /// diagnostic at all, surfacing only as a confusing `NoInstance` at
    /// whatever call site tried to use it.
    InstanceTargetNotNominal {
        interface: String,
    },
    /// A `deriving (Interface, ...)` clause named an interface this checker
    /// doesn't know how to derive for the given ADT's own shape — see
    /// `interface::instance::derive_instances`'s own doc comment for
    /// exactly which field shapes qualify as "easy enough" (a bare use of
    /// one of the type's own declared parameters, a canonical self-
    /// reference, a zero-argument concrete builtin type that already has
    /// the interface, or — `Eq`/`Ord`/`Show` only — `Unit`). Reported
    /// rather than silently accepted or left to surface as a confusing
    /// `NoInstance` at some unrelated call site.
    CannotDeriveInterface {
        interface: String,
        type_name: String,
        reason: String,
    },
}
