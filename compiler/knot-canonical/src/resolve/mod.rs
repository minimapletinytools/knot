//! Each sibling module here walks one source-AST layer (`Type`, `Pattern`,
//! `Expr`, `Decl`/`Module`) into its Canonical-AST counterpart. Every walk
//! takes `&mut Vec<CanonError>` and keeps going on error rather than bailing
//! out (see `error.rs`) — a single bad reference shouldn't hide every other
//! problem in the same file, matching how a real editor wants all diagnostics
//! at once, not one-at-a-time.

pub mod decl;
pub mod expr;
pub mod pattern;
pub mod ty;

use knot_syntax::span::Span;

use crate::ast::Ref;
use crate::env::UnresolvedKind;
use crate::error::{CanonError, CanonErrorKind};

/// What kind of name a failed lookup was for, so `unresolved_to_ref` can build
/// the right `CanonErrorKind` — the lookup logic in `env.rs` is identical
/// across namespaces, but the error variant reported to the user isn't.
#[derive(Clone, Copy)]
pub enum NameKind {
    Value,
    Ctor,
    Type,
}

/// Turns a failed `Env` lookup into both a recorded error and an
/// `Ref::Unresolved` placeholder so the caller can keep building a `CType`/
/// `CPattern`/`CExpr` node and continue resolving the rest of the tree.
pub fn unresolved_to_ref(
    kind: UnresolvedKind,
    name: &str,
    name_kind: NameKind,
    span: Span,
    errors: &mut Vec<CanonError>,
) -> Ref {
    let error_kind = match kind {
        UnresolvedKind::Unbound => match name_kind {
            NameKind::Value => CanonErrorKind::UnboundVariable(name.to_string()),
            NameKind::Ctor => CanonErrorKind::UnboundConstructor(name.to_string()),
            NameKind::Type => CanonErrorKind::UnboundType(name.to_string()),
        },
        UnresolvedKind::UnknownQualifier => CanonErrorKind::UnknownQualifier(name.to_string()),
        UnresolvedKind::Ambiguous(modules) => CanonErrorKind::AmbiguousImport {
            name: name.to_string(),
            modules,
        },
        UnresolvedKind::NotExported { module } => CanonErrorKind::NotExportedByModule {
            module,
            name: name.to_string(),
        },
    };
    errors.push(CanonError::new(error_kind, span));
    Ref::Unresolved(name.to_string())
}
