//! Built-in instance and scheme wiring (plan §7, TM8): the actual
//! `SchemeEnv`/`InstanceTable` entries a real program's `Constraint::Lookup`
//! and `HasInstance` obligations resolve against. Parallel to
//! `knot-canonical::prelude` — that crate answers "is this name real,"
//! this one answers "what type does it have."
//!
//! **`Collection`/`Context`** (spec §6.3/§6.4) — `map`, `foldl`, `foldr`,
//! `filter`, `length`, `pure`, `bind` — are seeded with real, higher-kinded
//! signatures now (`knot-checker-gaps-plan.md`'s Fix #2): each one's `f` is
//! a `Substitution::fresh_ctor_unbound` variable threaded through a
//! `Structure::VarApp`, exactly like an ordinary type variable elsewhere in
//! this file, just constrained by `Collection`/`Context` instead of e.g.
//! `Num`. See `ty::Structure::VarApp`'s own doc comment for why that needs
//! no new machinery beyond the two `Structure` variants themselves.
//!
//! `Eq`/`Ord`/`Show` for `List`/`Maybe`/`Result` are registered with a
//! `requires` on each own element position, so `interface::instance::
//! check_instance`'s recursive check (Fix #4) correctly rejects e.g. `List
//! SomeTypeWithNoEqInstance` rather than inheriting `List`'s own head-level
//! instance unconditionally.

use knot_canonical::ast::Ref;

use crate::interface::instance::InstanceTable;
use crate::solve::SchemeEnv;
use crate::solve::SchemeKey;
use crate::ty::{Scheme, Structure};
use crate::var::Substitution;

fn app0(sub: &mut Substitution, name: &str) -> crate::var::TypeVarId {
    sub.fresh_bound(Structure::App(Ref::Builtin(name.to_string()), vec![]))
}

/// `f arg` for a constructor-*variable*-headed application (spec §6.3/§6.4)
/// -- the higher-kinded counterpart of `app0`/`app1` above, which only ever
/// build a fixed, concrete head.
fn var_app(
    sub: &mut Substitution,
    f: crate::var::TypeVarId,
    arg: crate::var::TypeVarId,
) -> crate::var::TypeVarId {
    sub.fresh_bound(Structure::VarApp(f, vec![arg]))
}

/// Builds the curried function type `args[0] -> args[1] -> ... -> ret`,
/// folding from the right exactly like every hand-curried signature
/// elsewhere in this file (and `constrain::expr`'s `Lambda` case) already
/// does -- factored out here since `Collection`/`Context`'s signatures are
/// long enough that repeating the fold inline at each one would obscure the
/// actual shape being built.
fn curried(
    sub: &mut Substitution,
    args: &[crate::var::TypeVarId],
    ret: crate::var::TypeVarId,
) -> crate::var::TypeVarId {
    args.iter()
        .rev()
        .fold(ret, |acc, &a| sub.fresh_bound(Structure::Fn(a, acc)))
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
    table.insert_builtin("Num", Ref::Builtin("Int".to_string()), vec![]);
    table.insert_builtin("Num", Ref::Builtin("Float".to_string()), vec![]);
    table.insert_builtin("Integral", Ref::Builtin("Int".to_string()), vec![]);
    table.insert_builtin("Fractional", Ref::Builtin("Float".to_string()), vec![]);

    // Eq/Ord/Show for every primitive that needs them (plan §9's open
    // question #1's own answer: String, Bool alongside the numerics). Unit
    // deliberately isn't here -- `interface::instance::check_instance`'s
    // hardcoded `Structure::Unit` case answers for it structurally now
    // (Fix #4), and `Ref::Builtin("Unit")` is unreachable in practice
    // anyway: Knot's grammar only ever produces the unit type via literal
    // `()` syntax (`Type::Unit`/`CType::Unit`), never by naming the
    // identifier `Unit`.
    for ty in ["Int", "Float", "String", "Bool"] {
        table.insert_builtin("Eq", Ref::Builtin(ty.to_string()), vec![]);
        table.insert_builtin("Ord", Ref::Builtin(ty.to_string()), vec![]);
        table.insert_builtin("Show", Ref::Builtin(ty.to_string()), vec![]);
    }

    // Container instances -- `requires` is what makes these genuinely
    // recursive now (Fix #4): `List Weird`'s `Eq` correctly fails unless
    // `Weird` itself has one, instead of the container's own instance
    // being unconditional.
    for container in ["List", "Maybe"] {
        for interface in ["Eq", "Ord", "Show"] {
            table.insert_builtin(
                interface,
                Ref::Builtin(container.to_string()),
                vec![(0, interface.to_string())],
            );
        }
    }

    // Semigroup/Monoid for String (concatenation, `""`) and List
    // (concatenation, `[]`) -- previously missing entirely (Fix #11: no
    // type anywhere had either instance, so `<>` and `empty` failed on
    // every builtin type, found via `corpus/programs`'s own realistic-
    // program probing). Concatenating either needs nothing from its own
    // element type, unlike Eq/Ord/Show above, so `requires` stays empty.
    for ty in ["String", "List"] {
        table.insert_builtin("Semigroup", Ref::Builtin(ty.to_string()), vec![]);
        table.insert_builtin("Monoid", Ref::Builtin(ty.to_string()), vec![]);
    }
    // Result e a -- both positions need the interface (comparing/showing a
    // Result needs both its error and value types to support it).
    for interface in ["Eq", "Ord", "Show"] {
        table.insert_builtin(
            interface,
            Ref::Builtin("Result".to_string()),
            vec![(0, interface.to_string()), (1, interface.to_string())],
        );
    }

    // spec §6.3/§6.4: Collection (List, Map), Context (Maybe, Result, IO,
    // List) -- keyed by the constructor's own `Ref`, exactly like every
    // other instance here; no `requires` of their own (the obligation is
    // on the bare constructor variable itself, not on some argument
    // position of it -- see `interface::instance::check_instance`'s
    // `Structure::Ctor` case).
    table.insert_builtin("Collection", Ref::Builtin("List".to_string()), vec![]);
    table.insert_builtin("Collection", Ref::Builtin("Map".to_string()), vec![]);
    for context in ["Maybe", "Result", "IO", "List"] {
        table.insert_builtin("Context", Ref::Builtin(context.to_string()), vec![]);
    }
}

fn seed_values(sub: &mut Substitution, env: &mut SchemeEnv) {
    let bool_ty = app0(sub, "Bool");
    let string_ty = app0(sub, "String");
    let ordering_ty = app0(sub, "Ordering");
    let int_ty = app0(sub, "Int");

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

    seed_collection_and_context(sub, env, bool_ty, int_ty);
    seed_constructors(sub, env, bool_ty, ordering_ty);
}

/// Real, higher-kinded signatures for spec §6.3's `Collection` (`map`,
/// `foldl`, `foldr`, `filter`, `length`) and §6.4's `Context` (`pure`,
/// `bind`) -- each one's `f` is a `Substitution::fresh_ctor_unbound`
/// variable, constrained by the relevant interface exactly like an ordinary
/// type variable elsewhere in this file is constrained by e.g. `Num`.
fn seed_collection_and_context(
    sub: &mut Substitution,
    env: &mut SchemeEnv,
    bool_ty: crate::var::TypeVarId,
    int_ty: crate::var::TypeVarId,
) {
    // map :: Collection f => (a -> b) -> f a -> f b
    {
        let f = sub.fresh_ctor_unbound();
        let a = sub.fresh_unbound();
        let b = sub.fresh_unbound();
        let a_to_b = sub.fresh_bound(Structure::Fn(a, b));
        let fa = var_app(sub, f, a);
        let fb = var_app(sub, f, b);
        let ty = curried(sub, &[a_to_b, fa], fb);
        env.insert(
            SchemeKey::Builtin("map".to_string()),
            Scheme {
                vars: vec![f, a, b],
                constraints: vec![(f, "Collection".to_string())],
                ty,
            },
        );
    }

    // foldl :: Collection f => (b -> a -> b) -> b -> f a -> b
    {
        let f = sub.fresh_ctor_unbound();
        let a = sub.fresh_unbound();
        let b = sub.fresh_unbound();
        let combine = curried(sub, &[b, a], b);
        let fa = var_app(sub, f, a);
        let ty = curried(sub, &[combine, b, fa], b);
        env.insert(
            SchemeKey::Builtin("foldl".to_string()),
            Scheme {
                vars: vec![f, a, b],
                constraints: vec![(f, "Collection".to_string())],
                ty,
            },
        );
    }

    // foldr :: Collection f => (a -> b -> b) -> b -> f a -> b
    {
        let f = sub.fresh_ctor_unbound();
        let a = sub.fresh_unbound();
        let b = sub.fresh_unbound();
        let combine = curried(sub, &[a, b], b);
        let fa = var_app(sub, f, a);
        let ty = curried(sub, &[combine, b, fa], b);
        env.insert(
            SchemeKey::Builtin("foldr".to_string()),
            Scheme {
                vars: vec![f, a, b],
                constraints: vec![(f, "Collection".to_string())],
                ty,
            },
        );
    }

    // filter :: Collection f => (a -> Bool) -> f a -> f a
    {
        let f = sub.fresh_ctor_unbound();
        let a = sub.fresh_unbound();
        let pred = curried(sub, &[a], bool_ty);
        let fa = var_app(sub, f, a);
        let ty = curried(sub, &[pred, fa], fa);
        env.insert(
            SchemeKey::Builtin("filter".to_string()),
            Scheme {
                vars: vec![f, a],
                constraints: vec![(f, "Collection".to_string())],
                ty,
            },
        );
    }

    // length :: Collection f => f a -> Int
    {
        let f = sub.fresh_ctor_unbound();
        let a = sub.fresh_unbound();
        let fa = var_app(sub, f, a);
        let ty = curried(sub, &[fa], int_ty);
        env.insert(
            SchemeKey::Builtin("length".to_string()),
            Scheme {
                vars: vec![f, a],
                constraints: vec![(f, "Collection".to_string())],
                ty,
            },
        );
    }

    // pure :: Context f => a -> f a
    {
        let f = sub.fresh_ctor_unbound();
        let a = sub.fresh_unbound();
        let fa = var_app(sub, f, a);
        let ty = curried(sub, &[a], fa);
        env.insert(
            SchemeKey::Builtin("pure".to_string()),
            Scheme {
                vars: vec![f, a],
                constraints: vec![(f, "Context".to_string())],
                ty,
            },
        );
    }

    // bind :: Context f => f a -> (a -> f b) -> f b (also exposed as (>>=), spec §6.4)
    {
        let f = sub.fresh_ctor_unbound();
        let a = sub.fresh_unbound();
        let b = sub.fresh_unbound();
        let fa = var_app(sub, f, a);
        let fb = var_app(sub, f, b);
        let a_to_fb = curried(sub, &[a], fb);
        let ty = curried(sub, &[fa, a_to_fb], fb);
        env.insert(
            SchemeKey::Builtin("bind".to_string()),
            Scheme {
                vars: vec![f, a, b],
                constraints: vec![(f, "Context".to_string())],
                ty,
            },
        );
    }
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

    // Just :: a -> Maybe a
    {
        let a = sub.fresh_unbound();
        let maybe_a = sub.fresh_bound(Structure::App(Ref::Builtin("Maybe".to_string()), vec![a]));
        let ty = sub.fresh_bound(Structure::Fn(a, maybe_a));
        env.insert(
            SchemeKey::Builtin("Just".to_string()),
            Scheme {
                vars: vec![a],
                constraints: vec![],
                ty,
            },
        );
    }
    // Nothing :: Maybe a
    {
        let a = sub.fresh_unbound();
        let maybe_a = sub.fresh_bound(Structure::App(Ref::Builtin("Maybe".to_string()), vec![a]));
        env.insert(
            SchemeKey::Builtin("Nothing".to_string()),
            Scheme {
                vars: vec![a],
                constraints: vec![],
                ty: maybe_a,
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
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn seeded_eq_and_true_false_let_boolean_returning_code_type_check() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls("isZero n = if n == 0 then True else False\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
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
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
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

    /// Normalizes either an ordinary `App` or a *resolved* `VarApp` (head
    /// pinned to a concrete `Ctor`) into `(head, args)` -- `resolve_structure`
    /// deliberately doesn't do this rewriting itself (a `VarApp` node's own
    /// stored structure never changes just because its head variable later
    /// gets resolved elsewhere; see `ty::Structure::VarApp`'s own doc
    /// comment), so these tests need to peek one level further for anything
    /// built through `map`/`filter`/etc.
    fn resolved_head(
        sub: &mut Substitution,
        ty: crate::var::TypeVarId,
    ) -> Option<(Ref, Vec<crate::var::TypeVarId>)> {
        match sub.resolve_structure(ty)? {
            Structure::App(r, args) => Some((r, args)),
            Structure::VarApp(f, args) => match sub.resolve_structure(f)? {
                // The Ctor's own leading (already-applied) arguments come
                // first, then the VarApp's own trailing one(s) -- see
                // `ty::Structure::Ctor`'s own doc comment.
                Structure::Ctor(r, mut leading) => {
                    leading.extend(args);
                    Some((r, leading))
                }
                _ => None,
            },
            _ => None,
        }
    }

    #[test]
    fn map_over_a_list_literal_infers_list_of_the_result_type() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls("result = map (\\x -> x + 1) [1, 2, 3]\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        let scheme = env
            .get(&SchemeKey::TopLevel("result".to_string()))
            .expect("result should have a scheme")
            .clone();
        assert!(scheme.vars.is_empty(), "result should be fully concrete");
        match resolved_head(&mut sub, scheme.ty) {
            Some((r, args)) if r == Ref::Builtin("List".to_string()) => {
                let int_ty = app0(&mut sub, "Int");
                assert_eq!(
                    sub.resolve_structure(args[0]),
                    sub.resolve_structure(int_ty)
                );
            }
            other => panic!("expected List Int, got {other:?}"),
        }
    }

    #[test]
    fn length_and_filter_type_check_against_a_list() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls(
            "n = length [1, 2, 3]\n\
             kept = filter (\\x -> x == 0) [1, 2, 3]\n",
        );
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        let n_scheme = env
            .get(&SchemeKey::TopLevel("n".to_string()))
            .unwrap()
            .clone();
        let int_ty = app0(&mut sub, "Int");
        assert_eq!(
            sub.resolve_structure(n_scheme.ty),
            sub.resolve_structure(int_ty)
        );

        let kept_scheme = env
            .get(&SchemeKey::TopLevel("kept".to_string()))
            .unwrap()
            .clone();
        match resolved_head(&mut sub, kept_scheme.ty) {
            Some((r, args)) if r == Ref::Builtin("List".to_string()) => {
                assert_eq!(
                    sub.resolve_structure(args[0]),
                    sub.resolve_structure(int_ty)
                );
            }
            other => panic!("expected List Int, got {other:?}"),
        }
    }

    #[test]
    fn map_over_a_non_collection_constructor_is_a_no_instance_error() {
        // Maybe is a Context, not a Collection (spec §6.3/§6.4) -- map's f
        // still happily unifies structurally with Maybe (unify doesn't
        // check instances), but check_pending must reject it.
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls("bad = map (\\x -> x) (Just 1)\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            crate::error::TypeErrorKind::NoInstance { interface } if interface == "Collection"
        )));
    }

    #[test]
    fn map_is_polymorphic_over_which_collection_constructor_it_targets() {
        // Same top-level `map` used at `List` in one binding and left
        // abstract (still a bare, uninstantiated ctor var) in another --
        // two independent instantiations of the same scheme must not leak
        // into each other (mirrors the `identity`-at-two-types flagship
        // test in solve.rs, but for a constructor-sorted variable).
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls(
            "overList = map (\\x -> x + 1) [1, 2, 3]\n\
             overListAgain = map (\\x -> x) [True]\n",
        );
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        let int_ty = app0(&mut sub, "Int");
        let bool_ty = app0(&mut sub, "Bool");
        let over_list = env
            .get(&SchemeKey::TopLevel("overList".to_string()))
            .unwrap()
            .clone();
        let over_list_again = env
            .get(&SchemeKey::TopLevel("overListAgain".to_string()))
            .unwrap()
            .clone();
        match (
            resolved_head(&mut sub, over_list.ty),
            resolved_head(&mut sub, over_list_again.ty),
        ) {
            (Some((r1, a1)), Some((r2, a2)))
                if r1 == Ref::Builtin("List".to_string())
                    && r2 == Ref::Builtin("List".to_string()) =>
            {
                assert_eq!(sub.resolve_structure(a1[0]), sub.resolve_structure(int_ty));
                assert_eq!(sub.resolve_structure(a2[0]), sub.resolve_structure(bool_ty));
            }
            other => panic!("expected two independent List results, got {other:?}"),
        }
    }

    #[test]
    fn do_notation_desugars_to_bind_and_pure_and_type_checks() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls("result = do\n  x <- Just 1\n  y <- Just 2\n  pure (x + y)\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        let scheme = env
            .get(&SchemeKey::TopLevel("result".to_string()))
            .unwrap()
            .clone();
        match resolved_head(&mut sub, scheme.ty) {
            Some((r, args)) if r == Ref::Builtin("Maybe".to_string()) => {
                let int_ty = app0(&mut sub, "Int");
                assert_eq!(
                    sub.resolve_structure(args[0]),
                    sub.resolve_structure(int_ty)
                );
            }
            other => panic!("expected Maybe Int, got {other:?}"),
        }
    }

    #[test]
    fn do_notation_over_a_2_parameter_result_type_checks_and_infers_correctly() {
        // Fix: VarApp only ever built a Ctor with zero leading arguments,
        // requiring an exact arity match against the concrete App it met --
        // so a 2-parameter Context like `Result e a` (unlike 1-parameter
        // `Maybe`/`List`) always failed to unify at all, breaking Result's
        // own do-notation and any map/bind/pure use entirely. Now the
        // error type `e` becomes the Ctor's own one leading argument,
        // leaving only the value type `a` for bind/pure's own VarApp
        // argument to vary over -- exactly like `Maybe`, just with an
        // extra fixed parameter carried alongside.
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls(concat!(
            "type ParseError = ParseError String\n",
            "result = do\n",
            "  x <- Ok 1\n",
            "  y <- Ok 2\n",
            "  pure (x + y)\n",
            "annotated :: Result ParseError Int\n",
            "annotated = result\n",
        ));
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        let scheme = env
            .get(&SchemeKey::TopLevel("result".to_string()))
            .unwrap()
            .clone();
        match resolved_head(&mut sub, scheme.ty) {
            Some((r, args)) if r == Ref::Builtin("Result".to_string()) => {
                assert_eq!(args.len(), 2);
                let int_ty = app0(&mut sub, "Int");
                assert_eq!(
                    sub.resolve_structure(args[1]),
                    sub.resolve_structure(int_ty)
                );
            }
            other => panic!("expected Result _ Int, got {other:?}"),
        }
    }

    #[test]
    fn using_an_interface_with_no_seeded_instance_for_the_type_is_a_no_instance_error() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        // Ordering has no seeded Num instance.
        let cs = decls("bad = LT + EQ\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
        assert!(errors
            .iter()
            .any(|e| matches!(&e.kind, crate::error::TypeErrorKind::NoInstance { interface } if interface == "Num")));
    }

    #[test]
    fn string_concatenation_via_semigroup_type_checks() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls("greet name = \"Hello, \" <> name\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn list_concatenation_via_semigroup_type_checks() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls("combine a b = a <> b\ncombined = combine [1, 2] [3, 4]\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn empty_resolves_for_string_and_list_via_monoid() {
        let mut sub = Substitution::new();
        let (mut env, table) = seed(&mut sub);
        let cs = decls(
            "emptyStr :: String\nemptyStr = empty\nemptyList :: List Int\nemptyList = empty\n",
        );
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (pending, mut errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        check_pending(
            &mut sub,
            &table,
            &std::collections::HashMap::new(),
            pending,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");
    }
}
