//! Annotation-key → expected-type derivation (plan §3.5, spec §13.4): what
//! type an annotation's value must have. Most keys have a fixed type;
//! `unravel`'s is *derived* from the signature of whatever binding it's
//! attached to (plan §3.5's "the hard case"). `solver`'s own shape isn't
//! pinned down yet (the discussion doc's §8 proposal, referenced in the
//! plan) and deliberately isn't modeled here.
//!
//! Checking an annotation's *value* against the type this module derives is
//! a further step (ordinary constraint generation + solving over the
//! value's own `CExpr`) that isn't wired up yet — `constrain::expr`'s
//! `Annotated` case still looks straight through to the target without
//! touching annotation values at all (see its own doc comment). This module
//! only answers "what type would that value need," not "does it have one."

use knot_canonical::ast::Ref;

use crate::annotation::sensitivity::sensitivity_of;
use crate::ty::Structure;
use crate::var::{Substitution, TypeVarId};

fn app0(sub: &mut Substitution, name: &str) -> TypeVarId {
    sub.fresh_bound(Structure::App(Ref::Builtin(name.to_string()), vec![]))
}

/// The fixed-type keys (spec §13.4) — everything except `unravel`. `None`
/// for `unravel` itself (see `derive_unravel_type`) and for any unknown
/// key (the annotation set is open, spec §13.4 — an unrecognized key isn't
/// an error at this layer, just something this table has nothing to say
/// about).
pub fn fixed_expected_type(sub: &mut Substitution, key: &str) -> Option<TypeVarId> {
    match key {
        "nodeId" | "label" | "doc" | "color" | "group" => Some(app0(sub, "String")),
        "position" => {
            let x = app0(sub, "Float");
            let y = app0(sub, "Float");
            Some(sub.fresh_bound(Structure::Tuple(vec![x, y])))
        }
        "collapsed" => Some(app0(sub, "Bool")),
        _ => None,
    }
}

/// `unravel`'s expected type (plan §3.5's derivation rule), given the
/// annotated binding's own already-built function type `f_ty` — a curried
/// chain of `Structure::Fn`s ending in a non-function `Out`. Builds
/// `Sensitivity Out -> UnravelInput A -> UnravelInput B -> ... -> Maybe
/// (A, B, ...)`, collapsing the result to a bare `Maybe A` for a single
/// parameter (matching how a 1-element grouping isn't a real tuple in Knot,
/// spec §5.5). `None` if `f_ty` isn't a function at all (arity 0 — nothing
/// to unravel), or has more than 3 parameters: spec's tuple-arity cap means
/// that case needs a record instead of a bare tuple, which the plan's own
/// §3.5 follow-ups already flagged as an authoring convention to handle
/// later, not something to paper over here.
pub fn derive_unravel_type(sub: &mut Substitution, f_ty: TypeVarId) -> Option<TypeVarId> {
    let mut params = Vec::new();
    let mut cur = sub.find(f_ty);
    while let Some(Structure::Fn(a, b)) = sub.resolve_structure(cur) {
        params.push(a);
        cur = sub.find(b);
    }
    if params.is_empty() || params.len() > 3 {
        return None;
    }
    let out = cur;

    let sensitivity_out = sensitivity_of(sub, out);
    let unravel_inputs: Vec<TypeVarId> = params
        .iter()
        .map(|p| {
            sub.fresh_bound(Structure::App(
                Ref::Builtin("UnravelInput".to_string()),
                vec![*p],
            ))
        })
        .collect();
    let result = if params.len() == 1 {
        params[0]
    } else {
        sub.fresh_bound(Structure::Tuple(params))
    };
    let maybe_result = sub.fresh_bound(Structure::App(
        Ref::Builtin("Maybe".to_string()),
        vec![result],
    ));
    let chain = unravel_inputs
        .into_iter()
        .rev()
        .fold(maybe_result, |acc, input| {
            sub.fresh_bound(Structure::Fn(input, acc))
        });
    Some(sub.fresh_bound(Structure::Fn(sensitivity_out, chain)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app0(sub: &mut Substitution, name: &str) -> TypeVarId {
        sub.fresh_bound(Structure::App(Ref::Builtin(name.to_string()), vec![]))
    }

    #[test]
    fn fixed_keys_get_their_documented_types() {
        let mut sub = Substitution::new();
        let node_id_ty = fixed_expected_type(&mut sub, "nodeId").unwrap();
        assert_eq!(
            sub.resolve_structure(node_id_ty),
            Some(Structure::App(Ref::Builtin("String".to_string()), vec![]))
        );
        let collapsed_ty = fixed_expected_type(&mut sub, "collapsed").unwrap();
        assert_eq!(
            sub.resolve_structure(collapsed_ty),
            Some(Structure::App(Ref::Builtin("Bool".to_string()), vec![]))
        );
        let position_ty = fixed_expected_type(&mut sub, "position").unwrap();
        match sub.resolve_structure(position_ty) {
            Some(Structure::Tuple(elems)) => assert_eq!(elems.len(), 2),
            other => panic!("expected a 2-tuple, got {other:?}"),
        }
    }

    #[test]
    fn unravel_is_not_a_fixed_key() {
        let mut sub = Substitution::new();
        assert!(fixed_expected_type(&mut sub, "unravel").is_none());
    }

    #[test]
    fn unknown_key_has_no_expected_type() {
        let mut sub = Substitution::new();
        assert!(fixed_expected_type(&mut sub, "bogus").is_none());
    }

    #[test]
    fn derives_the_single_parameter_shape() {
        // f :: Int -> Bool  =>  unravel :: Sensitivity Bool -> UnravelInput Int -> Maybe Int
        let mut sub = Substitution::new();
        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let f_ty = sub.fresh_bound(Structure::Fn(int_ty, bool_ty));
        let derived = derive_unravel_type(&mut sub, f_ty).unwrap();

        match sub.resolve_structure(derived) {
            Some(Structure::Fn(sens, rest)) => {
                assert!(matches!(
                    sub.resolve_structure(sens),
                    Some(Structure::App(r, args)) if r == Ref::Builtin("Sensitivity".to_string()) && args == vec![bool_ty]
                ));
                match sub.resolve_structure(rest) {
                    Some(Structure::Fn(unravel_input, maybe_result)) => {
                        assert!(matches!(
                            sub.resolve_structure(unravel_input),
                            Some(Structure::App(r, args))
                                if r == Ref::Builtin("UnravelInput".to_string()) && args == vec![int_ty]
                        ));
                        assert!(matches!(
                            sub.resolve_structure(maybe_result),
                            Some(Structure::App(r, args)) if r == Ref::Builtin("Maybe".to_string()) && args == vec![int_ty]
                        ));
                    }
                    other => panic!("expected UnravelInput Int -> Maybe Int, got {other:?}"),
                }
            }
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn derives_the_multi_parameter_tuple_shape() {
        // f :: A -> B -> C  =>  Sensitivity C -> UnravelInput A -> UnravelInput B -> Maybe (A, B)
        let mut sub = Substitution::new();
        let a = app0(&mut sub, "Int");
        let b = app0(&mut sub, "Bool");
        let c = app0(&mut sub, "String");
        let bc = sub.fresh_bound(Structure::Fn(b, c));
        let f_ty = sub.fresh_bound(Structure::Fn(a, bc));
        let derived = derive_unravel_type(&mut sub, f_ty).unwrap();

        let Some(Structure::Fn(_sens, rest1)) = sub.resolve_structure(derived) else {
            panic!("expected a Fn shape")
        };
        let Some(Structure::Fn(_input_a, rest2)) = sub.resolve_structure(rest1) else {
            panic!("expected a second Fn arrow")
        };
        let Some(Structure::Fn(_input_b, maybe_result)) = sub.resolve_structure(rest2) else {
            panic!("expected a third Fn arrow")
        };
        match sub.resolve_structure(maybe_result) {
            Some(Structure::App(r, args)) if r == Ref::Builtin("Maybe".to_string()) => {
                match sub.resolve_structure(args[0]) {
                    Some(Structure::Tuple(elems)) => assert_eq!(elems.len(), 2),
                    other => panic!("expected a Tuple(A, B), got {other:?}"),
                }
            }
            other => panic!("expected Maybe (A, B), got {other:?}"),
        }
    }

    #[test]
    fn zero_parameter_binding_has_no_unravel_type() {
        let mut sub = Substitution::new();
        let out = app0(&mut sub, "Int");
        assert!(derive_unravel_type(&mut sub, out).is_none());
    }

    #[test]
    fn more_than_three_parameters_is_not_handled_yet() {
        let mut sub = Substitution::new();
        let int_ty = app0(&mut sub, "Int");
        let mut f_ty = int_ty;
        for _ in 0..4 {
            f_ty = sub.fresh_bound(Structure::Fn(int_ty, f_ty));
        }
        assert!(derive_unravel_type(&mut sub, f_ty).is_none());
    }
}
