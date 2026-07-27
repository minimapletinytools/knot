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
