//! `Type` -> `CType`: resolves every `Named` type reference (built-in, local
//! `type`/`type alias`, or imported) and a signature's interface constraints.
//! Type *variables* need no lookup at all here — they're implicitly,
//! freely universally quantified wherever a signature or record extension row
//! uses one (matches Elm/Haskell), so `Type::Var` passes straight through.
//! The one place a type variable name gets checked is `type`/`type alias`
//! declarations, in `resolve::decl`, since a variant/body using a variable
//! absent from the declaration's own parameter list is unambiguously wrong.

use std::collections::HashSet;

use knot_syntax::ast::ty::{Constraint, Type, TypeSignature};
use knot_syntax::span::Span;

use crate::ast::{CConstraint, CType, CTypeSignature};
use crate::env::Env;
use crate::error::{CanonError, CanonErrorKind};
use crate::prelude;
use crate::resolve::{unresolved_to_ref, NameKind};

/// `span` is the enclosing signature/alias/variant's span — `Type` itself
/// carries no per-node span (same reasoning as `knot_syntax::validate`'s tuple
/// arity checks), so every error inside one `Type` is attributed to it.
pub fn resolve_type(env: &Env, ty: &Type, span: Span, errors: &mut Vec<CanonError>) -> CType {
    match ty {
        Type::Named(name, args) => {
            let cargs = args
                .iter()
                .map(|a| resolve_type(env, a, span, errors))
                .collect();
            let r = match env.resolve_type(name) {
                Ok(r) => r,
                Err(kind) => unresolved_to_ref(kind, name, NameKind::Type, span, errors),
            };
            CType::Named(r, cargs)
        }
        Type::Var(v) => CType::Var(v.clone()),
        Type::Fn(a, b) => CType::Fn(
            Box::new(resolve_type(env, a, span, errors)),
            Box::new(resolve_type(env, b, span, errors)),
        ),
        Type::Tuple(ts) => CType::Tuple(
            ts.iter()
                .map(|t| resolve_type(env, t, span, errors))
                .collect(),
        ),
        Type::Record(fields, ext) => CType::Record(
            fields
                .iter()
                .map(|(name, t)| (name.clone(), resolve_type(env, t, span, errors)))
                .collect(),
            ext.clone(),
        ),
        Type::Unit => CType::Unit,
    }
}

pub fn resolve_constraint(
    constraint: &Constraint,
    span: Span,
    errors: &mut Vec<CanonError>,
) -> CConstraint {
    if !prelude::is_builtin_interface(&constraint.interface) {
        errors.push(CanonError::new(
            CanonErrorKind::UnknownInterface(constraint.interface.clone()),
            span,
        ));
    }
    CConstraint {
        interface: constraint.interface.clone(),
        type_var: constraint.type_var.clone(),
    }
}

pub fn resolve_signature(
    env: &Env,
    sig: &TypeSignature,
    span: Span,
    errors: &mut Vec<CanonError>,
) -> CTypeSignature {
    CTypeSignature {
        constraints: sig
            .constraints
            .iter()
            .map(|c| resolve_constraint(c, span, errors))
            .collect(),
        ty: resolve_type(env, &sig.ty, span, errors),
    }
}

/// Every `Type::Var` name appearing anywhere in `ty`, for the `type`/`type
/// alias` unbound-type-variable check in `resolve::decl`.
pub fn collect_type_vars(ty: &Type, out: &mut HashSet<String>) {
    match ty {
        Type::Var(v) => {
            out.insert(v.clone());
        }
        Type::Named(_, args) => {
            for a in args {
                collect_type_vars(a, out);
            }
        }
        Type::Fn(a, b) => {
            collect_type_vars(a, out);
            collect_type_vars(b, out);
        }
        Type::Tuple(ts) => {
            for t in ts {
                collect_type_vars(t, out);
            }
        }
        Type::Record(fields, ext) => {
            for (_, t) in fields {
                collect_type_vars(t, out);
            }
            if let Some(row_var) = ext {
                out.insert(row_var.clone());
            }
        }
        Type::Unit => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knot_syntax::span::Span;

    fn s() -> Span {
        Span::new(0, 0)
    }

    #[test]
    fn builtin_type_resolves() {
        let env = Env::for_decls();
        let mut errors = Vec::new();
        let ty = Type::Named(
            "List".to_string(),
            vec![Type::Named("Int".to_string(), vec![])],
        );
        let cty = resolve_type(&env, &ty, s(), &mut errors);
        assert!(errors.is_empty());
        assert!(matches!(cty, CType::Named(crate::ast::Ref::Builtin(_), _)));
    }

    #[test]
    fn unknown_type_is_an_error() {
        let env = Env::for_decls();
        let mut errors = Vec::new();
        let ty = Type::Named("Frobnicator".to_string(), vec![]);
        resolve_type(&env, &ty, s(), &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0].kind, CanonErrorKind::UnboundType(_)));
    }

    #[test]
    fn type_var_never_needs_resolution() {
        let env = Env::for_decls();
        let mut errors = Vec::new();
        resolve_type(&env, &Type::Var("a".to_string()), s(), &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn unknown_interface_constraint_is_an_error() {
        let mut errors = Vec::new();
        resolve_constraint(
            &Constraint {
                interface: "Frobnicable".to_string(),
                type_var: "a".to_string(),
            },
            s(),
            &mut errors,
        );
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            errors[0].kind,
            CanonErrorKind::UnknownInterface(_)
        ));
    }

    #[test]
    fn known_interface_constraint_is_fine() {
        let mut errors = Vec::new();
        resolve_constraint(
            &Constraint {
                interface: "Ord".to_string(),
                type_var: "a".to_string(),
            },
            s(),
            &mut errors,
        );
        assert!(errors.is_empty());
    }
}
