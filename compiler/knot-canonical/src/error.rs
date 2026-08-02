//! One flat `CanonError` per problem, in the same spirit as `knot_syntax::ParseError`
//! (see that crate's own reasoning) — a small fixed set of kinds rather than a
//! bespoke type per check. Unlike parsing, name resolution doesn't need a `fatal`
//! flag or a context stack: there's no backtracking/alternative-grammar concept
//! here, so every error found is simply collected and reported (see `lib.rs`).

use knot_syntax::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonErrorKind {
    UnboundVariable(String),
    UnboundConstructor(String),
    UnboundType(String),
    /// A constraint (`Foo a =>`) or `instance Foo ...` names an interface
    /// outside the closed built-in set (spec §2.3/§7).
    UnknownInterface(String),
    /// The same variable name bound twice within one pattern, e.g. `(x, x)` or
    /// `\x x -> ...` — always wrong regardless of type, so this is a scope-level
    /// check rather than something deferred to type checking.
    DuplicateBindingInPattern(String),
    /// A constructor pattern applied to the wrong number of sub-patterns, e.g.
    /// `Circle r w` when `Circle` was declared with one field. Purely a count
    /// check against the constructor's declared arity — doesn't need type
    /// inference, unlike over/under-application of a constructor as an
    /// *expression*, which is genuinely a type error (partial application is
    /// legal there) and is correctly left to the type checker.
    ConstructorArityMismatch {
        name: String,
        expected: usize,
        found: usize,
    },
    /// A type variable used in a `type`/`type alias` body that doesn't appear
    /// in that declaration's own parameter list, e.g. `type Option a = Some b`.
    /// Signature-position type variables need no such check — those are always
    /// implicitly, freely universally quantified (matches Elm/Haskell).
    UnboundTypeVariable {
        name: String,
        decl: String,
    },
    /// The qualifier on `Foo.bar` doesn't match any import's alias or module
    /// path in scope. Always checkable locally (needs only this module's own
    /// import list, never another module's contents) — see `lib.rs` on why this
    /// only applies in `canonicalize_module`, not `canonicalize_decls`.
    UnknownQualifier(String),
    /// The same unqualified name is brought into scope, unqualified, by two or
    /// more different imports (e.g. two `exposing (..)` imports that both
    /// define `map`) — ambiguous regardless of whether either target module's
    /// contents are otherwise known.
    AmbiguousImport {
        name: String,
        modules: Vec<Vec<String>>,
    },
    /// Only ever produced when a `ModuleRegistry` was supplied and it positively
    /// knows the named module doesn't export this name — see `env.rs`.
    NotExportedByModule {
        module: Vec<String>,
        name: String,
    },
    /// A `type alias` whose own expansion never terminates, e.g. `type alias
    /// Bad = Bad`, or a longer cycle through several aliases — see
    /// `resolve::alias`'s own module doc comment on why this can't just be
    /// expanded like a recursive ADT can.
    CyclicTypeAlias(String),
    /// A type alias used with the wrong number of type arguments, e.g. `type
    /// alias Pair a = (a, a)` referenced as bare `Pair` or as `Pair Int
    /// Bool` — checked once during `resolve::alias`'s own expansion, the
    /// same spirit as `ConstructorArityMismatch` above.
    TypeAliasArityMismatch {
        name: String,
        expected: usize,
        found: usize,
    },
    /// An extensible-record alias (`type alias Selectable a = { a |
    /// isSelected : Bool }`) had its own row-extension parameter (`a`)
    /// applied to a concrete argument that isn't itself record-shaped (nor
    /// another still-open row variable) — there's no record to merge
    /// `isSelected` into. `param` is the alias's own declared parameter
    /// name that ended up here; `alias` is the alias itself.
    RecordExtensionNotARecord {
        alias: String,
        param: String,
    },
    /// Merging an extensible-record alias's own fields with its concrete
    /// extension argument's fields found the same field name on both
    /// sides — e.g. `type alias Selectable a = { a | name : Bool }`
    /// applied to a record that already declares its own `name` field.
    RecordExtensionFieldConflict {
        alias: String,
        field: String,
    },
    /// A record spread (`{ ..Name, ... }`) names something this crate has
    /// no field list for at all — an imported alias (no cross-module
    /// linking yet), a builtin type, or an already-reported unbound name.
    /// Distinct from `SpreadTargetNotARecord`: this is "can't even look,"
    /// not "looked, and it isn't one."
    SpreadTargetNotALocalAlias {
        name: String,
    },
    /// A record spread's target, once resolved to a local `type alias`,
    /// isn't a record at all after its own alias chain is fully expanded
    /// (e.g. `type alias Foo = Int`, or an ADT `type` name — those have
    /// variants, not a field list, so they can never be spread either).
    SpreadTargetNotARecord {
        name: String,
    },
    /// A record spread's target alias is still "open" in some way there's
    /// nothing concrete yet to splice — either it declares its own type
    /// parameters (spread syntax takes no arguments, so nothing could fill
    /// them), or its own record body still has an unresolved row-extension
    /// variable.
    SpreadTargetNotClosed {
        name: String,
    },
    /// A record spread's own fields collide with an explicit field already
    /// in the same record literal, or with another spread's fields —
    /// always a hard error, never resolved by shadowing or ordering.
    /// `name` is the *spread's own target* whose field collided, so a
    /// literal with several spreads can tell which one is at fault.
    SpreadFieldConflict {
        name: String,
        field: String,
    },
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonError {
    pub kind: CanonErrorKind,
    pub span: Span,
}

impl CanonError {
    pub fn new(kind: CanonErrorKind, span: Span) -> Self {
        CanonError { kind, span }
    }
}
