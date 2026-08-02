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
//! argument, and `Tuple`/`Record`/`Unit` get hardcoded structural rules
//! (spec: these derive `Eq`/`Ord`/`Show` from their own parts) rather than
//! a table lookup at all — no `Ref` to key them by in the first place.
//! `has_instance` stays a flat existence check underneath (used for
//! coherence: does *this exact* head have *some* instance, full stop,
//! ignoring what its own parameters might need) — `insert_declared_unchecked`'s
//! caller (`build_instance_table`'s own superclass/duplicate passes) only
//! ever needs that shallow question, not the recursive one.
//!
//! **A user can't declare an instance *for* a `Var`/`Fn`/`Tuple`/`Record`/
//! `Unit` shape directly** (`head_ref` returns `None` for those `CType`s —
//! no `Ref` to key an instance-table entry by at all) — only this table's
//! own hardcoded structural rule ever answers for `Eq`/`Ord`/`Show` on
//! `Tuple`/`Record`/`Unit`, and no interface at all can be given to a bare
//! `Fn`/`Var` target. This used to be silent (the declaration just vanished
//! with no diagnostic, surfacing only as a confusing `NoInstance` wherever
//! a caller tried to use it) — `build_instance_table` now reports a real
//! `InstanceTargetNotNominal` at the declaration site instead. A `VarApp`
//! whose head is still unresolved (a genuine kind error, spec §6.3/§6.4) is
//! never confirmed by anything — see `check_instance`'s own doc comment.

use std::collections::HashMap;

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
/// the same rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceEntry {
    pub kind: InstanceKind,
    pub requires: Vec<(usize, String)>,
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
            },
        );
    }

    /// Merges `other`'s own entries into `self`, keeping `self`'s own entry
    /// on a `(interface, head)` collision — `check::check_module`'s own
    /// glue for combining `prelude::seed`'s builtin table with a module's
    /// own `build_instance_table` result. Called with the *declared* table
    /// as `self` so a builtin entry never silently shadows a real user
    /// declaration; a user "redeclaring" an interface a builtin type
    /// already has (`instance Eq Int where ...`) is untested, unusual
    /// territory this doesn't specially diagnose — it just keeps the
    /// user's own entry, same as any other collision here.
    pub fn merge_from(&mut self, other: InstanceTable) {
        for (key, entry) in other.entries {
            self.entries.entry(key).or_insert(entry);
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
            },
        );
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

/// `(interface, head, requires, span)` for one declared instance this table
/// can key by `Ref` — `build_instance_table`'s own intermediate shape
/// between collecting every such declaration and its two checking passes.
type DeclaredInstance<'a> = (&'a str, Ref, Vec<(usize, String)>, knot_syntax::span::Span);

/// Builds the table from every `CDecl::Instance` in `decls`. Unknown
/// interfaces are skipped (`knot-canonical` already reported that error);
/// `insert_builtin` for the actual built-in seeding happens separately.
///
/// **Two passes, deliberately not one.** A single forward pass checking
/// each instance's own superclasses as it's inserted would make coherence
/// depend on declaration order *within the same module* — `instance Ord
/// Shape` followed later by `instance Eq Shape` would wrongly report a
/// missing `Eq` superclass, purely because `Eq` hadn't been registered yet
/// at the point `Ord` was processed, even though both are declared in this
/// same module. Elm/Haskell have no such ordering requirement, so neither
/// should this. Pass 1 registers every instance's own existence first
/// (first occurrence per `(interface, head)` wins; a later duplicate is
/// reported via `DuplicateInstance` and otherwise ignored — not inserted,
/// not superclass-checked in pass 2 either, matching what a single
/// early-return pass would have done for it). Pass 2 then checks every
/// *accepted* instance's own superclasses against the now-fully-populated
/// table, so order within the module no longer matters.
pub fn build_instance_table(decls: &[Spanned<CDecl>]) -> (InstanceTable, Vec<TypeError>) {
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
        match head_ref(&inst.target) {
            Some(head) => declared.push((
                inst.interface.as_str(),
                head.clone(),
                instance_requires(&inst.target, &inst.constraints),
                d.span,
            )),
            None => errors.push(TypeError {
                span: d.span,
                kind: TypeErrorKind::InstanceTargetNotNominal {
                    interface: inst.interface.clone(),
                },
            }),
        }
    }

    let mut accepted = vec![false; declared.len()];
    for (i, (interface, head, requires, span)) in declared.iter().enumerate() {
        if table.has_instance(interface, head) {
            errors.push(TypeError {
                span: *span,
                kind: TypeErrorKind::DuplicateInstance {
                    interface: interface.to_string(),
                },
            });
        } else {
            table.insert_declared_unchecked(interface, head.clone(), requires.clone());
            accepted[i] = true;
        }
    }

    for (i, (interface, head, _, span)) in declared.iter().enumerate() {
        if !accepted[i] {
            continue;
        }
        for superclass in superclasses(interface) {
            if !table.has_instance(superclass, head) {
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
pub fn check_instance(
    sub: &mut Substitution,
    table: &InstanceTable,
    interface: &str,
    ty: TypeVarId,
) -> bool {
    match sub.resolve_structure(ty) {
        Some(Structure::App(head, args)) => match table.entry(interface, &head) {
            Some(entry) => entry
                .requires
                .iter()
                .all(|(pos, req_interface)| check_instance(sub, table, req_interface, args[*pos])),
            None => false,
        },
        Some(Structure::Ctor(head)) => table.has_instance(interface, &head),
        // Structural facts, not table lookups -- gated to exactly `Eq`/
        // `Ord`/`Show` (same as `Unit` below), so a dangling `Num (Int,
        // Int)` obligation (nonsensical -- Knot has no tuple arithmetic)
        // correctly still fails instead of wrongly inheriting `Int`'s own
        // `Num` instance through the recursion.
        Some(Structure::Tuple(elems)) if matches!(interface, "Eq" | "Ord" | "Show") => elems
            .iter()
            .all(|e| check_instance(sub, table, interface, *e)),
        Some(Structure::Record(fields, _)) if matches!(interface, "Eq" | "Ord" | "Show") => fields
            .values()
            .all(|f| check_instance(sub, table, interface, *f)),
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
/// VarApp`'s own doc comment), not a missing-instance one.
pub fn check_pending(
    sub: &mut Substitution,
    table: &InstanceTable,
    pending: Vec<PendingInstance>,
    errors: &mut Vec<TypeError>,
) {
    for p in pending {
        let is_resolved = sub.resolve_structure(p.ty).is_some();
        if is_resolved && !check_instance(sub, table, &p.interface, p.ty) {
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
        let (table, errors) = build_instance_table(&cs);
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
        let (_table, errors) = build_instance_table(&cs);
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
        let (_table, errors) = build_instance_table(&cs);
        assert!(errors
            .iter()
            .any(|e| matches!(&e.kind, TypeErrorKind::DuplicateInstance { interface } if interface == "Eq")));
    }

    #[test]
    fn a_non_eq_ord_show_instance_targeting_a_record_is_a_real_error_not_silent() {
        // Semigroup has no structural fallback the way Eq/Ord/Show do --
        // this used to just vanish from the table with no diagnostic at
        // all, only surfacing later as a confusing NoInstance wherever a
        // caller tried to use `<>` on a Point.
        let cs = decls("instance Semigroup { x : Float } where\n  (<>) a b = { x = a.x }\n");
        let (table, errors) = build_instance_table(&cs);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            TypeErrorKind::InstanceTargetNotNominal { interface } if interface == "Semigroup"
        )));
        // And it really isn't registered anywhere -- the diagnostic isn't
        // just cosmetic alongside a table entry that still works.
        assert!(table.entries.is_empty());
    }

    #[test]
    fn an_eq_instance_targeting_a_tuple_is_also_reported_even_though_eq_derives_structurally() {
        // Eq/Ord/Show *do* derive automatically for Tuple/Record/Unit via
        // check_instance's own hardcoded rule, with no declaration needed
        // -- so an explicit one here would be unreachable dead code, not a
        // working instance. Reported the same way as any other
        // non-nominal target, for the same reason: silence would be
        // actively misleading (it'd look like the explicit body matters).
        let cs = decls("instance Eq (Int, Int) where\n  (==) a b = True\n");
        let (_table, errors) = build_instance_table(&cs);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            TypeErrorKind::InstanceTargetNotNominal { interface } if interface == "Eq"
        )));
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
        table.insert_builtin("Num", Ref::Builtin("Int".to_string()), vec![]);
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
    fn check_pending_resolves_a_ctor_headed_obligation_against_the_table() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Collection", Ref::Builtin("List".to_string()), vec![]);
        let ty = sub.fresh_bound(Structure::Ctor(Ref::Builtin("List".to_string())));
        let pending = vec![PendingInstance {
            span: knot_syntax::span::Span::new(0, 0),
            interface: "Collection".to_string(),
            ty,
        }];
        let mut errors = Vec::new();
        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn check_pending_flags_a_ctor_headed_obligation_with_no_instance() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_bound(Structure::Ctor(Ref::Builtin("IO".to_string())));
        let pending = vec![PendingInstance {
            span: knot_syntax::span::Span::new(0, 0),
            interface: "Collection".to_string(),
            ty,
        }];
        let mut errors = Vec::new();
        check_pending(&mut sub, &table, pending, &mut errors);
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
        check_pending(&mut sub, &table, pending, &mut errors);
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
        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn empty_tuple_trivially_satisfies_eq_ord_show() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_bound(Structure::Tuple(vec![]));
        for interface in ["Eq", "Ord", "Show"] {
            assert!(check_instance(&mut sub, &table, interface, ty));
        }
    }

    #[test]
    fn a_function_is_never_eq_ord_or_show() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let a = sub.fresh_unbound();
        let fn_ty = sub.fresh_bound(Structure::Fn(a, a));
        assert!(!check_instance(&mut sub, &table, "Eq", fn_ty));
    }

    #[test]
    fn tuple_is_eq_only_if_every_element_is() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Eq", Ref::Builtin("Int".to_string()), vec![]);
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let weird_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Weird".to_string()), vec![]));

        let ok_pair = sub.fresh_bound(Structure::Tuple(vec![int_ty, int_ty]));
        assert!(check_instance(&mut sub, &table, "Eq", ok_pair));

        let bad_pair = sub.fresh_bound(Structure::Tuple(vec![int_ty, weird_ty]));
        assert!(!check_instance(&mut sub, &table, "Eq", bad_pair));
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
        assert!(!check_instance(&mut sub, &table, "Num", pair));
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
        assert!(check_instance(&mut sub, &table, "Eq", ok_record));

        let mut bad_fields = std::collections::BTreeMap::new();
        bad_fields.insert("x".to_string(), weird_ty);
        let bad_record = sub.fresh_bound(Structure::Record(bad_fields, None));
        assert!(!check_instance(&mut sub, &table, "Eq", bad_record));
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

        assert!(check_instance(&mut sub, &table, "Eq", list_int));
        assert!(!check_instance(&mut sub, &table, "Eq", list_weird));
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
        assert!(check_instance(&mut sub, &table, "Eq", list_list_int));
    }

    #[test]
    fn a_declared_parametric_instance_populates_requires_from_its_own_constraints() {
        let cs = decls(
            "type Box a = Box a\n\
             instance Eq a => Eq (Box a) where\n  (==) x y = True\n",
        );
        let (table, errors) = build_instance_table(&cs);
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
        assert!(check_instance(&mut sub, &table_with_int, "Eq", box_int));
        assert!(!check_instance(&mut sub, &table_with_int, "Eq", box_weird));
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
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");

        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors.iter().any(
            |e| matches!(&e.kind, TypeErrorKind::NoInstance { interface } if interface == "Eq")
        ));
    }

    // -- Fix #5: instance method bodies are checked against their interface --

    #[test]
    fn an_instance_methods_body_returning_the_wrong_type_is_a_real_error() {
        let cs = decls("type Shape = Circle Float\ninstance Eq Shape where\n  (==) a b = 5\n");
        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(
            errors
                .iter()
                .any(|e| matches!(&e.kind, TypeErrorKind::Unify(_))),
            "expected a Unify error (Int where Bool expected), got {errors:?}"
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
        // map must return `Box b`, not an Int.
        let cs = decls("type Box a = Box a\ninstance Collection Box where\n  map f b = 5\n");
        let mut sub = Substitution::new();
        let (tree, _members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        seed_box_ctor_local(&mut sub, &mut env);
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(
            errors
                .iter()
                .any(|e| matches!(&e.kind, TypeErrorKind::Unify(_))),
            "expected a Unify error, got {errors:?}"
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
