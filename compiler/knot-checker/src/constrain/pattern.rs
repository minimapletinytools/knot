//! Constraint generation over `CPattern` (mirrors Elm's own
//! `Constrain/Pattern.hs`, conceptually). Each function returns the
//! pattern's own inferred type, wrapped in a `TypedPattern` (Fix #3's Stage
//! A — see `ast.rs`'s own doc comment) — relating that type to whatever
//! it's actually being matched against (a scrutinee, a signature's
//! parameter type, ...) is the caller's job, via an ordinary
//! `Constraint::Equal`, not this module's.
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

use crate::ast::{Typed, TypedPattern};
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
) -> TypedPattern {
    let span = pattern.span;
    let wrap = |ty, node| Typed { span, ty, node };
    match &pattern.node {
        // A named wildcard (`_name`) is a discard exactly like `_` -- spec
        // §15.1/§15.2: the name is a mnemonic for the reader only, never
        // compiler-checked, so it's never bound into scope.
        CPattern::Wildcard(name) => wrap(
            sub.fresh_unbound(),
            crate::ast::TPattern::Wildcard(name.clone()),
        ),
        CPattern::Var(name) => {
            let ty = sub.fresh_unbound();
            scope.bind(name, ty);
            wrap(ty, crate::ast::TPattern::Var(name.clone()))
        }
        CPattern::Literal(lit @ PatternLiteral::Int(_)) => {
            wrap(app0(sub, "Int"), crate::ast::TPattern::Literal(lit.clone()))
        }
        CPattern::Literal(lit @ PatternLiteral::Str(_)) => wrap(
            app0(sub, "String"),
            crate::ast::TPattern::Literal(lit.clone()),
        ),
        // Arity against the constructor's declared field count is already
        // checked during canonicalization (see `CPattern::Ctor`'s own doc
        // comment in knot-canonical's `ast.rs`) -- `subpatterns.len()` is
        // trusted as correct here, no re-check needed.
        CPattern::Ctor(reference, subpatterns) => {
            let typed_subs: Vec<TypedPattern> = subpatterns
                .iter()
                .map(|p| constrain_pattern(sub, scope, p, constraints))
                .collect();
            let result = sub.fresh_unbound();
            let curried = typed_subs.iter().rev().fold(result, |acc, sub_pat| {
                sub.fresh_bound(Structure::Fn(sub_pat.ty, acc))
            });
            constraints.push(Constraint::Lookup {
                span,
                reference: reference.clone(),
                expected: curried,
            });
            wrap(
                result,
                crate::ast::TPattern::Ctor(reference.clone(), typed_subs),
            )
        }
        CPattern::Tuple(subpatterns) => {
            let typed_subs: Vec<TypedPattern> = subpatterns
                .iter()
                .map(|p| constrain_pattern(sub, scope, p, constraints))
                .collect();
            let elem_tys = typed_subs.iter().map(|p| p.ty).collect();
            wrap(
                sub.fresh_bound(Structure::Tuple(elem_tys)),
                crate::ast::TPattern::Tuple(typed_subs),
            )
        }
        CPattern::Cons(head, tail) => {
            let head_typed = constrain_pattern(sub, scope, head, constraints);
            let tail_typed = constrain_pattern(sub, scope, tail, constraints);
            let list_ty = app1(sub, "List", head_typed.ty);
            constraints.push(Constraint::Equal {
                span,
                expected: tail_typed.ty,
                actual: list_ty,
            });
            wrap(
                list_ty,
                crate::ast::TPattern::Cons(Box::new(head_typed), Box::new(tail_typed)),
            )
        }
        CPattern::Nil => {
            let elem_ty = sub.fresh_unbound();
            wrap(app1(sub, "List", elem_ty), crate::ast::TPattern::Nil)
        }
        // Open row, exactly like `constrain::expr`'s own `FieldAccess`/
        // `RecordUpdate` -- `{ x, y }` only ever demands *at least* an `x`
        // and a `y` field (spec §5.4/§8.1), never these fields exactly.
        CPattern::Record(fields) => {
            let mut field_tys = std::collections::BTreeMap::new();
            let mut typed_fields = Vec::with_capacity(fields.len());
            for name in fields {
                let ty = sub.fresh_unbound();
                scope.bind(name, ty);
                field_tys.insert(name.clone(), ty);
                typed_fields.push((name.clone(), ty));
            }
            let rest = sub.fresh_unbound();
            let record_ty = sub.fresh_bound(Structure::Record(field_tys, Some(rest)));
            wrap(record_ty, crate::ast::TPattern::Record(typed_fields))
        }
        CPattern::As(inner, name) => {
            let inner_typed = constrain_pattern(sub, scope, inner, constraints);
            scope.bind(name, inner_typed.ty);
            let ty = inner_typed.ty;
            wrap(
                ty,
                crate::ast::TPattern::As(Box::new(inner_typed), name.clone()),
            )
        }
        CPattern::Unit => wrap(sub.fresh_bound(Structure::Unit), crate::ast::TPattern::Unit),
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
        let typed = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Var("x".to_string())),
            &mut cs,
        );
        assert_eq!(scope.lookup("x").header_ty(), typed.ty);
        assert_eq!(typed.node, crate::ast::TPattern::Var("x".to_string()));
    }

    #[test]
    fn int_and_string_literal_patterns_get_the_right_builtin_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let int_typed = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Literal(PatternLiteral::Int(1))),
            &mut cs,
        );
        let str_typed = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Literal(PatternLiteral::Str("x".to_string()))),
            &mut cs,
        );
        assert_eq!(
            sub.resolve_structure(int_typed.ty),
            Some(Structure::App(
                knot_canonical::ast::Ref::Builtin("Int".to_string()),
                vec![]
            ))
        );
        assert_eq!(
            sub.resolve_structure(str_typed.ty),
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
        let reference = knot_canonical::ast::Ref::Builtin("Just".to_string());
        let typed = constrain_pattern(
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
                        assert_eq!(ret, typed.ty);
                    }
                    other => panic!("expected a curried Fn shape, got {other:?}"),
                }
            }
            other => panic!("expected a Lookup constraint, got {other:?}"),
        }
        match &typed.node {
            crate::ast::TPattern::Ctor(r, subs) => {
                assert_eq!(*r, reference);
                assert_eq!(subs.len(), 1);
            }
            other => panic!("expected a TPattern::Ctor, got {other:?}"),
        }
    }

    #[test]
    fn tuple_pattern_builds_a_tuple_type_from_subpatterns() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let typed = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Tuple(vec![
                p(CPattern::Var("a".to_string())),
                p(CPattern::Var("b".to_string())),
            ])),
            &mut cs,
        );
        match sub.resolve_structure(typed.ty) {
            Some(Structure::Tuple(elems)) => {
                assert_eq!(
                    elems,
                    vec![scope.lookup("a").header_ty(), scope.lookup("b").header_ty()]
                );
            }
            other => panic!("expected a Tuple shape, got {other:?}"),
        }
        assert!(matches!(typed.node, crate::ast::TPattern::Tuple(ref subs) if subs.len() == 2));
    }

    #[test]
    fn record_pattern_binds_each_field_and_is_an_open_row() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let typed = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::Record(vec!["x".to_string(), "y".to_string()])),
            &mut cs,
        );
        match sub.resolve_structure(typed.ty) {
            Some(Structure::Record(fields, rest)) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields.get("x"), Some(&scope.lookup("x").header_ty()));
                assert_eq!(fields.get("y"), Some(&scope.lookup("y").header_ty()));
                // Open, not closed -- `{ x, y }` demands *at least* these
                // fields, same reasoning as an explicit `{ r | x : ..., y :
                // ... }` signature (spec §5.4/§8.1).
                assert!(rest.is_some());
            }
            other => panic!("expected an open Record shape, got {other:?}"),
        }
        assert!(matches!(typed.node, crate::ast::TPattern::Record(ref fs) if fs.len() == 2));
    }

    #[test]
    fn cons_pattern_requires_the_tail_to_be_a_list_of_the_head_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let typed = constrain_pattern(
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
        match sub.resolve_structure(typed.ty) {
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
        let typed = constrain_pattern(&mut sub, &mut scope, &p(CPattern::Nil), &mut cs);
        assert!(matches!(
            sub.resolve_structure(typed.ty),
            Some(Structure::App(r, args)) if r == knot_canonical::ast::Ref::Builtin("List".to_string()) && args.len() == 1
        ));
    }

    #[test]
    fn as_pattern_binds_the_alias_to_the_inner_patterns_type() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let typed = constrain_pattern(
            &mut sub,
            &mut scope,
            &p(CPattern::As(
                Box::new(p(CPattern::Var("x".to_string()))),
                "full".to_string(),
            )),
            &mut cs,
        );
        assert_eq!(scope.lookup("x").header_ty(), typed.ty);
        assert_eq!(scope.lookup("full").header_ty(), typed.ty);
    }

    #[test]
    fn unit_pattern_is_unit() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let mut cs = Vec::new();
        let typed = constrain_pattern(&mut sub, &mut scope, &p(CPattern::Unit), &mut cs);
        assert_eq!(sub.resolve_structure(typed.ty), Some(Structure::Unit));
    }
}
