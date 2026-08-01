//! Structural unification over `Structure` (`App`/`Fn`/`Tuple`/`Unit` —
//! records wait for TM2's field-gathering) plus the occurs check and correct
//! rigid-variable handling. No dictionary/interface concerns here at all
//! (`HasInstance` obligations are a `constrain`/`solve` concept, later
//! milestones) — this module only ever asks "can these two types be made
//! equal, and if so, what does the substitution need to record."

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
        (Structure::Record(..), Structure::Record(..)) => {
            unimplemented!("record unification is TM2 (field-gathering, plan §4) -- not built yet")
        }
        _ => Err(UnifyError::Mismatch {
            expected: a,
            actual: b,
        }),
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
}
