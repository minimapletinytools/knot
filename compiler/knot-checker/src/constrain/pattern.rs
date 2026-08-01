//! Constraint generation over `CPattern` (mirrors Elm's own
//! `Constrain/Pattern.hs`, conceptually). Each function returns the
//! pattern's own inferred type — relating that to whatever it's actually
//! being matched against (a scrutinee, a signature's parameter type, ...) is
//! the caller's job, via an ordinary `Constraint::Equal`, not this module's.
//!
//! Binds pattern variables into `scope` as a side effect, exactly the way
//! `knot_canonical::resolve::pattern` binds *names* into its own `Env` — this
//! is that same shape, one layer up, mapping to a `TypeVarId` instead of just
//! recording that the name exists (name-level duplicate/arity checks already
//! happened during canonicalization, so none of that is re-checked here; see
//! `CPattern::Ctor`'s own doc comment on that trust boundary).

use knot_canonical::ast::CPattern;
use knot_syntax::ast::pattern::PatternLiteral;
use knot_syntax::span::Spanned;

use crate::constrain::{Constraint, LocalScope};
use crate::ty::Structure;
use crate::var::{Substitution, TypeVarId};

fn app0(sub: &mut Substitution, name: &str) -> TypeVarId {
    sub.fresh_bound(Structure::App(
        knot_canonical::ast::Ref::Builtin(name.to_string()),
        vec![],
    ))
}

fn app1(sub: &mut Substitution, name: &str, arg: TypeVarId) -> TypeVarId {
    sub.fresh_bound(Structure::App(
        knot_canonical::ast::Ref::Builtin(name.to_string()),
        vec![arg],
    ))
}

pub fn constrain_pattern(
    sub: &mut Substitution,
    scope: &mut LocalScope,
    pattern: &Spanned<CPattern>,
    constraints: &mut Vec<Constraint>,
) -> TypeVarId {
    let span = pattern.span;
    match &pattern.node {
        // A named wildcard (`_name`) is a discard exactly like `_` -- spec
        // §12.2/§12.3: the name is a mnemonic for the reader only, never
        // compiler-checked, so it's never bound into scope.
        CPattern::Wildcard(_) => sub.fresh_unbound(),
        CPattern::Var(name) => {
            let ty = sub.fresh_unbound();
            scope.bind(name, ty);
            ty
        }
        CPattern::Literal(PatternLiteral::Int(_)) => app0(sub, "Int"),
        CPattern::Literal(PatternLiteral::Str(_)) => app0(sub, "String"),
        // Arity against the constructor's declared field count is already
        // checked during canonicalization (see `CPattern::Ctor`'s own doc
        // comment in knot-canonical's `ast.rs`) -- `subpatterns.len()` is
        // trusted as correct here, no re-check needed.
        CPattern::Ctor(reference, subpatterns) => {
            let field_tys: Vec<TypeVarId> = subpatterns
                .iter()
                .map(|p| constrain_pattern(sub, scope, p, constraints))
                .collect();
            let result = sub.fresh_unbound();
            let curried = field_tys.into_iter().rev().fold(result, |acc, field_ty| {
                sub.fresh_bound(Structure::Fn(field_ty, acc))
            });
            constraints.push(Constraint::Lookup {
                span,
                reference: reference.clone(),
                expected: curried,
            });
            result
        }
        CPattern::Tuple(subpatterns) => {
            let elem_tys = subpatterns
                .iter()
                .map(|p| constrain_pattern(sub, scope, p, constraints))
                .collect();
            sub.fresh_bound(Structure::Tuple(elem_tys))
        }
        CPattern::Cons(head, tail) => {
            let head_ty = constrain_pattern(sub, scope, head, constraints);
            let tail_ty = constrain_pattern(sub, scope, tail, constraints);
            let list_ty = app1(sub, "List", head_ty);
            constraints.push(Constraint::Equal {
                span,
                expected: tail_ty,
                actual: list_ty,
            });
            list_ty
        }
        CPattern::Nil => {
            let elem_ty = sub.fresh_unbound();
            app1(sub, "List", elem_ty)
        }
        CPattern::As(inner, name) => {
            let ty = constrain_pattern(sub, scope, inner, constraints);
            scope.bind(name, ty);
            ty
        }
        CPattern::Unit => sub.fresh_bound(Structure::Unit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knot_syntax::span::Span;

    fn s() -> Span {
        Span::new(0, 0)
    }

    fn p(node: CPattern) -> Spanned<CPattern> {
        Spanned::new(s(), node)
    }

    #[test]
    fn wildcard_binds_nothing() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        constrain_pattern(&mut sub, &mut scope, &p(CPattern::Wildcard(None)), &mut cs);
        assert!(cs.is_empty());
    }

    #[test]
    fn named_wildcard_is_not_bound_into_scope() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Wildcard(Some("dbg".to_string()))),
            &mut cs,
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scope.lookup("dbg")));
        assert!(result.is_err(), "named wildcard must not be usable");
    }

    #[test]
    fn var_pattern_binds_a_fresh_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let ty = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Var("x".to_string())),
            &mut cs,
        );
        assert_eq!(scope.lookup("x").header_ty(), ty);
    }

    #[test]
    fn int_and_string_literal_patterns_get_the_right_builtin_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let int_ty = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Literal(PatternLiteral::Int(1))),
            &mut cs,
        );
        let str_ty = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Literal(PatternLiteral::Str("x".to_string()))),
            &mut cs,
        );
        assert_eq!(
            sub.resolve_structure(int_ty),
            Some(Structure::App(
                knot_canonical::ast::Ref::Builtin("Int".to_string()),
                vec![]
            ))
        );
        assert_eq!(
            sub.resolve_structure(str_ty),
            Some(Structure::App(
                knot_canonical::ast::Ref::Builtin("String".to_string()),
                vec![]
            ))
        );
    }

    #[test]
    fn ctor_pattern_binds_subpatterns_and_emits_a_curried_lookup() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let reference = knot_canonical::ast::Ref::Builtin("Some".to_string());
        let result = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Ctor(
                reference.clone(),
                vec![p(CPattern::Var("x".to_string()))],
            )),
            &mut cs,
        );
        assert_eq!(cs.len(), 1);
        match &cs[0] {
            Constraint::Lookup {
                reference: r,
                expected,
                ..
            } => {
                assert_eq!(*r, reference);
                match sub.resolve_structure(*expected) {
                    Some(Structure::Fn(field, ret)) => {
                        assert_eq!(field, scope.lookup("x").header_ty());
                        assert_eq!(ret, result);
                    }
                    other => panic!("expected a curried Fn shape, got {other:?}"),
                }
            }
            other => panic!("expected a Lookup constraint, got {other:?}"),
        }
    }

    #[test]
    fn tuple_pattern_builds_a_tuple_type_from_subpatterns() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let ty = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Tuple(vec![
                p(CPattern::Var("a".to_string())),
                p(CPattern::Var("b".to_string())),
            ])),
            &mut cs,
        );
        match sub.resolve_structure(ty) {
            Some(Structure::Tuple(elems)) => {
                assert_eq!(
                    elems,
                    vec![scope.lookup("a").header_ty(), scope.lookup("b").header_ty()]
                );
            }
            other => panic!("expected a Tuple shape, got {other:?}"),
        }
    }

    #[test]
    fn cons_pattern_requires_the_tail_to_be_a_list_of_the_head_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let ty = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Cons(
                Box::new(p(CPattern::Var("h".to_string()))),
                Box::new(p(CPattern::Nil)),
            )),
            &mut cs,
        );
        // Solve the one Equal constraint the Cons case emits.
        for c in cs {
            if let Constraint::Equal {
                expected, actual, ..
            } = c
            {
                crate::unify::unify(&mut sub, expected, actual).unwrap();
            }
        }
        match sub.resolve_structure(ty) {
            Some(Structure::App(r, args)) => {
                assert_eq!(r, knot_canonical::ast::Ref::Builtin("List".to_string()));
                assert_eq!(args.len(), 1);
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn nil_pattern_is_a_list_of_a_fresh_element_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let ty = constrain_pattern(&mut sub, &mut scope, &p(CPattern::Nil), &mut cs);
        assert!(matches!(
            sub.resolve_structure(ty),
            Some(Structure::App(r, args)) if r == knot_canonical::ast::Ref::Builtin("List".to_string()) && args.len() == 1
        ));
    }

    #[test]
    fn as_pattern_binds_the_alias_to_the_inner_patterns_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let ty = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::As(
                Box::new(p(CPattern::Var("x".to_string()))),
                "full".to_string(),
            )),
            &mut cs,
        );
        assert_eq!(scope.lookup("x").header_ty(), ty);
        assert_eq!(scope.lookup("full").header_ty(), ty);
    }

    #[test]
    fn unit_pattern_is_unit() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let ty = constrain_pattern(&mut sub, &mut scope, &p(CPattern::Unit), &mut cs);
        assert_eq!(sub.resolve_structure(ty), Some(Structure::Unit));
    }
}
