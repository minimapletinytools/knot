//! Structural unification over `Structure` (`App`/`Fn`/`Tuple`/`Unit`/
//! `Record`, the last via field-gathering, plan §4) plus the occurs check and
//! correct rigid-variable handling. No dictionary/interface concerns here at
//! all (`HasInstance` obligations are a `constrain`/`solve` concept, later
//! milestones) — this module only ever asks "can these two types be made
//! equal, and if so, what does the substitution need to record."

use std::collections::BTreeMap;

use crate::error::UnifyError;
use crate::ty::Structure;
use crate::var::{Substitution, TypeVarId};

pub fn unify(sub: &mut Substitution, a: TypeVarId, b: TypeVarId) -> Result<(), UnifyError> {
    let (a, b) = (sub.find(a), sub.find(b));
    if a == b {
        return Ok(());
    }
    if sub.is_rigid(a) || sub.is_rigid(b) {
        return unify_rigid(sub, a, b);
    }
    match (sub.resolve_structure(a), sub.resolve_structure(b)) {
        (None, None) => {
            sub.union_unbound(a, b);
            Ok(())
        }
        (None, Some(sb)) => bind(sub, a, sb),
        (Some(sa), None) => bind(sub, b, sa),
        (Some(sa), Some(sb)) => unify_structure(sub, a, b, sa, sb),
    }
}

/// Reached only when `a != b` (checked by the caller) and at least one side
/// is rigid. A rigid variable never unifies with another *distinct* rigid
/// variable or with any concrete structure — either would let a signature's
/// own type variable quietly specialize, which is exactly the guarantee
/// rigidity exists to prevent (plan §5: "a signature would be a lie the
/// checker doesn't actually enforce"). Unifying with a genuinely unbound
/// flexible variable is the normal, expected case (e.g. checking `myMax`'s
/// own body against its `Ord a => a -> a -> a` signature unifies each
/// parameter's fresh inferred type against the rigid `a`) and must succeed.
fn unify_rigid(sub: &mut Substitution, a: TypeVarId, b: TypeVarId) -> Result<(), UnifyError> {
    match (sub.is_rigid(a), sub.is_rigid(b)) {
        (true, true) => Err(UnifyError::Mismatch {
            expected: a,
            actual: b,
        }),
        (true, false) => bind_flexible_to_rigid(sub, b, a),
        (false, true) => bind_flexible_to_rigid(sub, a, b),
        (false, false) => unreachable!("unify_rigid called with neither side rigid"),
    }
}

fn bind_flexible_to_rigid(
    sub: &mut Substitution,
    flexible: TypeVarId,
    rigid: TypeVarId,
) -> Result<(), UnifyError> {
    if sub.resolve_structure(flexible).is_some() {
        return Err(UnifyError::Mismatch {
            expected: rigid,
            actual: flexible,
        });
    }
    sub.link_to(flexible, rigid);
    Ok(())
}

fn bind(sub: &mut Substitution, var: TypeVarId, structure: Structure) -> Result<(), UnifyError> {
    occurs_check(sub, var, &structure)?;
    sub.bind(var, structure);
    Ok(())
}

fn occurs_check(
    sub: &mut Substitution,
    var: TypeVarId,
    structure: &Structure,
) -> Result<(), UnifyError> {
    for child in structure_children(structure) {
        check_var_not_in(sub, var, child)?;
    }
    Ok(())
}

fn check_var_not_in(
    sub: &mut Substitution,
    var: TypeVarId,
    ty: TypeVarId,
) -> Result<(), UnifyError> {
    let root = sub.find(ty);
    if root == var {
        return Err(UnifyError::Occurs { var, in_ty: ty });
    }
    if let Some(structure) = sub.resolve_structure(root) {
        occurs_check(sub, var, &structure)?;
    }
    Ok(())
}

fn structure_children(structure: &Structure) -> Vec<TypeVarId> {
    match structure {
        Structure::App(_, args) => args.clone(),
        Structure::Fn(a, b) => vec![*a, *b],
        Structure::Tuple(elems) => elems.clone(),
        Structure::Record(fields, ext) => {
            let mut children: Vec<TypeVarId> = fields.values().copied().collect();
            children.extend(*ext);
            children
        }
        Structure::Unit => Vec::new(),
    }
}

fn unify_structure(
    sub: &mut Substitution,
    a: TypeVarId,
    b: TypeVarId,
    sa: Structure,
    sb: Structure,
) -> Result<(), UnifyError> {
    match (sa, sb) {
        (Structure::Unit, Structure::Unit) => Ok(()),
        (Structure::App(ra, args_a), Structure::App(rb, args_b))
            if ra == rb && args_a.len() == args_b.len() =>
        {
            for (x, y) in args_a.into_iter().zip(args_b) {
                unify(sub, x, y)?;
            }
            Ok(())
        }
        (Structure::Fn(a1, r1), Structure::Fn(a2, r2)) => {
            unify(sub, a1, a2)?;
            unify(sub, r1, r2)
        }
        (Structure::Tuple(xs), Structure::Tuple(ys)) if xs.len() == ys.len() => {
            for (x, y) in xs.into_iter().zip(ys) {
                unify(sub, x, y)?;
            }
            Ok(())
        }
        (Structure::Record(fields1, ext1), Structure::Record(fields2, ext2)) => {
            unify_record(sub, a, b, fields1, ext1, fields2, ext2)
        }
        _ => Err(UnifyError::Mismatch {
            expected: a,
            actual: b,
        }),
    }
}

/// Field-gathering record unification (plan §2/§4, mirroring Elm's row-
/// polymorphism approach conceptually, not in code): partition into fields
/// both sides declare vs. fields only one side does, unify the shared ones
/// pairwise, then reconcile each side's exclusive fields against the other
/// side's extension variable (see `absorb`/`unify_extensions`).
///
/// Every reconciliation below goes through the top-level `unify` rather than
/// a raw `Substitution::bind` — that's what makes this correct for *chained*
/// extensible records for free: if an extension variable is already bound to
/// an earlier record (from a prior unification), recursing through `unify`
/// lands back in this same function to merge the two record shapes, instead
/// of needing a separate explicit row-flattening pass. It's also what makes
/// a rigid extension variable (spec §3.4/plan §5's row-polymorphic `{ r | x :
/// Float, y : Float }`) correctly refuse to be forced into a concrete shape:
/// `unify`'s existing rigid-vs-structure case rejects that uniformly, with
/// no extra code needed here.
fn unify_record(
    sub: &mut Substitution,
    a: TypeVarId,
    b: TypeVarId,
    fields1: BTreeMap<String, TypeVarId>,
    ext1: Option<TypeVarId>,
    fields2: BTreeMap<String, TypeVarId>,
    ext2: Option<TypeVarId>,
) -> Result<(), UnifyError> {
    let mut only1 = BTreeMap::new();
    let mut only2 = fields2;
    for (name, ty1) in fields1 {
        match only2.remove(&name) {
            Some(ty2) => unify(sub, ty1, ty2)?,
            None => {
                only1.insert(name, ty1);
            }
        }
    }
    // `only2` now holds exactly the fields side 2 declared that side 1 didn't.

    match (only1.is_empty(), only2.is_empty()) {
        (true, true) => unify_extensions(sub, ext1, ext2),
        (true, false) => absorb(sub, ext1, only2, ext2, a, b),
        (false, true) => absorb(sub, ext2, only1, ext1, a, b),
        (false, false) => {
            let (e1, e2) = match (ext1, ext2) {
                (Some(e1), Some(e2)) => (e1, e2),
                // At least one side is closed but missing fields the other
                // side has -- no extension variable to absorb them into.
                _ => {
                    return Err(UnifyError::Mismatch {
                        expected: a,
                        actual: b,
                    })
                }
            };
            // Each side must absorb the other's exclusive fields, and once
            // absorbed, both sides' *further* remainders must be the same
            // unknown row -- a fresh shared variable, not `e1`/`e2` reused
            // directly (that would make each side's own structure reference
            // the other circularly, failing the occurs check for no reason).
            let rest = sub.fresh_unbound();
            let needs1 = sub.fresh_bound(Structure::Record(only2, Some(rest)));
            let needs2 = sub.fresh_bound(Structure::Record(only1, Some(rest)));
            unify(sub, e1, needs1)?;
            unify(sub, e2, needs2)
        }
    }
}

/// One side (`absent_ext`'s owner) is missing `extra_fields`, which the other
/// side declares. If `absent_ext` is `None` (closed — exactly its declared
/// fields, no more), that's unresolvable: `Mismatch`. Otherwise its extension
/// variable must equal a record made of exactly `extra_fields` plus whatever
/// `donor_ext` (the other side's own remaining row) turns out to be.
fn absorb(
    sub: &mut Substitution,
    absent_ext: Option<TypeVarId>,
    extra_fields: BTreeMap<String, TypeVarId>,
    donor_ext: Option<TypeVarId>,
    a: TypeVarId,
    b: TypeVarId,
) -> Result<(), UnifyError> {
    match absent_ext {
        None => Err(UnifyError::Mismatch {
            expected: a,
            actual: b,
        }),
        Some(e) => {
            let needed = sub.fresh_bound(Structure::Record(extra_fields, donor_ext));
            unify(sub, e, needed)
        }
    }
}

/// Both sides declare exactly the same fields — nothing left to reconcile
/// except whether each side allows *more* fields beyond that. Closed+closed
/// is trivially fine; two open rows must describe the same remainder; an
/// open row unified against a closed one must close up to exactly nothing
/// more (forcing its extension variable to unify with an empty closed
/// record) — which correctly fails if that extension variable is rigid.
fn unify_extensions(
    sub: &mut Substitution,
    ext1: Option<TypeVarId>,
    ext2: Option<TypeVarId>,
) -> Result<(), UnifyError> {
    match (ext1, ext2) {
        (None, None) => Ok(()),
        (Some(e1), Some(e2)) => unify(sub, e1, e2),
        (Some(e), None) | (None, Some(e)) => {
            let empty = sub.fresh_bound(Structure::Record(BTreeMap::new(), None));
            unify(sub, e, empty)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knot_canonical::ast::Ref;

    fn app0(sub: &mut Substitution, name: &str) -> TypeVarId {
        sub.fresh_bound(Structure::App(Ref::Builtin(name.to_string()), vec![]))
    }

    fn app1(sub: &mut Substitution, name: &str, arg: TypeVarId) -> TypeVarId {
        sub.fresh_bound(Structure::App(Ref::Builtin(name.to_string()), vec![arg]))
    }

    #[test]
    fn identical_builtin_apps_unify() {
        let mut sub = Substitution::new();
        let a = app0(&mut sub, "Int");
        let b = app0(&mut sub, "Int");
        assert!(unify(&mut sub, a, b).is_ok());
    }

    #[test]
    fn mismatched_builtin_apps_fail() {
        let mut sub = Substitution::new();
        let a = app0(&mut sub, "Int");
        let b = app0(&mut sub, "Float");
        assert!(matches!(
            unify(&mut sub, a, b),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn generic_apps_unify_by_unifying_their_arguments() {
        // List a ~ List Int -- a should resolve to Int.
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        let list_a = app1(&mut sub, "List", a);
        let int_ty = app0(&mut sub, "Int");
        let list_int = app1(&mut sub, "List", int_ty);
        assert!(unify(&mut sub, list_a, list_int).is_ok());
        assert_eq!(sub.resolve_structure(a), sub.resolve_structure(int_ty));
    }

    #[test]
    fn different_heads_fail_even_with_unifiable_args() {
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        let list_a = app1(&mut sub, "List", a);
        let b = sub.fresh_unbound();
        let option_b = app1(&mut sub, "Option", b);
        assert!(matches!(
            unify(&mut sub, list_a, option_b),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn sensitivity_unifies_by_unifying_its_argument_like_any_other_generic() {
        // Per the type-checker plan's "stub Sensitivity" note: Sensitivity a
        // ~ Sensitivity Int unifies (a := Int), with no special-casing beyond
        // the ordinary App rule -- no introspection into `a`'s own shape.
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        let sens_a = app1(&mut sub, "Sensitivity", a);
        let int_ty = app0(&mut sub, "Int");
        let sens_int = app1(&mut sub, "Sensitivity", int_ty);
        assert!(unify(&mut sub, sens_a, sens_int).is_ok());
        assert_eq!(sub.resolve_structure(a), sub.resolve_structure(int_ty));
    }

    #[test]
    fn sensitivity_of_different_concrete_types_do_not_unify() {
        let mut sub = Substitution::new();
        let int_ty = app0(&mut sub, "Int");
        let float_ty = app0(&mut sub, "Float");
        let sens_int = app1(&mut sub, "Sensitivity", int_ty);
        let sens_float = app1(&mut sub, "Sensitivity", float_ty);
        assert!(matches!(
            unify(&mut sub, sens_int, sens_float),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn function_types_unify_argument_and_return_independently() {
        let mut sub = Substitution::new();
        let a1 = sub.fresh_unbound();
        let r1 = sub.fresh_unbound();
        let fn1 = sub.fresh_bound(Structure::Fn(a1, r1));

        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let fn2 = sub.fresh_bound(Structure::Fn(int_ty, bool_ty));

        assert!(unify(&mut sub, fn1, fn2).is_ok());
        assert_eq!(sub.resolve_structure(a1), sub.resolve_structure(int_ty));
        assert_eq!(sub.resolve_structure(r1), sub.resolve_structure(bool_ty));
    }

    #[test]
    fn mismatched_function_argument_fails() {
        let mut sub = Substitution::new();
        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let string_ty = app0(&mut sub, "String");
        let fn1 = sub.fresh_bound(Structure::Fn(int_ty, bool_ty));
        let fn2 = sub.fresh_bound(Structure::Fn(string_ty, bool_ty));
        assert!(matches!(
            unify(&mut sub, fn1, fn2),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn tuples_unify_elementwise() {
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        let b = sub.fresh_unbound();
        let t1 = sub.fresh_bound(Structure::Tuple(vec![a, b]));

        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let t2 = sub.fresh_bound(Structure::Tuple(vec![int_ty, bool_ty]));

        assert!(unify(&mut sub, t1, t2).is_ok());
        assert_eq!(sub.resolve_structure(a), sub.resolve_structure(int_ty));
        assert_eq!(sub.resolve_structure(b), sub.resolve_structure(bool_ty));
    }

    #[test]
    fn mismatched_tuple_arity_fails() {
        let mut sub = Substitution::new();
        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let pair = sub.fresh_bound(Structure::Tuple(vec![int_ty, bool_ty]));
        let triple = sub.fresh_bound(Structure::Tuple(vec![int_ty, bool_ty, int_ty]));
        assert!(matches!(
            unify(&mut sub, pair, triple),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn unit_unifies_with_unit() {
        let mut sub = Substitution::new();
        let u1 = sub.fresh_bound(Structure::Unit);
        let u2 = sub.fresh_bound(Structure::Unit);
        assert!(unify(&mut sub, u1, u2).is_ok());
    }

    #[test]
    fn occurs_check_rejects_an_infinite_type() {
        // a ~ List a must fail: binding a to List a would build a cycle.
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        let list_a = app1(&mut sub, "List", a);
        assert!(matches!(
            unify(&mut sub, a, list_a),
            Err(UnifyError::Occurs { .. })
        ));
    }

    #[test]
    fn occurs_check_catches_indirect_cycles_through_a_function_type() {
        // a ~ (a -> Int) must fail too, not just the single-level case.
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        let int_ty = app0(&mut sub, "Int");
        let fn_ty = sub.fresh_bound(Structure::Fn(a, int_ty));
        assert!(matches!(
            unify(&mut sub, a, fn_ty),
            Err(UnifyError::Occurs { .. })
        ));
    }

    #[test]
    fn rigid_var_unifies_with_a_fresh_flexible_var() {
        // Models ordinary body-checking against a signature: `myMax`'s rigid
        // `a` unifying with a parameter's freshly-inferred flexible type must
        // succeed, not be treated as a mismatch.
        let mut sub = Substitution::new();
        let rigid_a = sub.fresh_rigid("a".to_string());
        let param = sub.fresh_unbound();
        assert!(unify(&mut sub, rigid_a, param).is_ok());
        assert_eq!(sub.find(rigid_a), sub.find(param));
    }

    #[test]
    fn distinct_rigid_vars_never_unify() {
        let mut sub = Substitution::new();
        let a = sub.fresh_rigid("a".to_string());
        let b = sub.fresh_rigid("b".to_string());
        assert!(matches!(
            unify(&mut sub, a, b),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn rigid_var_never_unifies_with_a_concrete_type() {
        let mut sub = Substitution::new();
        let a = sub.fresh_rigid("a".to_string());
        let int_ty = app0(&mut sub, "Int");
        assert!(matches!(
            unify(&mut sub, a, int_ty),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn a_flexible_var_already_linked_to_rigid_still_rejects_a_concrete_type() {
        // Guards against a signature being a lie the checker doesn't actually
        // enforce (plan §5): once `param` has been unified with rigid `a`,
        // it must behave exactly as rigid from then on.
        let mut sub = Substitution::new();
        let rigid_a = sub.fresh_rigid("a".to_string());
        let param = sub.fresh_unbound();
        unify(&mut sub, rigid_a, param).unwrap();

        let int_ty = app0(&mut sub, "Int");
        assert!(matches!(
            unify(&mut sub, param, int_ty),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    // -- records (TM2) --

    fn record(
        sub: &mut Substitution,
        fields: &[(&str, TypeVarId)],
        ext: Option<TypeVarId>,
    ) -> TypeVarId {
        let fields = fields.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        sub.fresh_bound(Structure::Record(fields, ext))
    }

    #[test]
    fn closed_records_with_identical_fields_unify_fieldwise() {
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        let bool_ty = app0(&mut sub, "Bool");
        let r1 = record(&mut sub, &[("x", a)], None);
        let r2 = record(&mut sub, &[("x", bool_ty)], None);
        assert!(unify(&mut sub, r1, r2).is_ok());
        assert_eq!(sub.resolve_structure(a), sub.resolve_structure(bool_ty));
    }

    #[test]
    fn closed_records_with_a_shared_field_type_mismatch_fail() {
        let mut sub = Substitution::new();
        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let r1 = record(&mut sub, &[("x", int_ty)], None);
        let r2 = record(&mut sub, &[("x", bool_ty)], None);
        assert!(matches!(
            unify(&mut sub, r1, r2),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn closed_records_with_different_field_sets_fail() {
        let mut sub = Substitution::new();
        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let r1 = record(&mut sub, &[("x", int_ty)], None);
        let r2 = record(&mut sub, &[("y", bool_ty)], None);
        assert!(matches!(
            unify(&mut sub, r1, r2),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn open_record_absorbs_a_closed_records_extra_field() {
        // { x : Int, y : Bool } (closed) ~ { x : Int | e } (open) should
        // succeed, with `e` resolving to the leftover `{ y : Bool }`.
        let mut sub = Substitution::new();
        let int_ty1 = app0(&mut sub, "Int");
        let int_ty2 = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let closed = record(&mut sub, &[("x", int_ty1), ("y", bool_ty)], None);
        let e = sub.fresh_unbound();
        let open = record(&mut sub, &[("x", int_ty2)], Some(e));

        assert!(unify(&mut sub, closed, open).is_ok());
        match sub.resolve_structure(e) {
            Some(Structure::Record(fields, ext)) => {
                assert_eq!(ext, None);
                assert_eq!(fields.len(), 1);
                assert_eq!(
                    sub.resolve_structure(fields["y"]),
                    sub.resolve_structure(bool_ty)
                );
            }
            other => panic!("expected e to resolve to a closed record, got {other:?}"),
        }
    }

    #[test]
    fn open_record_with_a_field_the_closed_side_lacks_fails() {
        let mut sub = Substitution::new();
        let int_ty1 = app0(&mut sub, "Int");
        let int_ty2 = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let closed = record(&mut sub, &[("x", int_ty1)], None);
        let e = sub.fresh_unbound();
        let open = record(&mut sub, &[("x", int_ty2), ("y", bool_ty)], Some(e));
        assert!(matches!(
            unify(&mut sub, closed, open),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn two_open_records_with_no_exclusive_fields_unify_their_extensions() {
        let mut sub = Substitution::new();
        let int_ty1 = app0(&mut sub, "Int");
        let int_ty2 = app0(&mut sub, "Int");
        let e1 = sub.fresh_unbound();
        let e2 = sub.fresh_unbound();
        let r1 = record(&mut sub, &[("x", int_ty1)], Some(e1));
        let r2 = record(&mut sub, &[("x", int_ty2)], Some(e2));
        assert!(unify(&mut sub, r1, r2).is_ok());
        assert_eq!(sub.find(e1), sub.find(e2));
    }

    #[test]
    fn two_open_records_each_absorb_the_others_exclusive_field() {
        // { x : Int | e1 } ~ { y : Bool | e2 } should succeed: each row is
        // open enough to also contain the other's field.
        let mut sub = Substitution::new();
        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let e1 = sub.fresh_unbound();
        let e2 = sub.fresh_unbound();
        let r1 = record(&mut sub, &[("x", int_ty)], Some(e1));
        let r2 = record(&mut sub, &[("y", bool_ty)], Some(e2));

        assert!(unify(&mut sub, r1, r2).is_ok());

        match sub.resolve_structure(e1) {
            Some(Structure::Record(fields, _)) => {
                assert_eq!(
                    sub.resolve_structure(fields["y"]),
                    sub.resolve_structure(bool_ty)
                );
            }
            other => panic!("expected e1 to resolve to a record containing y, got {other:?}"),
        }
        match sub.resolve_structure(e2) {
            Some(Structure::Record(fields, _)) => {
                assert_eq!(
                    sub.resolve_structure(fields["x"]),
                    sub.resolve_structure(int_ty)
                );
            }
            other => panic!("expected e2 to resolve to a record containing x, got {other:?}"),
        }
    }

    #[test]
    fn rigid_row_variable_refuses_to_be_forced_closed() {
        // Models `distance :: { r | x : Float, y : Float } -> Float`'s own
        // body-check: `r` must stay opaque even when the rest of the record
        // happens to line up exactly with a closed record elsewhere -- a
        // signature's row variable can never be quietly narrowed to "exactly
        // these fields," or the polymorphism guarantee is a lie (plan §5).
        let mut sub = Substitution::new();
        let rigid_r = sub.fresh_rigid("r".to_string());
        let float_ty1 = app0(&mut sub, "Float");
        let float_ty2 = app0(&mut sub, "Float");
        let open = record(&mut sub, &[("x", float_ty1)], Some(rigid_r));
        let closed = record(&mut sub, &[("x", float_ty2)], None);
        assert!(matches!(
            unify(&mut sub, open, closed),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn non_rigid_open_record_closes_to_empty_against_a_matching_closed_one() {
        let mut sub = Substitution::new();
        let e = sub.fresh_unbound();
        let float_ty1 = app0(&mut sub, "Float");
        let float_ty2 = app0(&mut sub, "Float");
        let open = record(&mut sub, &[("x", float_ty1)], Some(e));
        let closed = record(&mut sub, &[("x", float_ty2)], None);
        assert!(unify(&mut sub, open, closed).is_ok());
        assert_eq!(
            sub.resolve_structure(e),
            Some(Structure::Record(BTreeMap::new(), None))
        );
    }

    #[test]
    fn chained_open_record_unification_flattens_via_recursion() {
        // r1 = { x | e1 }, r2 = { y | e2 }: unifying them first makes e1/e2
        // each absorb the other's field plus a shared `rest`. Then unifying
        // r1 against a *third* record { z | e3 } must correctly account for
        // r1's field *and* whatever e1 already picked up (y), not just x --
        // exercising the "extension already bound" case, not only the
        // fresh-variable case the other tests cover.
        let mut sub = Substitution::new();
        let x_ty = app0(&mut sub, "Int");
        let y_ty = app0(&mut sub, "Int");
        let z_ty = app0(&mut sub, "Int");
        let e1 = sub.fresh_unbound();
        let e2 = sub.fresh_unbound();
        let r1 = record(&mut sub, &[("x", x_ty)], Some(e1));
        let r2 = record(&mut sub, &[("y", y_ty)], Some(e2));
        unify(&mut sub, r1, r2).unwrap();

        let e3 = sub.fresh_unbound();
        let r3 = record(&mut sub, &[("z", z_ty)], Some(e3));
        assert!(unify(&mut sub, r1, r3).is_ok());

        // e3 must now account for both of r1's fields beyond z: x directly,
        // and y transitively (via whatever r1's own e1 turned out to need).
        match sub.resolve_structure(e3) {
            Some(Structure::Record(fields, _)) => {
                assert!(fields.contains_key("x"), "e3 should carry x: {fields:?}");
            }
            other => panic!("expected e3 to resolve to a record, got {other:?}"),
        }
    }
}
