//! `knot-canonical` — resolves a `knot_syntax` AST's names (variables,
//! constructors, types, interfaces, imports) into a Canonical AST: the
//! post-parse, pre-type-check stage (Elm's own terminology for the same
//! phase in its compiler). Nothing here does type inference or dictionary
//! resolution — see `TODO.txt`'s "type analysis + dictionary passing stuff"
//! for that separate, later stage — this crate only answers "does every name
//! used actually refer to something, and to what."
//!
//! Two entry points, mirroring `knot_syntax`'s own `parse` vs. `parse_decls`
//! split — see `env.rs`'s module docs for why qualified-reference checking
//! differs between them.
//!
//! **Precondition**: the input has already passed
//! `knot_syntax::validate::validate_module` (automatic if it came from
//! `knot_syntax::parse`) — duplicate top-level names and tuple arity are
//! checked there and aren't re-checked here.
//!
//! Unlike `knot_syntax::ParseError`, resolution never stops at the first
//! problem: every error found across the whole tree is collected and returned
//! together (see `error.rs`).

pub mod ast;
pub mod env;
pub mod error;
pub mod prelude;
mod resolve;

pub use env::ModuleRegistry;
pub use error::{CanonError, CanonErrorKind};

use knot_syntax::ast::decl::{Decl, Module};
use knot_syntax::span::Spanned;

/// Canonicalizes a full module. `registry`, if given, lets qualified/exposed
/// references be checked against what the imported module actually exports;
/// without one, that specific question is resolved permissively (see
/// `env.rs`) since there's no project-wide module loader yet.
pub fn canonicalize_module(
    module: &Module,
    registry: Option<&dyn ModuleRegistry>,
) -> Result<ast::CModule, Vec<CanonError>> {
    let (cmodule, errors) = resolve::decl::resolve_module(module, registry);
    if errors.is_empty() {
        Ok(cmodule)
    } else {
        Err(errors)
    }
}

/// Canonicalizes a bare declaration list with no module header — the same
/// shape `knot_syntax::ParseState::parse_decls` produces. There is no import
/// list at all in this mode, so a qualified reference (`Map.fromList`) is
/// trusted at face value rather than requiring a matching `import`; see
/// `env.rs`. Unqualified names are still checked in both modes.
pub fn canonicalize_decls(
    decls: &[Spanned<Decl>],
) -> Result<Vec<Spanned<ast::CDecl>>, Vec<CanonError>> {
    let (cdecls, errors) = resolve::decl::resolve_decls(decls);
    if errors.is_empty() {
        Ok(cdecls)
    } else {
        Err(errors)
    }
}
