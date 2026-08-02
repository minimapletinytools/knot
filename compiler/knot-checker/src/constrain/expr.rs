//! Constraint generation over `CExpr` (mirrors Elm's own
//! `Constrain/Expression.hs`, conceptually). Each function returns the
//! expression's own inferred type wrapped in a `TypedExpr` (Fix #3's Stage
//! A — see `ast.rs`'s own doc comment for the full elaboration design);
//! relating two types is always done by pushing a `Constraint::Equal`,
//! never by calling `unify` directly — see `constrain/mod.rs`'s module docs
//! on why generation and solving stay separate passes.
//!
//! `BinOp`/`Negate` are handled via a small, closed operator → interface
//! table (`constrain_binop`/spec §4.8, §6.2): what interface (if any) the
//! operator needs, and how its operand/result types relate. This only
//! emits `HasInstance` obligations — it doesn't check whether a resolvable
//! instance actually exists (that's `interface/table.rs`'s job, TM6);
//! generation only ever records *what* must hold, never checks it. The
//! exact same `(interface, TypeVarId)` pairs also get stashed directly on
//! the resulting `TExpr::BinOp`/`TExpr::Negate` node (`collect_has_instance`)
//! — see `ast.rs`'s own doc comment on why this is safe to do eagerly here,
//! unlike a `Lookup`/`LookupLocal`-resolved reference's own obligations.
//!
//! `Let` delegates to `constrain::decl::constrain_let_bindings` (TM4) for
//! its SCC-based dependency splitting.
//!
//! `Do` (spec §6.4/§8) desugars straight to `bind`/`pure` calls
//! (`desugar_do`) before ever reaching `constrain_expr` proper -- `do { x <-
//! e1; rest }` becomes `bind e1 (\x -> rest)`, `do { e1; rest }` (a bare
//! statement, its result discarded) becomes `bind e1 (\_ -> rest)`, exactly
//! as spec §8 already documents. Reusing `App`/`Lambda`'s own constraint
//! generation this way, rather than writing bespoke logic for `Do`, is what
//! `bind`'s real `Context f => f a -> (a -> f b) -> f b` scheme (`prelude.
//! rs`, Fix #2) is *for* -- the ordinary `Lookup`+instantiate path already
//! handles everything a hand-rolled version would have to duplicate.
//!
//! **Not yet handled**: `Annotated`'s annotation values aren't constrained
//! here at all (that's TM6's annotation-checking layer, plan §3.5), so this
//! pass only ever looks straight through to the target expression, and its
//! `TExpr::Annotated` carries the original, un-typechecked `CAnnotation`s
//! verbatim.

use knot_canonical::ast::{CDoStmt, CExpr, CPattern, Ref};
use knot_syntax::ast::expr::BinOp;
use knot_syntax::span::{Span, Spanned};

use crate::ast::{TExpr, Typed, TypedExpr};
use crate::constrain::pattern::constrain_pattern;
use crate::constrain::{Constraint, LocalBinding, LocalScope};
use crate::ty::Structure;
use crate::var::{Substitution, TypeVarId};

fn app0(sub: &mut Substitution, name: &str) -> TypeVarId {
    sub.fresh_bound(Structure::App(Ref::Builtin(name.to_string()), vec![]))
}

fn app1(sub: &mut Substitution, name: &str, arg: TypeVarId) -> TypeVarId {
    sub.fresh_bound(Structure::App(Ref::Builtin(name.to_string()), vec![arg]))
}

/// Every `Constraint::HasInstance` in `constraints[from..]`, as `(interface,
/// ty)` pairs — used right after a call that's known to push zero or more
/// `HasInstance`s (`constrain_binop`, `Negate`) to recover exactly which
/// ones, for stashing directly on the resulting `TExpr` node (see module
/// docs).
fn collect_has_instance(constraints: &[Constraint], from: usize) -> Vec<(String, TypeVarId)> {
    constraints[from..]
        .iter()
        .filter_map(|c| match c {
            Constraint::HasInstance { interface, ty, .. } => Some((interface.clone(), *ty)),
            _ => None,
        })
        .collect()
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

/// Resolves a `Var`/`Ctor` reference. `Ref::Local` covers three cases (see
/// `LocalBinding`): a lambda/case/do param or a group's own self/mutual
/// reference is `Monomorphic` — resolves immediately, no constraint needed;
/// a `let`-bound name that's been `promote_to_generalizable`d resolves via
/// its own `Constraint::LookupLocal` instead, exactly mirroring `Lookup`
/// but keyed by `TypeVarId`. A `Ref::TopLevel` that happens to already be in
/// `scope` is that same self/mutual-reference case (`constrain::decl`, TM4)
/// — always `Monomorphic` while it's there, since it hasn't been
/// generalized yet and won't be until this whole group finishes solving.
/// Everything else defers via `Lookup` (see `constrain/mod.rs`).
/// `Ref::Unresolved` is neither — `knot-canonical` already recorded the
/// real error for it, so this returns a bare, unconstrained fresh variable
/// rather than compounding that error with a confusing downstream one.
fn constrain_name_ref(
    sub: &mut Substitution,
    scope: &LocalScope,
    reference: &Ref,
    span: knot_syntax::span::Span,
    constraints: &mut Vec<Constraint>,
) -> TypeVarId {
    match reference {
        Ref::Local(name) => match scope.lookup(name) {
            LocalBinding::Monomorphic(ty) => ty,
            LocalBinding::Generalizable(key) => {
                let ty = sub.fresh_unbound();
                constraints.push(Constraint::LookupLocal {
                    span,
                    key,
                    expected: ty,
                });
                ty
            }
        },
        Ref::Unresolved(_) => sub.fresh_unbound(),
        Ref::TopLevel(name) if scope.try_lookup(name).is_some() => scope.lookup(name).header_ty(),
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
) -> TypedExpr {
    let span = expr.span;
    let wrap = |ty, node| Typed { span, ty, node };
    match &expr.node {
        CExpr::IntLit(n) => wrap(app0(sub, "Int"), TExpr::IntLit(*n)),
        CExpr::FloatLit(f) => wrap(app0(sub, "Float"), TExpr::FloatLit(*f)),
        CExpr::StringLit(s) => wrap(app0(sub, "String"), TExpr::StringLit(s.clone())),
        CExpr::Unit => wrap(sub.fresh_bound(Structure::Unit), TExpr::Unit),
        // A hole's own type is unconstrained by design (it could be
        // anything) -- that a hole must always be a compile error (spec
        // §12.1) is a separate, later diagnostic concern (recording and
        // always rejecting every hole span seen), not a typing constraint,
        // so it isn't handled here.
        CExpr::Hole => wrap(sub.fresh_unbound(), TExpr::Hole),
        CExpr::Var(r) => {
            let ty = constrain_name_ref(sub, scope, r, span, constraints);
            wrap(ty, TExpr::Var(r.clone()))
        }
        CExpr::Ctor(r) => {
            let ty = constrain_name_ref(sub, scope, r, span, constraints);
            wrap(ty, TExpr::Ctor(r.clone()))
        }
        CExpr::Lambda(params, body) => {
            scope.push();
            let typed_params: Vec<crate::ast::TypedPattern> = params
                .iter()
                .map(|p| constrain_pattern(sub, scope, p, constraints))
                .collect();
            let typed_body = constrain_expr(sub, scope, body, constraints);
            scope.pop();
            let ty = typed_params.iter().rev().fold(typed_body.ty, |acc, p| {
                sub.fresh_bound(Structure::Fn(p.ty, acc))
            });
            wrap(ty, TExpr::Lambda(typed_params, Box::new(typed_body)))
        }
        CExpr::App(f, arg) => {
            let typed_f = constrain_expr(sub, scope, f, constraints);
            let typed_arg = constrain_expr(sub, scope, arg, constraints);
            let result_ty = sub.fresh_unbound();
            let expected_fn_ty = sub.fresh_bound(Structure::Fn(typed_arg.ty, result_ty));
            constraints.push(Constraint::Equal {
                span,
                expected: typed_f.ty,
                actual: expected_fn_ty,
            });
            wrap(
                result_ty,
                TExpr::App(Box::new(typed_f), Box::new(typed_arg)),
            )
        }
        CExpr::If(cond, then_branch, else_branch) => {
            let typed_cond = constrain_expr(sub, scope, cond, constraints);
            let bool_ty = app0(sub, "Bool");
            constraints.push(Constraint::Equal {
                span,
                expected: typed_cond.ty,
                actual: bool_ty,
            });
            let typed_then = constrain_expr(sub, scope, then_branch, constraints);
            let typed_else = constrain_expr(sub, scope, else_branch, constraints);
            constraints.push(Constraint::Equal {
                span,
                expected: typed_then.ty,
                actual: typed_else.ty,
            });
            let ty = typed_then.ty;
            wrap(
                ty,
                TExpr::If(
                    Box::new(typed_cond),
                    Box::new(typed_then),
                    Box::new(typed_else),
                ),
            )
        }
        CExpr::Case(scrutinee, arms) => {
            let typed_scrutinee = constrain_expr(sub, scope, scrutinee, constraints);
            let result_ty = sub.fresh_unbound();
            let mut typed_arms = Vec::new();
            for (pattern, body) in arms {
                scope.push();
                let typed_pattern = constrain_pattern(sub, scope, pattern, constraints);
                constraints.push(Constraint::Equal {
                    span,
                    expected: typed_scrutinee.ty,
                    actual: typed_pattern.ty,
                });
                let typed_body = constrain_expr(sub, scope, body, constraints);
                constraints.push(Constraint::Equal {
                    span,
                    expected: result_ty,
                    actual: typed_body.ty,
                });
                scope.pop();
                typed_arms.push((typed_pattern, typed_body));
            }
            wrap(
                result_ty,
                TExpr::Case(Box::new(typed_scrutinee), typed_arms),
            )
        }
        CExpr::List(elems) => {
            let elem_ty = sub.fresh_unbound();
            let mut typed_elems = Vec::new();
            for e in elems {
                let typed_e = constrain_expr(sub, scope, e, constraints);
                constraints.push(Constraint::Equal {
                    span,
                    expected: elem_ty,
                    actual: typed_e.ty,
                });
                typed_elems.push(typed_e);
            }
            wrap(app1(sub, "List", elem_ty), TExpr::List(typed_elems))
        }
        // Arity <= 3 is already enforced post-parse (`knot-syntax::validate`)
        // -- not re-checked here.
        CExpr::Tuple(elems) => {
            let typed_elems: Vec<TypedExpr> = elems
                .iter()
                .map(|e| constrain_expr(sub, scope, e, constraints))
                .collect();
            let elem_tys = typed_elems.iter().map(|e| e.ty).collect();
            wrap(
                sub.fresh_bound(Structure::Tuple(elem_tys)),
                TExpr::Tuple(typed_elems),
            )
        }
        // A record literal is always closed -- exactly these fields, spec
        // §4.7 -- unlike `FieldAccess`/`RecordUpdate` below, which only ever
        // need an *open* row (the value being accessed/updated might have
        // more fields than the expression cares about).
        CExpr::Record(fields) => {
            let typed_fields: Vec<(String, TypedExpr)> = fields
                .iter()
                .map(|(name, e)| (name.clone(), constrain_expr(sub, scope, e, constraints)))
                .collect();
            let field_tys = typed_fields
                .iter()
                .map(|(n, e)| (n.clone(), e.ty))
                .collect();
            wrap(
                sub.fresh_bound(Structure::Record(field_tys, None)),
                TExpr::Record(typed_fields),
            )
        }
        CExpr::RecordUpdate(base, updates) => {
            let typed_base = constrain_expr(sub, scope, base, constraints);
            let typed_updates: Vec<(String, TypedExpr)> = updates
                .iter()
                .map(|(name, e)| (name.clone(), constrain_expr(sub, scope, e, constraints)))
                .collect();
            // `base` only needs to have *at least* the updated fields, at
            // their new types -- the row-polymorphism machinery in
            // `unify::unify_record` figures out the rest, whatever it is.
            let update_tys = typed_updates
                .iter()
                .map(|(n, e)| (n.clone(), e.ty))
                .collect();
            let rest = sub.fresh_unbound();
            let required = sub.fresh_bound(Structure::Record(update_tys, Some(rest)));
            constraints.push(Constraint::Equal {
                span,
                expected: typed_base.ty,
                actual: required,
            });
            // The result has exactly `base`'s own shape -- an update can't
            // add or remove fields, only change values.
            let ty = typed_base.ty;
            wrap(ty, TExpr::RecordUpdate(Box::new(typed_base), typed_updates))
        }
        CExpr::FieldAccess(base, field) => {
            let typed_base = constrain_expr(sub, scope, base, constraints);
            let field_ty = sub.fresh_unbound();
            let rest = sub.fresh_unbound();
            let mut required_fields = std::collections::BTreeMap::new();
            required_fields.insert(field.clone(), field_ty);
            let required = sub.fresh_bound(Structure::Record(required_fields, Some(rest)));
            constraints.push(Constraint::Equal {
                span,
                expected: typed_base.ty,
                actual: required,
            });
            wrap(
                field_ty,
                TExpr::FieldAccess(Box::new(typed_base), field.clone()),
            )
        }
        // Annotation *values* aren't constrained here at all -- see this
        // module's doc comment. Just look straight through to the target,
        // keeping the original CAnnotations unelaborated.
        CExpr::Annotated(annotations, target) => {
            let typed_target = constrain_expr(sub, scope, target, constraints);
            let ty = typed_target.ty;
            wrap(
                ty,
                TExpr::Annotated(annotations.clone(), Box::new(typed_target)),
            )
        }
        CExpr::BinOp(op, l, r) => {
            let typed_l = constrain_expr(sub, scope, l, constraints);
            let typed_r = constrain_expr(sub, scope, r, constraints);
            let before = constraints.len();
            let ty = constrain_binop(sub, *op, span, typed_l.ty, typed_r.ty, constraints);
            let obligations = collect_has_instance(constraints, before);
            wrap(
                ty,
                TExpr::BinOp(*op, Box::new(typed_l), Box::new(typed_r), obligations),
            )
        }
        CExpr::Negate(e) => {
            let typed_e = constrain_expr(sub, scope, e, constraints);
            constraints.push(Constraint::HasInstance {
                span,
                interface: "Num".to_string(),
                ty: typed_e.ty,
            });
            let ty = typed_e.ty;
            wrap(
                ty,
                TExpr::Negate(Box::new(typed_e), vec![("Num".to_string(), ty)]),
            )
        }
        CExpr::Let(bindings, body) => {
            let (typed_let, let_con) = crate::constrain::decl::constrain_let_bindings(
                sub,
                scope,
                bindings,
                |sub, scope, cs| constrain_expr(sub, scope, body, cs),
            );
            constraints.push(let_con);
            typed_let
        }
        CExpr::Do(stmts, final_expr) => {
            let desugared = desugar_do(stmts, final_expr);
            constrain_expr(sub, scope, &desugared, constraints)
        }
    }
}

/// `do { stmts...; final_expr }` -> nested `bind`/lambda calls, right-
/// associatively -- see this module's own doc comment. `final_expr` is the
/// base case, passed through completely unchanged (spec §8's own example
/// ends in an explicit `pure (x + y)`; nothing here adds an implicit one).
fn desugar_do(stmts: &[CDoStmt], final_expr: &Spanned<CExpr>) -> Spanned<CExpr> {
    let Some((stmt, rest)) = stmts.split_first() else {
        return final_expr.clone();
    };
    let (pattern, action) = match stmt {
        CDoStmt::Bind(p, e) => (p.clone(), e.clone()),
        CDoStmt::Expr(e) => (Spanned::new(e.span, CPattern::Wildcard(None)), e.clone()),
    };
    let span = action.span;
    let rest_expr = desugar_do(rest, final_expr);
    let continuation = Spanned::new(span, CExpr::Lambda(vec![pattern], Box::new(rest_expr)));
    let bind_ref = Spanned::new(span, CExpr::Var(Ref::Builtin("bind".to_string())));
    let bind_applied = Spanned::new(span, CExpr::App(Box::new(bind_ref), Box::new(action)));
    Spanned::new(
        span,
        CExpr::App(Box::new(bind_applied), Box::new(continuation)),
    )
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
    /// Not the real `solve.rs` -- `Lookup`/`LookupLocal`/`HasInstance` aren't
    /// resolvable without a real environment, which doesn't exist here, so
    /// tests that actually need one of those resolved (e.g. real
    /// let-polymorphic reuse) go through `crate::solve::solve` directly
    /// instead (see `later_let_binding_can_reference_an_earlier_sibling`);
    /// everything else here only checks the constraint *shape*.
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
                Constraint::Given { body, .. } => solve_equalities(sub, vec![*body]),
                Constraint::True
                | Constraint::HasInstance { .. }
                | Constraint::Lookup { .. }
                | Constraint::LookupLocal { .. } => {}
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
        let int_typed = constrain_expr(&mut sub, &mut scope, &e(CExpr::IntLit(1)), &mut cs);
        let float_typed = constrain_expr(&mut sub, &mut scope, &e(CExpr::FloatLit(1.0)), &mut cs);
        let str_typed = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::StringLit("x".to_string())),
            &mut cs,
        );
        let unit_typed = constrain_expr(&mut sub, &mut scope, &e(CExpr::Unit), &mut cs);
        assert_eq!(
            sub.resolve_structure(int_typed.ty),
            Some(builtin(&mut sub, "Int"))
        );
        assert_eq!(
            sub.resolve_structure(float_typed.ty),
            Some(builtin(&mut sub, "Float"))
        );
        assert_eq!(
            sub.resolve_structure(str_typed.ty),
            Some(builtin(&mut sub, "String"))
        );
        assert_eq!(sub.resolve_structure(unit_typed.ty), Some(Structure::Unit));
        assert_eq!(int_typed.node, TExpr::IntLit(1));
        assert_eq!(unit_typed.node, TExpr::Unit);
    }

    #[test]
    fn hole_is_a_bare_unconstrained_fresh_variable() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let typed = constrain_expr(&mut sub, &mut scope, &e(CExpr::Hole), &mut cs);
        assert!(cs.is_empty());
        assert!(sub.resolve_structure(typed.ty).is_none());
    }

    #[test]
    fn local_var_resolves_immediately_with_no_constraint() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let bound = sub.fresh_unbound();
        scope.bind("x", bound);
        let mut cs = Vec::new();
        let typed = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Var(Ref::Local("x".to_string()))),
            &mut cs,
        );
        assert!(cs.is_empty());
        assert_eq!(typed.ty, bound);
    }

    #[test]
    fn non_local_var_defers_via_a_lookup_constraint() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let typed = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Var(Ref::Builtin("compare".to_string()))),
            &mut cs,
        );
        assert_eq!(cs.len(), 1);
        assert!(matches!(
            &cs[0],
            Constraint::Lookup { reference, expected, .. }
                if *reference == Ref::Builtin("compare".to_string()) && *expected == typed.ty
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
        let typed = constrain_expr(
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
        match sub.resolve_structure(typed.ty) {
            Some(Structure::Fn(x_ty, rest)) => match sub.resolve_structure(rest) {
                Some(Structure::Fn(_y_ty, body_ty)) => {
                    assert_eq!(sub.find(x_ty), sub.find(body_ty));
                }
                other => panic!("expected a second Fn arrow, got {other:?}"),
            },
            other => panic!("expected a Fn shape, got {other:?}"),
        }
        assert!(matches!(typed.node, TExpr::Lambda(ref params, _) if params.len() == 2));
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
        let typed = constrain_expr(
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
            sub.resolve_structure(typed.ty),
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
        let typed = constrain_expr(
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
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "Int"))
        );
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
        let typed = constrain_expr(
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
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "String"))
        );
        assert!(matches!(typed.node, TExpr::Case(_, ref arms) if arms.len() == 2));
    }

    #[test]
    fn list_unifies_every_element_to_the_same_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let typed = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::List(vec![e(CExpr::IntLit(1)), e(CExpr::IntLit(2))])),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        match sub.resolve_structure(typed.ty) {
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
        let typed = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Tuple(vec![
                e(CExpr::IntLit(1)),
                e(CExpr::StringLit("x".to_string())),
            ])),
            &mut cs,
        );
        match sub.resolve_structure(typed.ty) {
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
        let typed = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Record(vec![("x".to_string(), e(CExpr::IntLit(1)))])),
            &mut cs,
        );
        match sub.resolve_structure(typed.ty) {
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
        let typed = constrain_expr(
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
        assert_eq!(sub.find(typed.ty), sub.find(base_ty));
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
        let typed = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::FieldAccess(
                Box::new(e(CExpr::Var(Ref::Local("base".to_string())))),
                "x".to_string(),
            )),
            &mut cs,
        );
        solve_equalities(&mut sub, cs);
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "Int"))
        );
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
        let typed = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Annotated(vec![], Box::new(e(CExpr::IntLit(1))))),
            &mut cs,
        );
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "Int"))
        );
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
        let typed = constrain_expr(
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
        // The TExpr node itself should also carry the same obligation.
        match &typed.node {
            TExpr::BinOp(BinOp::Add, _, _, obligations) => {
                assert_eq!(obligations, &vec![("Num".to_string(), num_ty)]);
            }
            other => panic!("expected a TExpr::BinOp(Add, ...), got {other:?}"),
        }
        solve_equalities(&mut sub, cs);
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "Int"))
        );
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
        let typed = constrain_expr(
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
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "Bool"))
        );
    }

    #[test]
    fn ordering_comparison_requires_ord_and_returns_bool() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let typed = constrain_expr(
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
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "Bool"))
        );
    }

    #[test]
    fn cons_binop_builds_a_list_of_the_head_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let typed = constrain_expr(
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
        match sub.resolve_structure(typed.ty) {
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
        let typed = constrain_expr(
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
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "Bool"))
        );
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
        let typed = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::BinOp(
                BinOp::Pow,
                Box::new(e(CExpr::Var(Ref::Local("base".to_string())))),
                Box::new(e(CExpr::Var(Ref::Local("exp".to_string())))),
            )),
            &mut cs,
        );
        assert_eq!(typed.ty, base_ty); // base and exponent may differ -- result is base's type
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
        let typed = constrain_expr(
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
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "Bool"))
        );
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
        let typed = constrain_expr(
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
        match sub.resolve_structure(typed.ty) {
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
        let typed = constrain_expr(
            &mut sub,
            &mut scope,
            &e(CExpr::Negate(Box::new(e(CExpr::IntLit(1))))),
            &mut cs,
        );
        let num_ty = has_instance(&cs, "Num").expect("Negate should require Num");
        assert_eq!(num_ty, typed.ty);
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "Int"))
        );
        assert!(matches!(
            &typed.node,
            TExpr::Negate(_, obligations) if obligations == &vec![("Num".to_string(), num_ty)]
        ));
    }

    #[test]
    fn let_expression_delegates_to_constrain_decl_and_type_checks_end_to_end() {
        // let x = 1 in x + 1
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let typed = constrain_expr(
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
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "Int"))
        );
        assert!(matches!(typed.node, TExpr::Let(ref bs, _) if bs.len() == 1));
    }

    #[test]
    fn later_let_binding_can_reference_an_earlier_sibling() {
        // let x = 1; y = x + 1 in y  -- regression test: an earlier bug had
        // each SCC group's own scope pop before anything after it (later
        // siblings, or the final body) got a chance to see those names.
        // Uses the real `solve::solve`, not the shallow `solve_equalities`
        // above: Fix #1 gives `x` real let-polymorphism, so `y`'s reference
        // to it goes through `Constraint::LookupLocal`, resolved against
        // `solve.rs`'s own `local_env` -- which the test-only solver above
        // doesn't have (see its own doc comment).
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        let mut cs = Vec::new();
        let typed = constrain_expr(
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
        // Not checking `pending` here: `x + 1`'s `Num` obligation lands on a
        // concrete, non-generalized `Int` (nothing left to quantify over in
        // a bare `x = 1`), so it stays a real pending obligation -- checking
        // it against an instance table is `interface::instance`'s job
        // (TM6), not relevant to what this regression test is actually
        // about.
        let (_pending, errors) = crate::solve::solve(
            &mut sub,
            &mut crate::solve::SchemeEnv::new(),
            &Constraint::And(cs),
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            sub.resolve_structure(typed.ty),
            Some(builtin(&mut sub, "Int"))
        );
    }
}
