//! Built-in instance and scheme wiring (plan §7, TM8): the actual
//! `SchemeEnv`/`InstanceTable` entries a real program's `Constraint::Lookup`
//! and `HasInstance` obligations resolve against. Parallel to
//! `knot-canonical::prelude` — that crate answers "is this name real,"
//! this one answers "what type does it have."
//!
//! **Not seeded, and not able to be yet: the Collection/Context interface
//! values** (spec §6.3/§6.4) — `map`, `foldl`, `foldr`, `filter`, `length`,
//! `pure`, `bind`. Their signatures (`map :: (a -> b) -> f a -> f b`) are
//! polymorphic over `f` itself, a type *constructor* (`List`, `Map`,
//! `Option`, ...), not an ordinary type — `Structure::App(Ref, Vec
//! <TypeVarId>)` has no way to represent "some type constructor, not yet
//! known" as a `TypeVarId`, only a concrete `Ref` head. Giving these a
//! signature at all needs a real design decision (a new `Structure` variant
//! for a higher-kinded variable, plus everywhere that pattern-matches
//! `Structure` learning what it means) that's genuinely out of scope to
//! invent as a byproduct of seeding the prelude — flagged here rather than
//! worked around with an incorrect concrete-`f` signature.
//!
//! Also not seeded: `Eq`/`Ord`/`Show` for `List`/`Option`/`Result` are
//! registered as head-level instances (so `List Int == List Int`-shaped
//! *concrete* uses resolve), but — per `interface::instance`'s own doc
//! comment — this table never checks a parametric instance's own element
//! constraint recursively, so e.g. `List SomeTypeWithNoEqInstance` would
//! incorrectly be accepted as `Eq`-able. A real gap, inherited from TM6,
//! not introduced here.

use knot_canonical::ast::Ref;

use crate::interface::instance::InstanceTable;
use crate::solve::SchemeEnv;
use crate::solve::SchemeKey;
use crate::ty::{Scheme, Structure};
use crate::var::Substitution;

fn app0(sub: &mut Substitution, name: &str) -> crate::var::TypeVarId {
    sub.fresh_bound(Structure::App(Ref::Builtin(name.to_string()), vec![]))
}

/// Populates a fresh `SchemeEnv`/`InstanceTable` pair with every built-in
/// instance and value scheme this crate can currently give a correct
/// signature to — see module docs for what's deliberately left out.
pub fn seed(sub: &mut Substitution) -> (SchemeEnv, InstanceTable) {
    let mut env = SchemeEnv::new();
    let mut table = InstanceTable::new();
    seed_instances(&mut table);
    seed_values(sub, &mut env);
    (env, table)
}

fn seed_instances(table: &mut InstanceTable) {
    // spec §6.2: instances built-in for Num Int/Float, Integral Int, Fractional Float.
    table.insert_builtin("Num", Ref::Builtin("Int".to_string()));
    table.insert_builtin("Num", Ref::Builtin("Float".to_string()));
    table.insert_builtin("Integral", Ref::Builtin("Int".to_string()));
    table.insert_builtin("Fractional", Ref::Builtin("Float".to_string()));

    // Eq/Ord/Show for every primitive that needs them (plan §9's open
    // question #1's own answer: String, Bool, Unit alongside the numerics).
    for ty in ["Int", "Float", "String", "Bool", "Unit"] {
        table.insert_builtin("Eq", Ref::Builtin(ty.to_string()));
        table.insert_builtin("Ord", Ref::Builtin(ty.to_string()));
        table.insert_builtin("Show", Ref::Builtin(ty.to_string()));
    }

    // Container head instances -- concrete-element-type uses resolve
    // correctly; see module docs on the still-missing recursive element
    // check.
    for container in ["List", "Option", "Result"] {
        table.insert_builtin("Eq", Ref::Builtin(container.to_string()));
        table.insert_builtin("Ord", Ref::Builtin(container.to_string()));
        table.insert_builtin("Show", Ref::Builtin(container.to_string()));
    }
}

fn seed_values(sub: &mut Substitution, env: &mut SchemeEnv) {
    let bool_ty = app0(sub, "Bool");
    let string_ty = app0(sub, "String");
    let ordering_ty = app0(sub, "Ordering");

    // not :: Bool -> Bool (spec §4.8's "Boolean Operators" note)
    let not_ty = sub.fresh_bound(Structure::Fn(bool_ty, bool_ty));
    env.insert(
        SchemeKey::Builtin("not".to_string()),
        Scheme::monomorphic(not_ty),
    );

    // compare :: Ord a => a -> a -> Ordering
    {
        let a = sub.fresh_unbound();
        let a_to_ordering = sub.fresh_bound(Structure::Fn(a, ordering_ty));
        let ty = sub.fresh_bound(Structure::Fn(a, a_to_ordering));
        env.insert(
            SchemeKey::Builtin("compare".to_string()),
            Scheme {
                vars: vec![a],
                constraints: vec![(a, "Ord".to_string())],
                ty,
            },
        );
    }

    // show :: Show a => a -> String
    {
        let a = sub.fresh_unbound();
        let ty = sub.fresh_bound(Structure::Fn(a, string_ty));
        env.insert(
            SchemeKey::Builtin("show".to_string()),
            Scheme {
                vars: vec![a],
                constraints: vec![(a, "Show".to_string())],
                ty,
            },
        );
    }

    // negate, abs, signum :: Num a => a -> a
    for name in ["negate", "abs", "signum"] {
        let a = sub.fresh_unbound();
        let ty = sub.fresh_bound(Structure::Fn(a, a));
        env.insert(
            SchemeKey::Builtin(name.to_string()),
            Scheme {
                vars: vec![a],
                constraints: vec![(a, "Num".to_string())],
                ty,
            },
        );
    }

    // recip :: Fractional a => a -> a
    {
        let a = sub.fresh_unbound();
        let ty = sub.fresh_bound(Structure::Fn(a, a));
        env.insert(
            SchemeKey::Builtin("recip".to_string()),
            Scheme {
                vars: vec![a],
                constraints: vec![(a, "Fractional".to_string())],
                ty,
            },
        );
    }

    // div, mod :: Integral a => a -> a -> a
    for name in ["div", "mod"] {
        let a = sub.fresh_unbound();
        let a_to_a = sub.fresh_bound(Structure::Fn(a, a));
        let ty = sub.fresh_bound(Structure::Fn(a, a_to_a));
        env.insert(
            SchemeKey::Builtin(name.to_string()),
            Scheme {
                vars: vec![a],
                constraints: vec![(a, "Integral".to_string())],
                ty,
            },
        );
    }

    // fromIntegral :: (Integral a, Num b) => a -> b (spec §6.2) -- the return-
    // type-only variable `b` this whole design already accommodates without
    // any special-casing (see the earlier monomorphism-restriction decision).
    {
        let a = sub.fresh_unbound();
        let b = sub.fresh_unbound();
        let ty = sub.fresh_bound(Structure::Fn(a, b));
        env.insert(
            SchemeKey::Builtin("fromIntegral".to_string()),
            Scheme {
                vars: vec![a, b],
                constraints: vec![(a, "Integral".to_string()), (b, "Num".to_string())],
                ty,
            },
        );
    }

    // empty :: Monoid a => a (spec §6.1)
    {
        let a = sub.fresh_unbound();
        env.insert(
            SchemeKey::Builtin("empty".to_string()),
            Scheme {
                vars: vec![a],
                constraints: vec![(a, "Monoid".to_string())],
                ty: a,
            },
        );
    }

    seed_constructors(sub, env, bool_ty, ordering_ty);
}

fn seed_constructors(
    sub: &mut Substitution,
    env: &mut SchemeEnv,
    bool_ty: crate::var::TypeVarId,
    ordering_ty: crate::var::TypeVarId,
) {
    env.insert(
        SchemeKey::Builtin("True".to_string()),
        Scheme::monomorphic(bool_ty),
    );
    env.insert(
        SchemeKey::Builtin("False".to_string()),
        Scheme::monomorphic(bool_ty),
    );
    env.insert(
        SchemeKey::Builtin("LT".to_string()),
        Scheme::monomorphic(ordering_ty),
    );
    env.insert(
        SchemeKey::Builtin("EQ".to_string()),
        Scheme::monomorphic(ordering_ty),
    );
    env.insert(
        SchemeKey::Builtin("GT".to_string()),
        Scheme::monomorphic(ordering_ty),
    );

    // Some :: a -> Option a
    {
        let a = sub.fresh_unbound();
        let option_a = sub.fresh_bound(Structure::App(Ref::Builtin("Option".to_string()), vec![a]));
        let ty = sub.fresh_bound(Structure::Fn(a, option_a));
        env.insert(
            SchemeKey::Builtin("Some".to_string()),
            Scheme {
                vars: vec![a],
                constraints: vec![],
                ty,
            },
        );
    }
    // None :: Option a
    {
        let a = sub.fresh_unbound();
        let option_a = sub.fresh_bound(Structure::App(Ref::Builtin("Option".to_string()), vec![a]));
        env.insert(
            SchemeKey::Builtin("None".to_string()),
            Scheme {
                vars: vec![a],
                constraints: vec![],
                ty: option_a,
            },
        );
    }
    // Ok :: a -> Result e a
    {
        let e = sub.fresh_unbound();
        let a = sub.fresh_unbound();
        let result_ea = sub.fresh_bound(Structure::App(
            Ref::Builtin("Result".to_string()),
            vec![e, a],
        ));
        let ty = sub.fresh_bound(Structure::Fn(a, result_ea));
        env.insert(
            SchemeKey::Builtin("Ok".to_string()),
            Scheme {
                vars: vec![e, a],
                constraints: vec![],
                ty,
            },
        );
    }
    // Err :: e -> Result e a
    {
        let e = sub.fresh_unbound();
        let a = sub.fresh_unbound();
        let result_ea = sub.fresh_bound(Structure::App(
            Ref::Builtin("Result".to_string()),
            vec![e, a],
        ));
        let ty = sub.fresh_bound(Structure::Fn(e, result_ea));
        env.insert(
            SchemeKey::Builtin("Err".to_string()),
            Scheme {
                vars: vec![e, a],
                constraints: vec![],
                ty,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constrain::decl::constrain_module;
    use crate::interface::instance::check_pending;
    use knot_canonical::ast::CDecl;
    use knot_syntax::span::Spanned;

    fn decls(src: &str) -> Vec<Spanned<CDecl>> {
        let mut state = knot_syntax::ParseState::new(src);
        let raw = state.parse_decls().unwrap();
        assert!(state.is_eof(), "leftover input: {src}");
        knot_canonical::canonicalize_decls(&raw).unwrap_or_else(|errs| panic!("{errs:?}"))
    }

    #[test]
    fn seeded_num_instances_let_ordinary_arithmetic_type_check_end_to_end() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls("f x y = x + y\nresult = f 1 2\n");
        let tree = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn seeded_eq_and_true_false_let_boolean_returning_code_type_check() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls("isZero n = if n == 0 then True else False\n");
        let tree = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn from_integral_resolves_its_return_only_type_variable_from_context() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        // 1.0 + fromIntegral n -- b gets pinned to Float by the surrounding +.
        // a (n's own type) stays free -- no numeric-literal-style defaulting
        // to Int exists in this design (a deliberate decision), so f is
        // correctly generalized as `Integral a => a -> Float`, not `Int ->
        // Float`; Int just happens to be the only seeded Integral instance.
        let cs = decls("f n = 1.0 + fromIntegral n\n");
        let tree = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");

        let scheme = env
            .get(&SchemeKey::TopLevel("f".to_string()))
            .expect("f should have a scheme")
            .clone();
        assert_eq!(scheme.vars.len(), 1);
        assert_eq!(
            scheme.constraints,
            vec![(scheme.vars[0], "Integral".to_string())]
        );
        match sub.resolve_structure(scheme.ty) {
            Some(Structure::Fn(arg, ret)) => {
                assert_eq!(sub.find(arg), sub.find(scheme.vars[0]));
                let float_ty = app0(&mut sub, "Float");
                assert_eq!(sub.resolve_structure(ret), sub.resolve_structure(float_ty));
            }
            other => panic!("expected a -> Float, got {other:?}"),
        }
    }

    #[test]
    fn using_an_interface_with_no_seeded_instance_for_the_type_is_a_no_instance_error() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        // Ordering has no seeded Num instance.
        let cs = decls("bad = LT + EQ\n");
        let tree = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(&mut sub, &table, pending, &mut errors);
        assert!(errors
            .iter()
            .any(|e| matches!(&e.kind, crate::error::TypeErrorKind::NoInstance { interface } if interface == "Num")));
    }
}
