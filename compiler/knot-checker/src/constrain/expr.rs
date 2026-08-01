//! Constraint generation over `CExpr` (mirrors Elm's own
//! `Constrain/Expression.hs`, conceptually). Each function returns the
//! expression's own inferred type; relating two types is always done by
//! pushing a `Constraint::Equal`, never by calling `unify` directly — see
//! `constrain/mod.rs`'s module docs on why generation and solving stay
//! separate passes.
//!
//! `BinOp`/`Negate` are handled via a small, closed operator → interface
//! table (`constrain_binop`/spec §4.8, §6.2): what interface (if any) the
//! operator needs, and how its operand/result types relate. This only
//! emits `HasInstance` obligations — it doesn't check whether a resolvable
//! instance actually exists (that's `interface/table.rs`'s job, TM6);
//! generation only ever records *what* must hold, never checks it.
//!
//! `Let` delegates to `constrain::decl::constrain_let_bindings` (TM4) for
//! its SCC-based dependency splitting.
//!
//! **Not yet handled**: `Do` needs the Context interface's `pure`/`bind`
//! dictionary story (spec §6.4). `Annotated`, by contrast, *is* handled —
//! deliberately shallow: its annotation values aren't constrained here at
//! all (that's TM6's annotation-checking layer, plan §3.5), so this pass
//! only ever looks straight through to the target
//! expression.

use knot_canonical::ast::{CExpr, Ref};
use knot_syntax::ast::expr::BinOp;
use knot_syntax::span::{Span, Spanned};

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

/// `left`/`right` must be the same type, and that type needs `interface`.
/// Covers every operator whose signature is `Interface a => a -> a -> a`
/// (`Num`'s `+`/`-`/`*`, `Fractional`'s `/`, `Integral`'s `div`/`mod`,
/// `Semigroup`'s `<>`) as well as the comparison operators, whose only
/// difference is a `Bool` result instead of `a` — see call sites.
fn same_type_with_instance(
    constraints: &mut Vec<Constraint>,
    span: Span,
    interface: &str,
    left: TypeVarId,
    right: TypeVarId,
) {
    constraints.push(Constraint::Equal {
        span,
        expected: left,
        actual: right,
    });
    constraints.push(Constraint::HasInstance {
        span,
        interface: interface.to_string(),
        ty: left,
    });
}

fn constrain_binop(
    sub: &mut Substitution,
    op: BinOp,
    span: Span,
    left: TypeVarId,
    right: TypeVarId,
    constraints: &mut Vec<Constraint>,
) -> TypeVarId {
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul => {
            same_type_with_instance(constraints, span, "Num", left, right);
            left
        }
        BinOp::Div => {
            same_type_with_instance(constraints, span, "Fractional", left, right);
            left
        }
        BinOp::IntDiv | BinOp::Mod => {
            same_type_with_instance(constraints, span, "Integral", left, right);
            left
        }
        BinOp::Append => {
            same_type_with_instance(constraints, span, "Semigroup", left, right);
            left
        }
        BinOp::Eq | BinOp::Neq => {
            same_type_with_instance(constraints, span, "Eq", left, right);
            app0(sub, "Bool")
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            same_type_with_instance(constraints, span, "Ord", left, right);
            app0(sub, "Bool")
        }
        // `a -> List a -> List a` -- no interface, an ordinary built-in.
        BinOp::Cons => {
            let list_ty = app1(sub, "List", left);
            constraints.push(Constraint::Equal {
                span,
                expected: right,
                actual: list_ty,
            });
            list_ty
        }
        // `Bool -> Bool -> Bool` -- concrete, no interface (spec §4.8's
        // "Boolean Operators" note: only `not` is a named function; `&&`/`||`
        // are built-in operators over the one concrete type).
        BinOp::And | BinOp::Or => {
            let bool_ty = app0(sub, "Bool");
            constraints.push(Constraint::Equal {
                span,
                expected: left,
                actual: bool_ty,
            });
            constraints.push(Constraint::Equal {
                span,
                expected: right,
                actual: bool_ty,
            });
            bool_ty
        }
        // `(Num a, Integral b) => a -> b -> a` (spec §6.2's "Exponentiation")
        // -- the one operator whose two operands are constrained by two
        // *different* interfaces on two different type variables, rather
        // than needing to be the same type at all.
        BinOp::Pow => {
            constraints.push(Constraint::HasInstance {
                span,
                interface: "Num".to_string(),
                ty: left,
            });
            constraints.push(Constraint::HasInstance {
                span,
                interface: "Integral".to_string(),
                ty: right,
            });
            left
        }
        // `a |> f` === `f a`.
        BinOp::Pipe => {
            let result = sub.fresh_unbound();
            let expected_fn_ty = sub.fresh_bound(Structure::Fn(left, result));
            constraints.push(Constraint::Equal {
                span,
                expected: right,
                actual: expected_fn_ty,
            });
            result
        }
        // `f >> g :: a -> c` where `f :: a -> b`, `g :: b -> c`.
        BinOp::Compose => {
            let a = sub.fresh_unbound();
            let b = sub.fresh_unbound();
            let c = sub.fresh_unbound();
            let f_ty = sub.fresh_bound(Structure::Fn(a, b));
            let g_ty = sub.fresh_bound(Structure::Fn(b, c));
            constraints.push(Constraint::Equal {
                span,
                expected: left,
                actual: f_ty,
            });
            constraints.push(Constraint::Equal {
                span,
                expected: right,
                actual: g_ty,
            });
            sub.fresh_bound(Structure::Fn(a, c))
        }
    }
}

/// Resolves a `Var`/`Ctor` reference: immediate for `Ref::Local` (never
/// generalized, so no constraint needed at all), deferred via `Lookup` for
/// everything else (see `constrain/mod.rs`) -- *except* a `Ref::TopLevel`
/// that happens to already be in `scope`, which means it's a self/mutual
/// reference to a binding in the very group currently being checked
/// (`constrain::decl`, TM4): still monomorphic for now, exactly like a
/// local, since it hasn't been generalized yet and won't be until this
/// whole group finishes solving. `Ref::Unresolved` is neither — `knot-
/// canonical` already recorded the real error for it, so this returns a
/// bare, unconstrained fresh variable rather than compounding that error
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
        Ref::TopLevel(name) if scope.try_lookup(name).is_some() => scope.lookup(name),
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
            param_tys.into_iter().rev().fold(body_ty, |acc, param_ty| {
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
        CExpr::BinOp(op, l, r) => {
            let left = constrain_expr(sub, scope, l, constraints);
            let right = constrain_expr(sub, scope, r, constraints);
            constrain_binop(sub, *op, span, left, right, constraints)
        }
        CExpr::Negate(e) => {
            let e_ty = constrain_expr(sub, scope, e, constraints);
            constraints.push(Constraint::HasInstance {
                span,
                interface: "Num".to_string(),
                ty: e_ty,
            });
            e_ty
        }
        CExpr::Let(bindings, body) => {
            let (result_ty, let_con) = crate::constrain::decl::constrain_let_bindings(
                sub,
                scope,
                bindings,
                |sub, scope, cs| constrain_expr(sub, scope, body, cs),
            );
            constraints.push(let_con);
            result_ty
        }
        CExpr::Do(..) => {
            todo!("Do needs the Context interface's pure/bind dictionary story (spec §6.4)")
        }
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
                // Not the real solve.rs (TM5): no generalization, no scheme
                // environment for Lookup -- just enough to prove a `let`
                // with no self-referencing polymorphic reuse type-checks
                // end-to-end. `header_con`'s self/mutual references already
                // resolved monomorphically at generation time (constrain::
                // decl), so walking straight into both halves is sound here.
                Constraint::Let {
                    header_con,
                    body_con,
                    ..
                } => {
                    solve_equalities(sub, vec![*header_con]);
                    solve_equalities(sub, vec![*body_con]);
                }
                Constraint::True | Constraint::HasInstance { .. } | Constraint::Lookup { .. } => {}
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

    fn has_instance(cs: &[Constraint], interface: &str) -> Option<TypeVarId> {
        cs.iter().find_map(|c| match c {
            Constraint::HasInstance {
                interface: i, ty, ..
            } if i == interface => Some(*ty),
            _ => None,
        })
    }

    #[test]
    fn add_unifies_operands_and_requires_num() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::Add,
                Box::new(e(CExpr::IntLit(1))),
                Box::new(e(CExpr::IntLit(2))),
            )),
            &mut cs,
        );
        let num_ty = has_instance(&cs, "Num").expect("Add should require Num");
        solve_equalities(&mut sub, cs);
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "Int")));
        assert_eq!(
            sub.resolve_structure(num_ty),
            Some(builtin(&mut sub, "Int"))
        );
    }

    #[test]
    fn mismatched_add_operands_fail_to_solve() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::Add,
                Box::new(e(CExpr::IntLit(1))),
                Box::new(e(CExpr::StringLit("x".to_string()))),
            )),
            &mut cs,
        );
        let equal = cs
            .into_iter()
            .find_map(|c| match c {
                Constraint::Equal {
                    expected, actual, ..
                } => Some((expected, actual)),
                _ => None,
            })
            .unwrap();
        assert!(crate::unify::unify(&mut sub, equal.0, equal.1).is_err());
    }

    #[test]
    fn div_requires_fractional() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::Div,
                Box::new(e(CExpr::FloatLit(1.0))),
                Box::new(e(CExpr::FloatLit(2.0))),
            )),
            &mut cs,
        );
        assert!(has_instance(&cs, "Fractional").is_some());
    }

    #[test]
    fn int_div_and_mod_require_integral() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::IntDiv,
                Box::new(e(CExpr::IntLit(7))),
                Box::new(e(CExpr::IntLit(2))),
            )),
            &mut cs,
        );
        assert!(has_instance(&cs, "Integral").is_some());
    }

    #[test]
    fn append_requires_semigroup() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::Append,
                Box::new(e(CExpr::StringLit("a".to_string()))),
                Box::new(e(CExpr::StringLit("b".to_string()))),
            )),
            &mut cs,
        );
        assert!(has_instance(&cs, "Semigroup").is_some());
    }

    #[test]
    fn equality_requires_eq_and_returns_bool() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::Eq,
                Box::new(e(CExpr::IntLit(1))),
                Box::new(e(CExpr::IntLit(2))),
            )),
            &mut cs,
        );
        assert!(has_instance(&cs, "Eq").is_some());
        solve_equalities(&mut sub, cs);
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "Bool")));
    }

    #[test]
    fn ordering_comparison_requires_ord_and_returns_bool() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::Lt,
                Box::new(e(CExpr::IntLit(1))),
                Box::new(e(CExpr::IntLit(2))),
            )),
            &mut cs,
        );
        assert!(has_instance(&cs, "Ord").is_some());
        solve_equalities(&mut sub, cs);
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "Bool")));
    }

    #[test]
    fn cons_binop_builds_a_list_of_the_head_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::Cons,
                Box::new(e(CExpr::IntLit(1))),
                Box::new(e(CExpr::List(vec![]))),
            )),
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
    fn and_or_require_boolean_operands() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::And,
                Box::new(e(CExpr::Var(Ref::Builtin("bogus1".to_string())))),
                Box::new(e(CExpr::Var(Ref::Builtin("bogus2".to_string())))),
            )),
            &mut cs,
        );
        // Both operands are deferred Lookups here (not concrete literals),
        // so just check the two Equal-to-Bool constraints got generated and
        // the result itself is already Bool without needing to solve.
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "Bool")));
        let equal_count = cs
            .iter()
            .filter(|c| matches!(c, Constraint::Equal { .. }))
            .count();
        assert_eq!(equal_count, 2);
    }

    #[test]
    fn pow_constrains_base_with_num_and_exponent_with_integral_independently() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let base_ty = app0(&mut sub, "Float");
        scope.bind("base", base_ty);
        let exp_ty = app0(&mut sub, "Int");
        scope.bind("exp", exp_ty);

        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::Pow,
                Box::new(e(CExpr::Var(Ref::Local("base".to_string())))),
                Box::new(e(CExpr::Var(Ref::Local("exp".to_string())))),
            )),
            &mut cs,
        );
        assert_eq!(ty, base_ty); // base and exponent may differ -- result is base's type
        let num_ty = has_instance(&cs, "Num").unwrap();
        let integral_ty = has_instance(&cs, "Integral").unwrap();
        assert_eq!(
            sub.resolve_structure(num_ty),
            Some(builtin(&mut sub, "Float"))
        );
        assert_eq!(
            sub.resolve_structure(integral_ty),
            Some(builtin(&mut sub, "Int"))
        );
    }

    #[test]
    fn pipe_applies_the_right_hand_function_to_the_left_hand_value() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let f_ty = sub.fresh_bound(Structure::Fn(int_ty, bool_ty));
        scope.bind("x", int_ty);
        scope.bind("f", f_ty);

        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::Pipe,
                Box::new(e(CExpr::Var(Ref::Local("x".to_string())))),
                Box::new(e(CExpr::Var(Ref::Local("f".to_string())))),
            )),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "Bool")));
    }

    #[test]
    fn compose_builds_the_end_to_end_function_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let string_ty = app0(&mut sub, "String");
        let f_ty = sub.fresh_bound(Structure::Fn(int_ty, bool_ty));
        let g_ty = sub.fresh_bound(Structure::Fn(bool_ty, string_ty));
        scope.bind("f", f_ty);
        scope.bind("g", g_ty);

        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::Compose,
                Box::new(e(CExpr::Var(Ref::Local("f".to_string())))),
                Box::new(e(CExpr::Var(Ref::Local("g".to_string())))),
            )),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        match sub.resolve_structure(ty) {
            Some(Structure::Fn(a, c)) => {
                assert_eq!(sub.resolve_structure(a), Some(builtin(&mut sub, "Int")));
                assert_eq!(sub.resolve_structure(c), Some(builtin(&mut sub, "String")));
            }
            other => panic!("expected a Fn Int String shape, got {other:?}"),
        }
    }

    #[test]
    fn negate_requires_num_and_preserves_the_operands_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Negate(Box::new(e(CExpr::IntLit(1))))),
            &mut cs,
        );
        let num_ty = has_instance(&cs, "Num").expect("Negate should require Num");
        assert_eq!(num_ty, ty);
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "Int")));
    }

    #[test]
    fn let_expression_delegates_to_constrain_decl_and_type_checks_end_to_end() {
        // let x = 1 in x + 1
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Let(
                vec![(p(CPattern::Var("x".to_string())), e(CExpr::IntLit(1)))],
                Box::new(e(CExpr::BinOp(
                    BinOp::Add,
                    Box::new(e(CExpr::Var(Ref::Local("x".to_string())))),
                    Box::new(e(CExpr::IntLit(1))),
                ))),
            )),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "Int")));
    }

    #[test]
    fn later_let_binding_can_reference_an_earlier_sibling() {
        // let x = 1; y = x + 1 in y  -- regression test: an earlier bug had
        // each SCC group's own scope pop before anything after it (later
        // siblings, or the final body) got a chance to see those names.
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let ty = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Let(
                vec![
                    (p(CPattern::Var("x".to_string())), e(CExpr::IntLit(1))),
                    (
                        p(CPattern::Var("y".to_string())),
                        e(CExpr::BinOp(
                            BinOp::Add,
                            Box::new(e(CExpr::Var(Ref::Local("x".to_string())))),
                            Box::new(e(CExpr::IntLit(1))),
                        )),
                    ),
                ],
                Box::new(e(CExpr::Var(Ref::Local("y".to_string())))),
            )),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        assert_eq!(sub.resolve_structure(ty), Some(builtin(&mut sub, "Int")));
    }
}
