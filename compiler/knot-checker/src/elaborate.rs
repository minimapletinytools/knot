//! Dictionary resolution (plan §3/§7): given a concrete `HasInstance`
//! obligation and the fully-solved state `solve::solve` +
//! `interface::instance::build_instance_table` produce, determine exactly
//! which instance answers it. See `ast.rs`'s own doc comment for what's
//! *not* wired up yet (a full `CExpr` -> `TExpr` tree walk) and why.

use knot_syntax::span::Span;

use crate::ast::Dictionary;
use crate::error::{TypeError, TypeErrorKind};
use crate::interface::instance::InstanceTable;
use crate::solve::PendingInstance;
use crate::ty::Structure;
use crate::var::{Substitution, TypeVarId};

/// Resolves one obligation to the `Dictionary` that answers it. `ty` must
/// already be fully resolved (post-`solve::solve`) to a concrete,
/// `Structure::App`-headed type with a matching table entry — anything else
/// (still unresolved, or headed by a structural type `interface::instance`
/// doesn't support yet, spec its own doc comment) reports the same
/// `TypeErrorKind::NoInstance` `interface::instance::check_pending` would,
/// rather than confirming a dictionary that isn't really there.
pub fn resolve_dictionary(
    sub: &mut Substitution,
    table: &InstanceTable,
    interface: &str,
    ty: TypeVarId,
    span: Span,
) -> Result<Dictionary, TypeError> {
    match sub.resolve_structure(ty) {
        Some(Structure::App(head, _)) if table.has_instance(interface, &head) => Ok(Dictionary {
            interface: interface.to_string(),
            head,
        }),
        _ => Err(TypeError {
            span,
            kind: TypeErrorKind::NoInstance {
                interface: interface.to_string(),
            },
        }),
    }
}

/// Resolves every obligation `solve::solve` left unresolved, in one pass —
/// the elaboration-time counterpart of `interface::instance::check_pending`
/// (which performs the same check but only to report an error, never
/// keeping the resolved `Dictionary` — this is what a full elaboration
/// pass, once the tree-walk gap `ast.rs` documents is closed, would thread
/// back into each call site). Collects every dictionary it can and every
/// error it finds, rather than stopping at the first — same stance as
/// `solve::solve` and `knot-canonical`'s own error handling.
pub fn resolve_pending(
    sub: &mut Substitution,
    table: &InstanceTable,
    pending: Vec<PendingInstance>,
) -> (Vec<Dictionary>, Vec<TypeError>) {
    let mut dictionaries = Vec::new();
    let mut errors = Vec::new();
    for p in pending {
        match resolve_dictionary(sub, table, &p.interface, p.ty, p.span) {
            Ok(d) => dictionaries.push(d),
            Err(e) => errors.push(e),
        }
    }
    (dictionaries, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use knot_canonical::ast::Ref;

    fn span() -> Span {
        Span::new(0, 0)
    }

    #[test]
    fn resolves_a_concrete_obligation_with_a_matching_instance() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Num", Ref::Builtin("Int".to_string()));
        let ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));

        let dict = resolve_dictionary(&mut sub, &table, "Num", ty, span()).unwrap();
        assert_eq!(
            dict,
            Dictionary {
                interface: "Num".to_string(),
                head: Ref::Builtin("Int".to_string()),
            }
        );
    }

    #[test]
    fn errors_when_no_instance_exists() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_bound(Structure::App(Ref::Builtin("String".to_string()), vec![]));

        let err = resolve_dictionary(&mut sub, &table, "Num", ty, span()).unwrap_err();
        assert!(matches!(&err.kind, TypeErrorKind::NoInstance { interface } if interface == "Num"));
    }

    #[test]
    fn errors_on_a_still_unresolved_type_variable() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_unbound();

        let err = resolve_dictionary(&mut sub, &table, "Eq", ty, span()).unwrap_err();
        assert!(matches!(&err.kind, TypeErrorKind::NoInstance { .. }));
    }

    #[test]
    fn resolve_pending_splits_confirmed_dictionaries_from_errors() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Num", Ref::Builtin("Int".to_string()));
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let string_ty = sub.fresh_bound(Structure::App(Ref::Builtin("String".to_string()), vec![]));

        let pending = vec![
            PendingInstance {
                span: span(),
                interface: "Num".to_string(),
                ty: int_ty,
            },
            PendingInstance {
                span: span(),
                interface: "Num".to_string(),
                ty: string_ty,
            },
        ];
        let (dictionaries, errors) = resolve_pending(&mut sub, &table, pending);
        assert_eq!(dictionaries.len(), 1);
        assert_eq!(errors.len(), 1);
    }
}
