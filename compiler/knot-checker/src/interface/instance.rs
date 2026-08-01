//! The instance table: which `(interface, head type)` pairs actually have
//! an instance. Built from a module's own `CInstanceDecl`s here (plan §6's
//! `interface/instance.rs`); seeding the *built-in* instances (`Num Int`,
//! ...) is `insert_builtin`'s job, called from wherever TM8 ends up living.
//!
//! **Coherence and superclass existence** (plan §3) are checked when a
//! *declared* instance is inserted — at most one instance per `(interface,
//! head)` pair, and a superclass's own instance must already be present
//! (`instance Ord Shape` needs `Eq Shape` first). Built-ins are trusted
//! unconditionally (`insert_builtin` never checks either): they're
//! hand-written by this compiler, not user input to validate.
//!
//! **Not yet handled**: only `Structure::App`-headed obligations (an
//! ordinary named type, built-in or user-defined) can be checked against
//! this table at all — see `check_pending`. Extending `Eq`/`Ord`/`Show` to
//! structural types (`Tuple`, `Record`) needs matching on the *shape* of a
//! `Structure`, not a `Ref`, which is a different (and, notably, a
//! genuinely *recursive* — a tuple's own `Eq`-ness depends on each
//! element's) kind of lookup than this table does. A `Tuple`/`Record`/`Fn`/
//! `Unit`-headed (or still-unresolved) obligation is left unresolved rather
//! than either wrongly confirmed or wrongly rejected.
//!
//! Also not yet handled: a *parametric* instance's own constraints (e.g.
//! `instance Eq a => Eq (List a)`'s `Eq a` requirement) aren't checked
//! recursively here — `has_instance` only confirms the head has an entry at
//! all. Actually verifying the element type's own instance is exactly the
//! "real algorithm, not just a lookup" the plan's §3 flags for dictionary
//! *construction* — that's `elaborate.rs`'s job (TM7), which already needs
//! the recursive version to build the dictionary value itself; duplicating
//! a shallower version here for a mere existence check isn't worth it.

use std::collections::HashMap;

use knot_canonical::ast::{CDecl, CType, Ref};
use knot_syntax::span::Spanned;

use crate::error::{TypeError, TypeErrorKind};
use crate::interface::table::{is_known_interface, superclasses};
use crate::solve::PendingInstance;
use crate::ty::Structure;
use crate::var::Substitution;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceEntry {
    BuiltIn,
    Declared,
}

#[derive(Debug, Default)]
pub struct InstanceTable {
    entries: HashMap<(String, Ref), InstanceEntry>,
}

impl InstanceTable {
    pub fn new() -> Self {
        InstanceTable::default()
    }

    pub fn has_instance(&self, interface: &str, head: &Ref) -> bool {
        self.entries
            .contains_key(&(interface.to_string(), head.clone()))
    }

    /// TM8's seam — built-in instances are trusted, not checked.
    pub fn insert_builtin(&mut self, interface: &str, head: Ref) {
        self.entries
            .insert((interface.to_string(), head), InstanceEntry::BuiltIn);
    }

    fn insert_declared(
        &mut self,
        interface: &str,
        head: Ref,
        span: knot_syntax::span::Span,
        errors: &mut Vec<TypeError>,
    ) {
        if self.has_instance(interface, &head) {
            errors.push(TypeError {
                span,
                kind: TypeErrorKind::DuplicateInstance {
                    interface: interface.to_string(),
                },
            });
            return;
        }
        for superclass in superclasses(interface) {
            if !self.has_instance(superclass, &head) {
                errors.push(TypeError {
                    span,
                    kind: TypeErrorKind::MissingSuperclassInstance {
                        interface: interface.to_string(),
                        superclass: superclass.to_string(),
                    },
                });
            }
        }
        self.entries
            .insert((interface.to_string(), head), InstanceEntry::Declared);
    }
}

/// The head type constructor an instance's `target` names —
/// `instance Eq Shape` -> `Shape`'s `Ref`; `instance Eq (List a)` ->
/// `List`'s. `None` for a target this table can't key by `Ref` at all
/// (`Tuple`/`Record`/`Fn`/`Unit`/a bare type variable) — see module docs.
fn head_ref(target: &CType) -> Option<&Ref> {
    match target {
        CType::Named(r, _) => Some(r),
        CType::Var(_) | CType::Fn(..) | CType::Tuple(_) | CType::Record(..) | CType::Unit => None,
    }
}

/// Builds the table from every `CDecl::Instance` in `decls`. Unknown
/// interfaces are skipped (`knot-canonical` already reported that error);
/// `insert_builtin` for the actual built-in seeding happens separately.
pub fn build_instance_table(decls: &[Spanned<CDecl>]) -> (InstanceTable, Vec<TypeError>) {
    let mut table = InstanceTable::new();
    let mut errors = Vec::new();
    for d in decls {
        if let CDecl::Instance(inst) = &d.node {
            if !is_known_interface(&inst.interface) {
                continue;
            }
            if let Some(head) = head_ref(&inst.target) {
                table.insert_declared(&inst.interface, head.clone(), d.span, &mut errors);
            }
        }
    }
    (table, errors)
}

/// Checks every still-unresolved (concrete-typed) obligation `solve::solve`
/// returned against `table`, appending a `TypeErrorKind::NoInstance` for
/// each one with no matching entry. Anything not headed by a plain
/// `Structure::App` (see module docs) is silently left unchecked, not
/// flagged either way.
pub fn check_pending(
    sub: &mut Substitution,
    table: &InstanceTable,
    pending: Vec<PendingInstance>,
    errors: &mut Vec<TypeError>,
) {
    for p in pending {
        if let Some(Structure::App(head, _)) = sub.resolve_structure(p.ty) {
            if !table.has_instance(&p.interface, &head) {
                errors.push(TypeError {
                    span: p.span,
                    kind: TypeErrorKind::NoInstance {
                        interface: p.interface,
                    },
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_instances_are_trusted_unconditionally() {
        let mut table = InstanceTable::new();
        table.insert_builtin("Num", Ref::Builtin("Int".to_string()));
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
        let (table, errors) = build_instance_table(&cs);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(table.has_instance("Eq", &Ref::TopLevel("Shape".to_string())));
        assert!(table.has_instance("Ord", &Ref::TopLevel("Shape".to_string())));
    }

    #[test]
    fn declaring_ord_without_eq_first_is_a_missing_superclass_error() {
        let cs = decls("type Shape = Circle Float\ninstance Ord Shape where\n  compare a b = EQ\n");
        let (_table, errors) = build_instance_table(&cs);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            TypeErrorKind::MissingSuperclassInstance { interface, superclass }
                if interface == "Ord" && superclass == "Eq"
        )));
    }

    #[test]
    fn declaring_the_same_instance_twice_is_a_duplicate_error() {
        let cs = decls(
            "type Shape = Circle Float\n\
             instance Eq Shape where\n  (==) a b = True\n\
             instance Eq Shape where\n  (==) a b = False\n",
        );
        let (_table, errors) = build_instance_table(&cs);
        assert!(errors
            .iter()
            .any(|e| matches!(&e.kind, TypeErrorKind::DuplicateInstance { interface } if interface == "Eq")));
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
        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors.iter().any(
            |e| matches!(&e.kind, TypeErrorKind::NoInstance { interface } if interface == "Num")
        ));
    }

    #[test]
    fn check_pending_accepts_a_seeded_builtin_instance() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Num", Ref::Builtin("Int".to_string()));
        let ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let pending = vec![PendingInstance {
            span: knot_syntax::span::Span::new(0, 0),
            interface: "Num".to_string(),
            ty,
        }];
        let mut errors = Vec::new();
        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn check_pending_leaves_non_app_headed_obligations_unflagged() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_bound(Structure::Tuple(vec![]));
        let pending = vec![PendingInstance {
            span: knot_syntax::span::Span::new(0, 0),
            interface: "Eq".to_string(),
            ty,
        }];
        let mut errors = Vec::new();
        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors.is_empty());
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
        let (table, table_errors) = build_instance_table(&cs);
        assert!(table_errors.is_empty(), "{table_errors:?}");

        let mut sub = Substitution::new();
        let tree = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");

        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_concrete_use_with_no_declared_instance_is_a_no_instance_error() {
        let cs = decls(
            "type Shape = Circle Float\n\
             compareShapes :: Shape -> Shape -> Bool\n\
             compareShapes a b = a == b\n",
        );
        let (table, table_errors) = build_instance_table(&cs);
        assert!(table_errors.is_empty(), "{table_errors:?}");

        let mut sub = Substitution::new();
        let tree = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");

        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors.iter().any(
            |e| matches!(&e.kind, TypeErrorKind::NoInstance { interface } if interface == "Eq")
        ));
    }
}
