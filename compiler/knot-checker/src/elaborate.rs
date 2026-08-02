//! Dictionary resolution (plan §3/§7): given a concrete `HasInstance`
//! obligation and the fully-solved state `solve::solve` +
//! `interface::instance::build_instance_table` produce, determine exactly
//! which instance answers it. See `ast.rs`'s own doc comment for what's
//! *not* wired up yet (a full `CExpr` -> `TExpr` tree walk) and why.
//!
//! **Existence-checking vs. dictionary *construction*, post Fix #4.**
//! `interface::instance::check_instance` now correctly confirms a
//! parametric or structural (`Tuple`/`Record`/`Unit`) obligation
//! recursively — `resolve_dictionary` uses it too, so e.g. `List Weird`'s
//! `Eq` is correctly rejected here, not just by `check_pending`. But
//! *building* the `Dictionary` value itself still only succeeds for a
//! `Structure::App`-headed obligation, unchanged from before: `ast::
//! Dictionary`'s `{interface, head}` shape has a `Ref` to put in `head`
//! for that case, but no natural `Ref` at all for a `Tuple`/`Record` (there
//! is no single named instance answering "how do I compare this tuple" —
//! it would need each element's own dictionary threaded in, recursively,
//! same shape problem `TExpr` elaboration itself has). Designing that
//! properly belongs with a future fix, once there's a real elaboration
//! *consumer* to validate the shape against — inventing one now risks
//! exactly the "looks complete but is subtly wrong" trap `ast.rs`'s own
//! doc comment already warns against for the tree-walk gap.
//!
//! **`elaborate_module` is Fix #3's Stage B** — walks every top-level
//! binding's `elaborated_body` (Stage A, `constrain::expr`/`constrain::
//! decl`) and resolves each node's own interface obligations, using
//! `resolve_dictionary` for the ones a `BinOp`/`Negate` node already knows
//! at generation time, and `obligations` (built by `solve::
//! solve_with_obligations`) to recover a `Var`/`Ctor` reference's own,
//! only-known-after-instantiation ones — see `ast.rs`'s own doc comment for
//! why these need two different mechanisms. See `ObligationResolution`'s
//! own doc comment for why a plain `Result<Dictionary, TypeError>` isn't
//! enough here: a genuinely polymorphic binding's own body (`useEq x y = x
//! == y`, called nowhere at a concrete type in this module) has obligations
//! that never resolve to anything concrete *in this module* at all — that's
//! not a `NoInstance` error, it's `StillAbstract`.
use std::collections::HashMap;

use knot_syntax::span::Span;

use crate::ast::{Dictionary, ObligationResolution, TExpr, TypedExpr};
use crate::constrain::LetMember;
use crate::error::{TypeError, TypeErrorKind};
use crate::interface::instance::{check_instance, InstanceTable};
use crate::solve::{ObligationMap, PendingInstance};
use crate::ty::Structure;
use crate::var::{Substitution, TypeVarId};

/// Resolves one obligation to the `Dictionary` that answers it. `ty` must
/// already be fully resolved (post-`solve::solve`) to a concrete,
/// `Structure::App`-headed type with a (possibly parametric, now-
/// recursively-checked) matching instance — anything else (still
/// unresolved, or headed by a structural type, see module docs) reports
/// the same `TypeErrorKind::NoInstance` `interface::instance::check_pending`
/// would, rather than confirming a dictionary that isn't really there.
pub fn resolve_dictionary(
    sub: &mut Substitution,
    table: &InstanceTable,
    interface: &str,
    ty: TypeVarId,
    span: Span,
) -> Result<Dictionary, TypeError> {
    match sub.resolve_structure(ty) {
        Some(Structure::App(head, _)) if check_instance(sub, table, interface, ty) => {
            Ok(Dictionary {
                interface: interface.to_string(),
                head,
            })
        }
        _ => Err(TypeError {
            span,
            kind: TypeErrorKind::NoInstance {
                interface: interface.to_string(),
            },
        }),
    }
}

/// Resolves every obligation `solve::solve` left unresolved, in one pass —
/// the elaboration-time counterpart of `interface::instance::check_pending`
/// (which performs the same check but only to report an error, never
/// keeping the resolved `Dictionary` — this is what a full elaboration
/// pass, once the tree-walk gap `ast.rs` documents is closed, would thread
/// back into each call site). Collects every dictionary it can and every
/// error it finds, rather than stopping at the first — same stance as
/// `solve::solve` and `knot-canonical`'s own error handling.
pub fn resolve_pending(
    sub: &mut Substitution,
    table: &InstanceTable,
    pending: Vec<PendingInstance>,
) -> (Vec<Dictionary>, Vec<TypeError>) {
    let mut dictionaries = Vec::new();
    let mut errors = Vec::new();
    for p in pending {
        match resolve_dictionary(sub, table, &p.interface, p.ty, p.span) {
            Ok(d) => dictionaries.push(d),
            Err(e) => errors.push(e),
        }
    }
    (dictionaries, errors)
}

/// Fix #3's Stage B: resolves every interface obligation reachable from
/// `members`' own `elaborated_body`s, keyed by `(interface, TypeVarId)` (a
/// canonicalized `sub.find(ty)`, since e.g. two sibling operators sharing
/// the same operand can otherwise record the exact same pair twice). See
/// module docs on `ObligationResolution`. `obligations` is `solve::
/// solve_with_obligations`'s own side-table, needed to recover a `Var`/
/// `Ctor` node's obligations (unknown at generation time, unlike a
/// `BinOp`/`Negate` node's, which are already on the node itself).
pub fn elaborate_module(
    sub: &mut Substitution,
    table: &InstanceTable,
    obligations: &ObligationMap,
    members: &[LetMember],
) -> (
    HashMap<(String, TypeVarId), ObligationResolution>,
    Vec<TypeError>,
) {
    let mut resolved = HashMap::new();
    let mut errors = Vec::new();
    for m in members {
        walk_expr(
            sub,
            table,
            obligations,
            &m.elaborated_body,
            &mut resolved,
            &mut errors,
        );
    }
    (resolved, errors)
}

/// One obligation's resolution, memoized into `resolved` by its
/// canonicalized `(interface, ty)` key. `ty` still fully unresolved (a bare
/// flexible variable, `sub.resolve_structure` returns `None`) means this
/// obligation belongs to some enclosing binding's own polymorphism —
/// `StillAbstract`, not an error (see module docs). Anything else goes
/// through the ordinary `resolve_dictionary`, exactly as `check_pending`
/// would judge it.
fn resolve_one(
    sub: &mut Substitution,
    table: &InstanceTable,
    interface: &str,
    ty: TypeVarId,
    span: Span,
    resolved: &mut HashMap<(String, TypeVarId), ObligationResolution>,
    errors: &mut Vec<TypeError>,
) {
    let key = (interface.to_string(), sub.find(ty));
    if resolved.contains_key(&key) {
        return;
    }
    if sub.resolve_structure(ty).is_none() {
        resolved.insert(key, ObligationResolution::StillAbstract);
        return;
    }
    match resolve_dictionary(sub, table, interface, ty, span) {
        Ok(d) => {
            resolved.insert(key, ObligationResolution::Concrete(d));
        }
        Err(e) => errors.push(e),
    }
}

/// Walks every `TExpr` node reachable from `typed`, resolving whatever
/// obligations it carries -- either directly on the node (`BinOp`/
/// `Negate`) or via `obligations`, keyed by the node's own `ty` (`Var`/
/// `Ctor`). `TPattern` is never walked: per `ast.rs`'s own doc comment, a
/// data constructor's scheme never has `constraints` in this language, so a
/// pattern can never carry an obligation to resolve in the first place.
fn walk_expr(
    sub: &mut Substitution,
    table: &InstanceTable,
    obligations: &ObligationMap,
    typed: &TypedExpr,
    resolved: &mut HashMap<(String, TypeVarId), ObligationResolution>,
    errors: &mut Vec<TypeError>,
) {
    let mut recurse = |e: &TypedExpr, resolved: &mut _, errors: &mut _| {
        walk_expr(sub, table, obligations, e, resolved, errors)
    };
    match &typed.node {
        TExpr::IntLit(_) | TExpr::FloatLit(_) | TExpr::StringLit(_) | TExpr::Unit | TExpr::Hole => {
        }
        TExpr::Var(_) | TExpr::Ctor(_) => {
            if let Some(pairs) = obligations.get(&typed.ty) {
                for (interface, ty) in pairs.clone() {
                    resolve_one(sub, table, &interface, ty, typed.span, resolved, errors);
                }
            }
        }
        TExpr::Lambda(_, body) => recurse(body, resolved, errors),
        TExpr::App(f, arg) => {
            recurse(f, resolved, errors);
            recurse(arg, resolved, errors);
        }
        TExpr::BinOp(_, l, r, node_obligations) => {
            recurse(l, resolved, errors);
            recurse(r, resolved, errors);
            for (interface, ty) in node_obligations {
                resolve_one(sub, table, interface, *ty, typed.span, resolved, errors);
            }
        }
        TExpr::Negate(inner, node_obligations) => {
            recurse(inner, resolved, errors);
            for (interface, ty) in node_obligations {
                resolve_one(sub, table, interface, *ty, typed.span, resolved, errors);
            }
        }
        TExpr::If(c, t, e) => {
            recurse(c, resolved, errors);
            recurse(t, resolved, errors);
            recurse(e, resolved, errors);
        }
        TExpr::Let(bindings, body) => {
            for (_, rhs) in bindings {
                recurse(rhs, resolved, errors);
            }
            recurse(body, resolved, errors);
        }
        TExpr::Case(scrutinee, arms) => {
            recurse(scrutinee, resolved, errors);
            for (_, body) in arms {
                recurse(body, resolved, errors);
            }
        }
        TExpr::List(elems) | TExpr::Tuple(elems) => {
            for el in elems {
                recurse(el, resolved, errors);
            }
        }
        TExpr::Record(fields) => {
            for (_, e) in fields {
                recurse(e, resolved, errors);
            }
        }
        TExpr::RecordUpdate(base, updates) => {
            recurse(base, resolved, errors);
            for (_, e) in updates {
                recurse(e, resolved, errors);
            }
        }
        TExpr::FieldAccess(base, _) => recurse(base, resolved, errors),
        TExpr::Annotated(_, target) => recurse(target, resolved, errors),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knot_canonical::ast::{CDecl, Ref};
    use knot_syntax::span::Spanned;

    fn span() -> Span {
        Span::new(0, 0)
    }

    fn decls(src: &str) -> Vec<Spanned<CDecl>> {
        let mut state = knot_syntax::ParseState::new(src);
        let raw = state.parse_decls().unwrap();
        assert!(state.is_eof(), "leftover input: {src}");
        knot_canonical::canonicalize_decls(&raw).unwrap_or_else(|errs| panic!("{errs:?}"))
    }

    /// Digs through a top-level binding's `Lambda(params, body)` wrapper
    /// (`constrain::decl` always builds one when there are params) to reach
    /// its own `BinOp`'s first recorded obligation.
    fn binop_obligation(typed: &TypedExpr) -> (String, TypeVarId) {
        match &typed.node {
            TExpr::Lambda(_, body) => binop_obligation(body),
            TExpr::BinOp(_, _, _, obligations) => obligations[0].clone(),
            other => panic!("expected a Lambda wrapping a BinOp, got {other:?}"),
        }
    }

    #[test]
    fn resolves_a_concrete_obligation_with_a_matching_instance() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Num", Ref::Builtin("Int".to_string()), vec![]);
        let ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));

        let dict = resolve_dictionary(&mut sub, &table, "Num", ty, span()).unwrap();
        assert_eq!(
            dict,
            Dictionary {
                interface: "Num".to_string(),
                head: Ref::Builtin("Int".to_string()),
            }
        );
    }

    #[test]
    fn errors_when_no_instance_exists() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_bound(Structure::App(Ref::Builtin("String".to_string()), vec![]));

        let err = resolve_dictionary(&mut sub, &table, "Num", ty, span()).unwrap_err();
        assert!(matches!(&err.kind, TypeErrorKind::NoInstance { interface } if interface == "Num"));
    }

    #[test]
    fn errors_on_a_still_unresolved_type_variable() {
        let mut sub = Substitution::new();
        let table = InstanceTable::new();
        let ty = sub.fresh_unbound();

        let err = resolve_dictionary(&mut sub, &table, "Eq", ty, span()).unwrap_err();
        assert!(matches!(&err.kind, TypeErrorKind::NoInstance { .. }));
    }

    #[test]
    fn resolve_pending_splits_confirmed_dictionaries_from_errors() {
        let mut sub = Substitution::new();
        let mut table = InstanceTable::new();
        table.insert_builtin("Num", Ref::Builtin("Int".to_string()), vec![]);
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let string_ty = sub.fresh_bound(Structure::App(Ref::Builtin("String".to_string()), vec![]));

        let pending = vec![
            PendingInstance {
                span: span(),
                interface: "Num".to_string(),
                ty: int_ty,
            },
            PendingInstance {
                span: span(),
                interface: "Num".to_string(),
                ty: string_ty,
            },
        ];
        let (dictionaries, errors) = resolve_pending(&mut sub, &table, pending);
        assert_eq!(dictionaries.len(), 1);
        assert_eq!(errors.len(), 1);
    }

    // -- Fix #3's Stage B: elaborate_module --

    #[test]
    fn a_concrete_signatures_binop_obligation_resolves_directly() {
        let mut sub = Substitution::new();
        let cs = decls("check :: Int -> Int -> Bool\ncheck x y = x == y\n");
        let (tree, members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        let (_pending, errors, obligations) =
            crate::solve::solve_with_obligations(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");

        let mut table = InstanceTable::new();
        table.insert_builtin("Eq", Ref::Builtin("Int".to_string()), vec![]);
        let (resolved, elab_errors) = elaborate_module(&mut sub, &table, &obligations, &members);
        assert!(elab_errors.is_empty(), "{elab_errors:?}");

        let check_member = members.iter().find(|m| m.name == "check").unwrap();
        let (interface, ty) = binop_obligation(&check_member.elaborated_body);
        let key = (interface, sub.find(ty));
        assert_eq!(
            resolved.get(&key),
            Some(&ObligationResolution::Concrete(Dictionary {
                interface: "Eq".to_string(),
                head: Ref::Builtin("Int".to_string()),
            }))
        );
    }

    #[test]
    fn a_polymorphic_bindings_own_body_obligation_is_still_abstract() {
        // useEq is never called anywhere in this module -- its own body's
        // Eq obligation was absorbed into its own generalized scheme
        // (solve.rs's `generalize`), not concretely resolved. Walking its
        // own elaborated_body directly must report StillAbstract, not a
        // spurious NoInstance error.
        let mut sub = Substitution::new();
        let cs = decls("useEq x y = x == y\n");
        let (tree, members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        let (_pending, errors, obligations) =
            crate::solve::solve_with_obligations(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");

        let table = InstanceTable::new();
        let (resolved, elab_errors) = elaborate_module(&mut sub, &table, &obligations, &members);
        assert!(elab_errors.is_empty(), "{elab_errors:?}");

        let use_eq_member = members.iter().find(|m| m.name == "useEq").unwrap();
        let (interface, ty) = binop_obligation(&use_eq_member.elaborated_body);
        let key = (interface, sub.find(ty));
        assert_eq!(
            resolved.get(&key),
            Some(&ObligationResolution::StillAbstract)
        );
    }

    #[test]
    fn a_call_sites_lookup_obligation_resolves_via_the_solve_side_table() {
        // f itself stays polymorphic (same shape as the test above), but
        // result = f 1 2 instantiates it at a concrete Int -- that
        // particular *reference*'s own obligation (tracked via solve::
        // solve_with_obligations's side-table, not anything recorded at
        // generation time) should resolve concretely, even though f's own
        // internal body does not.
        let mut sub = Substitution::new();
        let cs = decls("f x y = x == y\nresult = f 1 2\n");
        let (tree, members) = crate::constrain::decl::constrain_module(&mut sub, &cs);
        let mut env = crate::solve::SchemeEnv::new();
        let (_pending, errors, obligations) =
            crate::solve::solve_with_obligations(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");

        let mut table = InstanceTable::new();
        table.insert_builtin("Eq", Ref::Builtin("Int".to_string()), vec![]);
        let (resolved, elab_errors) = elaborate_module(&mut sub, &table, &obligations, &members);
        assert!(elab_errors.is_empty(), "{elab_errors:?}");

        let result_member = members.iter().find(|m| m.name == "result").unwrap();
        // result's own body is `f 1 2`: App(App(Var(f), IntLit(1)), IntLit(2)).
        let f_ref_ty = match &result_member.elaborated_body.node {
            TExpr::App(inner, _) => match &inner.node {
                TExpr::App(f_ref, _) => f_ref.ty,
                other => panic!("expected App(Var(f), 1), got {other:?}"),
            },
            other => panic!("expected App(f 1, 2), got {other:?}"),
        };
        // `f_ref_ty` is the *reference's own* type (the whole `a -> a ->
        // Bool` function) -- the actual Eq obligation is on the
        // *instantiated* `a`, a different variable recorded in
        // `obligations` under this reference's key.
        let pairs = obligations
            .get(&f_ref_ty)
            .expect("f's reference should have a recorded Eq obligation");
        assert_eq!(pairs.len(), 1);
        let (interface, obligation_ty) = &pairs[0];
        let key = (interface.clone(), sub.find(*obligation_ty));
        assert_eq!(
            resolved.get(&key),
            Some(&ObligationResolution::Concrete(Dictionary {
                interface: "Eq".to_string(),
                head: Ref::Builtin("Int".to_string()),
            }))
        );
    }
}
