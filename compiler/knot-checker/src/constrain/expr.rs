//! Constraint generation over `CExpr` (mirrors Elm's own
//! `Constrain/Expression.hs`, conceptually). Each function returns the
//! expression's own inferred type; relating two types is always done by
//! pushing a `Constraint::Equal`, never by calling `unify` directly — see
//! `constrain/mod.rs`'s module docs on why generation and solving stay
//! separate passes.
//!
//! **Not yet handled** (each `todo!()`s with a reason): `BinOp`/`Negate`
//! need an operator → interface-and-shape table (`Num`/`Ord`/`Eq`/... —
//! spec §6) that doesn't exist yet; `Let` needs TM4's SCC dependency
//! splitting to build a real `Constraint::Let`; `Do` needs the Context
//! interface's `pure`/`bind` dictionary story (spec §6.4), which is
//! entangled with the same interface table `BinOp` needs. `Annotated`,
//! by contrast, *is* handled — deliberately shallow: its annotation values
//! aren't constrained here at all (that's TM6's annotation-checking layer,
//! plan §3.5), so this pass only ever looks straight through to the target
//! expression.

use knot_canonical::ast::{CExpr, Ref};
use knot_syntax::span::Spanned;

use crate::constrain::pattern::constrain_pattern;
use crate::constrain::{Constraint, LocalScope};
use crate::ty::Structure;
use crate::var::{Substitution, TypeVarId};

fn app0(sub: &mut Substitution, name: &str) -> TypeVarId {
    sub.fresh_bound(Structure::App(Ref::Builtin(name.to_string()), vec![]))
}

fn app1(sub: &mut Substitution, name: &str, arg: TypeVarId) -> TypeVarId {
    sub.fresh_bound(Structure::App(Ref::Builtin(name.to_string()), vec![arg]))
}

/// Resolves a `Var`/`Ctor` reference: immediate for `Ref::Local` (never
/// generalized, so no constraint needed at all), deferred via `Lookup` for
/// everything else (see `constrain/mod.rs`). `Ref::Unresolved` is neither —
/// `knot-canonical` already recorded the real error for it, so this returns
/// a bare, unconstrained fresh variable rather than compounding that error
/// with a confusing downstream one.
fn constrain_name_ref(
    sub: &mut Substitution,
    scope: &LocalScope,
    reference: &Ref,
    span: knot_syntax::span::Span,
    constraints: &mut Vec<Constraint>,
) -> TypeVarId {
    match reference {
        Ref::Local(name) => scope.lookup(name),
        Ref::Unresolved(_) => sub.fresh_unbound(),
        _ => {
            let ty = sub.fresh_unbound();
            constraints.push(Constraint::Lookup {
                span,
                reference: reference.clone(),
                expected: ty,
            });
            ty
        }
    }
}

pub fn constrain_expr(
    sub: &mut Substitution,
    scope: &mut LocalScope,
    expr: &Spanned<CExpr>,
    constraints: &mut Vec<Constraint>,
) -> TypeVarId {
    let span = expr.span;
    match &expr.node {
        CExpr::IntLit(_) => app0(sub, "Int"),
        CExpr::FloatLit(_) => app0(sub, "Float"),
        CExpr::StringLit(_) => app0(sub, "String"),
        CExpr::Unit => sub.fresh_bound(Structure::Unit),
        // A hole's own type is unconstrained by design (it could be
        // anything) -- that a hole must always be a compile error (spec
        // §12.1) is a separate, later diagnostic concern (recording and
        // always rejecting every hole span seen), not a typing constraint,
        // so it isn't handled here.
        CExpr::Hole => sub.fresh_unbound(),
        CExpr::Var(r) | CExpr::Ctor(r) => constrain_name_ref(sub, scope, r, span, constraints),
        CExpr::Lambda(params, body) => {
            scope.push();
            let param_tys: Vec<TypeVarId> = params
                .iter()
                .map(|p| constrain_pattern(sub, scope, p, constraints))
                .collect();
            let body_ty = constrain_expr(sub, scope, body, constraints);
            scope.pop();
            param_tys
                .into_iter()
                .rev()
                .fold(body_ty, |acc, param_ty| {
                    sub.fresh_bound(Structure::Fn(param_ty, acc))
                })
        }
        CExpr::App(f, arg) => {
            let f_ty = constrain_expr(sub, scope, f, constraints);
            let arg_ty = constrain_expr(sub, scope, arg, constraints);
            let result_ty = sub.fresh_unbound();
            let expected_fn_ty = sub.fresh_bound(Structure::Fn(arg_ty, result_ty));
            constraints.push(Constraint::Equal {
                span,
                expected: f_ty,
                actual: expected_fn_ty,
            });
            result_ty
        }
        CExpr::If(cond, then_branch, else_branch) => {
            let cond_ty = constrain_expr(sub, scope, cond, constraints);
            let bool_ty = app0(sub, "Bool");
            constraints.push(Constraint::Equal {
                span,
                expected: cond_ty,
                actual: bool_ty,
            });
            let then_ty = constrain_expr(sub, scope, then_branch, constraints);
            let else_ty = constrain_expr(sub, scope, else_branch, constraints);
            constraints.push(Constraint::Equal {
                span,
                expected: then_ty,
                actual: else_ty,
            });
            then_ty
        }
        CExpr::Case(scrutinee, arms) => {
            let scrutinee_ty = constrain_expr(sub, scope, scrutinee, constraints);
            let result_ty = sub.fresh_unbound();
            for (pattern, body) in arms {
                scope.push();
                let pattern_ty = constrain_pattern(sub, scope, pattern, constraints);
                constraints.push(Constraint::Equal {
                    span,
                    expected: scrutinee_ty,
                    actual: pattern_ty,
                });
                let body_ty = constrain_expr(sub, scope, body, constraints);
                constraints.push(Constraint::Equal {
                    span,
                    expected: result_ty,
                    actual: body_ty,
                });
                scope.pop();
            }
            result_ty
        }
        CExpr::List(elems) => {
            let elem_ty = sub.fresh_unbound();
            for e in elems {
                let e_ty = constrain_expr(sub, scope, e, constraints);
                constraints.push(Constraint::Equal {
                    span,
                    expected: elem_ty,
                    actual: e_ty,
                });
            }
            app1(sub, "List", elem_ty)
        }
        // Arity <= 3 is already enforced post-parse (`knot-syntax::validate`)
        // -- not re-checked here.
        CExpr::Tuple(elems) => {
            let elem_tys = elems
                .iter()
                .map(|e| constrain_expr(sub, scope, e, constraints))
                .collect();
            sub.fresh_bound(Structure::Tuple(elem_tys))
        }
        // A record literal is always closed -- exactly these fields, spec
        // §4.7 -- unlike `FieldAccess`/`RecordUpdate` below, which only ever
        // need an *open* row (the value being accessed/updated might have
        // more fields than the expression cares about).
        CExpr::Record(fields) => {
            let field_tys = fields
                .iter()
                .map(|(name, e)| (name.clone(), constrain_expr(sub, scope, e, constraints)))
                .collect();
            sub.fresh_bound(Structure::Record(field_tys, None))
        }
        CExpr::RecordUpdate(base, updates) => {
            let base_ty = constrain_expr(sub, scope, base, constraints);
            let update_tys = updates
                .iter()
                .map(|(name, e)| (name.clone(), constrain_expr(sub, scope, e, constraints)))
                .collect();
            // `base` only needs to have *at least* the updated fields, at
            // their new types -- the row-polymorphism machinery in
            // `unify::unify_record` figures out the rest, whatever it is.
            let rest = sub.fresh_unbound();
            let required = sub.fresh_bound(Structure::Record(update_tys, Some(rest)));
            constraints.push(Constraint::Equal {
                span,
                expected: base_ty,
                actual: required,
            });
            // The result has exactly `base`'s own shape -- an update can't
            // add or remove fields, only change values.
            base_ty
        }
        CExpr::FieldAccess(base, field) => {
            let base_ty = constrain_expr(sub, scope, base, constraints);
            let field_ty = sub.fresh_unbound();
            let rest = sub.fresh_unbound();
            let mut required_fields = std::collections::BTreeMap::new();
            required_fields.insert(field.clone(), field_ty);
            let required = sub.fresh_bound(Structure::Record(required_fields, Some(rest)));
            constraints.push(Constraint::Equal {
                span,
                expected: base_ty,
                actual: required,
            });
            field_ty
        }
        // Annotation *values* aren't constrained here at all -- see this
        // module's doc comment. Just look straight through to the target.
        CExpr::Annotated(_annotations, target) => constrain_expr(sub, scope, target, constraints),
        CExpr::BinOp(..) => todo!(
            "BinOp needs the operator -> interface/shape table (Num/Ord/Eq/Semigroup/...) -- not built yet"
        ),
        CExpr::Negate(_) => todo!("Negate needs the same Num-interface table BinOp does"),
        CExpr::Let(..) => todo!("Let needs TM4's SCC dependency splitting to build a real Constraint::Let"),
        CExpr::Do(..) => todo!("Do needs the Context interface's pure/bind dictionary story (spec §6.4)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knot_canonical::ast::CPattern;
    use knot_syntax::ast::pattern::PatternLiteral;
    use knot_syntax::span::Span;

    fn s() -> Span {
        Span::new(0, 0)
    }

    fn e(node: CExpr) -> Spanned<CExpr> {
        Spanned::new(s(), node)
    }

    fn p(node: CPattern) -> Spanned<CPattern> {
        Spanned::new(s(), node)
    }

    /// Test-only: solves the `Equal`/`And` constraints these tests produce.
    /// Not the real `solve.rs` -- `Lookup`/`HasInstance`/`Let` aren't
    /// resolvable without a real environment, which doesn't exist yet, so
    /// tests exercising those paths only check the constraint *shape*
    /// instead of round-tripping through this.
    fn solve_equalities(sub: &mut Substitution, constraints: Vec<Constraint>) {
        for c in constraints {
            match c {
                Constraint::Equal {
                    expected, actual, ..
                } => crate::unify::unify(sub, expected, actual).unwrap(),
                Constraint::And(cs) => solve_equalities(sub, cs),
                Constraint::True
                | Constraint::HasInstance { .. }
                | Constraint::Lookup { .. }
                | Constraint::Let { .. } => {}
            }
        }
    }

    fn builtin(sub: &mut Substitution, name: &str) -> Structure {
        let id = app0(sub, name);
        sub.resolve_structure(id).unwrap()
    }

    #[test]
    fn literals_get_their_builtin_types() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let int_ty = constrain_expr(&mut sub, &mut scope, &e(CExpr::IntLit(1)), &mut cs);
        let float_ty = constrain_expr(&mut sub, &mut scope, &e(CExpr::FloatLit(1.0)), &mut cs);
        let str_ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::StringLit("x".to_string())),
            &mut cs,
        );
        let unit_ty = constrain_expr(&mut sub, &mut scope, &e(CExpr::Unit), &mut cs);
        assert_eq!(
            sub.resolve_structure(int_ty),
            Some(builtin(&mut sub, "Int"))
        );
        assert_eq!(
            sub.resolve_structure(float_ty),
            Some(builtin(&mut sub, "Float"))
        );
        assert_eq!(
            sub.resolve_structure(str_ty),
            Some(builtin(&mut sub, "String"))
        );
        assert_eq!(sub.resolve_structure(unit_ty), Some(Structure::Unit));
    }

    #[test]
    fn hole_is_a_bare_unconstrained_fresh_variable() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(&mut sub, &mut scope, &e(CExpr::Hole), &mut cs);
        assert!(cs.is_empty());
        assert!(sub.resolve_structure(ty).is_none());
    }

    #[test]
    fn local_var_resolves_immediately_with_no_constraint() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let bound = sub.fresh_unbound();
        scope.bind("x", bound);
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Var(Ref::Local("x".to_string()))),
            &mut cs,
        );
        assert!(cs.is_empty());
        assert_eq!(ty, bound);
    }

    #[test]
    fn non_local_var_defers_via_a_lookup_constraint() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Var(Ref::Builtin("compare".to_string()))),
            &mut cs,
        );
        assert_eq!(cs.len(), 1);
        assert!(matches!(
            &cs[0],
            Constraint::Lookup { reference, expected, .. }
                if *reference == Ref::Builtin("compare".to_string()) && *expected == ty
        ));
    }

    #[test]
    fn unresolved_var_is_unconstrained_not_double_reported() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Var(Ref::Unresolved("ghost".to_string()))),
            &mut cs,
        );
        assert!(cs.is_empty());
    }

    #[test]
    fn lambda_builds_a_curried_function_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        // \x y -> x  ::  a -> b -> a
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Lambda(
                vec![
                    p(CPattern::Var("x".to_string())),
                    p(CPattern::Var("y".to_string())),
                ],
                Box::new(e(CExpr::Var(Ref::Local("x".to_string())))),
            )),
            &mut cs,
        );
        match sub.resolve_structure(ty) {
            Some(Structure::Fn(x_ty, rest)) => match sub.resolve_structure(rest) {
                Some(Structure::Fn(_y_ty, body_ty)) => {
                    assert_eq!(sub.find(x_ty), sub.find(body_ty));
                }
                other => panic!("expected a second Fn arrow, got {other:?}"),
            },
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn application_unifies_the_function_type_against_arg_and_result() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let f_ty = sub.fresh_bound(Structure::Fn(int_ty, bool_ty));
        scope.bind("f", f_ty);
        let arg_ty = app0(&mut sub, "Int");
        scope.bind("arg", arg_ty);

        let mut cs = Vec::new();
        let result = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::App(
                Box::new(e(CExpr::Var(Ref::Local("f".to_string())))),
                Box::new(e(CExpr::Var(Ref::Local("arg".to_string())))),
            )),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        assert_eq!(
            sub.resolve_structure(result),
            Some(builtin(&mut sub, "Bool"))
        );
    }

    #[test]
    fn if_requires_a_boolean_condition_and_matching_branches() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let cond_ty = app0(&mut sub, "Bool");
        scope.bind("cond", cond_ty);
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::If(
                Box::new(e(CExpr::Var(Ref::Local("cond".to_string())))),
                Box::new(e(CExpr::IntLit(1))),
                Box::new(e(CExpr::IntLit(2))),
            )),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "Int")));
    }

    #[test]
    fn case_unifies_every_arm_body_and_the_scrutinee_against_each_pattern() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let scrutinee_ty = app0(&mut sub, "Int");
        scope.bind("n", scrutinee_ty);
        let mut cs = Vec::new();
        // case n of 1 -> "one" | _ -> "other"
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Case(
                Box::new(e(CExpr::Var(Ref::Local("n".to_string())))),
                vec![
                    (
                        p(CPattern::Literal(PatternLiteral::Int(1))),
                        e(CExpr::StringLit("one".to_string())),
                    ),
                    (
                        p(CPattern::Wildcard(None)),
                        e(CExpr::StringLit("other".to_string())),
                    ),
                ],
            )),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "String")));
    }

    #[test]
    fn list_unifies_every_element_to_the_same_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::List(vec![e(CExpr::IntLit(1)), e(CExpr::IntLit(2))])),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        match sub.resolve_structure(ty) {
            Some(Structure::App(r, args)) if r == Ref::Builtin("List".to_string()) => {
                assert_eq!(
                    sub.resolve_structure(args[0]),
                    Some(builtin(&mut sub, "Int"))
                );
            }
            other => panic!("expected List Int, got {other:?}"),
        }
    }

    #[test]
    fn tuple_pairs_each_elements_own_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Tuple(vec![
                e(CExpr::IntLit(1)),
                e(CExpr::StringLit("x".to_string())),
            ])),
            &mut cs,
        );
        match sub.resolve_structure(ty) {
            Some(Structure::Tuple(elems)) => {
                assert_eq!(
                    sub.resolve_structure(elems[0]),
                    Some(builtin(&mut sub, "Int"))
                );
                assert_eq!(
                    sub.resolve_structure(elems[1]),
                    Some(builtin(&mut sub, "String"))
                );
            }
            other => panic!("expected a Tuple shape, got {other:?}"),
        }
    }

    #[test]
    fn record_literal_is_closed() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Record(vec![("x".to_string(), e(CExpr::IntLit(1)))])),
            &mut cs,
        );
        match sub.resolve_structure(ty) {
            Some(Structure::Record(fields, ext)) => {
                assert_eq!(ext, None);
                assert_eq!(
                    sub.resolve_structure(fields["x"]),
                    Some(builtin(&mut sub, "Int"))
                );
            }
            other => panic!("expected a closed Record, got {other:?}"),
        }
    }

    #[test]
    fn record_update_requires_the_updated_field_and_keeps_the_base_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("x".to_string(), int_ty);
        fields.insert("y".to_string(), bool_ty);
        let base_ty = sub.fresh_bound(Structure::Record(fields, None));
        scope.bind("base", base_ty);

        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::RecordUpdate(
                Box::new(e(CExpr::Var(Ref::Local("base".to_string())))),
                vec![("x".to_string(), e(CExpr::IntLit(1)))],
            )),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        // Result is exactly base's own type.
        assert_eq!(sub.find(ty), sub.find(base_ty));
    }

    #[test]
    fn field_access_requires_the_field_and_returns_its_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let int_ty = app0(&mut sub, "Int");
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("x".to_string(), int_ty);
        let base_ty = sub.fresh_bound(Structure::Record(fields, None));
        scope.bind("base", base_ty);

        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::FieldAccess(
                Box::new(e(CExpr::Var(Ref::Local("base".to_string())))),
                "x".to_string(),
            )),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "Int")));
    }

    #[test]
    fn field_access_on_a_closed_record_missing_the_field_fails() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let int_ty = app0(&mut sub, "Int");
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("x".to_string(), int_ty);
        let base_ty = sub.fresh_bound(Structure::Record(fields, None));
        scope.bind("base", base_ty);

        let mut cs = Vec::new();
        constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::FieldAccess(
                Box::new(e(CExpr::Var(Ref::Local("base".to_string())))),
                "y".to_string(),
            )),
            &mut cs,
        );
        let equal = cs.into_iter().find_map(|c| match c {
            Constraint::Equal {
                expected, actual, ..
            } => Some((expected, actual)),
            _ => None,
        });
        let (expected, actual) = equal.unwrap();
        assert!(crate::unify::unify(&mut sub, expected, actual).is_err());
    }

    #[test]
    fn annotated_passes_through_to_the_targets_type_unconstrained() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Annotated(vec![], Box::new(e(CExpr::IntLit(1))))),
            &mut cs,
        );
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "Int")));
    }
}
