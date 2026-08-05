//! `Expr` -> `CExpr`: resolves every `Var`/`Ctor` leaf and opens the right kind
//! of scope for each binding form. Three different binding disciplines show up
//! and each gets different treatment here:
//!
//! - **`Lambda` params**: one shared group (spec: `\x x -> ...` must be
//!   rejected even though neither param repeats a name *within itself* — see
//!   `resolve::pattern::resolve_pattern_group`).
//! - **`Let` bindings**: simultaneous/mutually recursive, matching Haskell's
//!   `let` — every binding's pattern is bound *before* any binding's RHS (or
//!   the body) is resolved, so bindings can reference each other and
//!   themselves (laziness makes this sound; see spec §1). A name repeated
//!   across two different bindings in the same block is exactly as wrong as
//!   repeating it in one pattern, so this also shares one duplicate-binding
//!   group across the whole block.
//! - **`Do` binds**: sequential, matching Haskell `do`-notation's desugaring
//!   to nested `>>=` — each `<-`'s right-hand expression is resolved *before*
//!   its own pattern is bound, and a later bind legitimately shadows an
//!   earlier one (it's sequential rebinding, not a simultaneous group).
//!
//! `Case` arms each get their own fresh scope — sibling arms are alternatives,
//! not bindings visible to each other, so reusing a name across two arms
//! (`Circle r -> ... | Square r -> ...`) is completely normal.

use knot_syntax::ast::expr::{DoStmt, Expr};
use knot_syntax::span::Spanned;

use crate::ast::{CAnnotation, CDoStmt, CExpr};
use crate::env::Env;
use crate::error::CanonError;
use crate::resolve::pattern::{resolve_pattern, resolve_pattern_group, resolve_pattern_shared};
use crate::resolve::{unresolved_to_ref, NameKind};

pub fn resolve_expr(
    env: &mut Env,
    expr: &Spanned<Expr>,
    errors: &mut Vec<CanonError>,
) -> Spanned<CExpr> {
    let span = expr.span;
    let node = match &expr.node {
        Expr::IntLit(n) => CExpr::IntLit(*n),
        Expr::FloatLit(f) => CExpr::FloatLit(*f),
        Expr::StringLit(s) => CExpr::StringLit(s.clone()),
        Expr::Unit => CExpr::Unit,
        Expr::Var(name) => {
            let r = match env.resolve_value(name) {
                Ok(r) => r,
                Err(kind) => unresolved_to_ref(kind, name, NameKind::Value, span, errors),
            };
            CExpr::Var(r)
        }
        Expr::Ctor(name) => {
            let r = match env.resolve_ctor(name) {
                Ok((r, _info)) => r,
                Err(kind) => unresolved_to_ref(kind, name, NameKind::Ctor, span, errors),
            };
            CExpr::Ctor(r)
        }
        Expr::Hole => CExpr::Hole,
        Expr::Lambda(params, body) => {
            env.push_scope();
            let cparams = resolve_pattern_group(env, params, errors);
            let cbody = resolve_expr(env, body, errors);
            env.pop_scope();
            CExpr::Lambda(cparams, Box::new(cbody))
        }
        Expr::App(f, a) => CExpr::App(
            Box::new(resolve_expr(env, f, errors)),
            Box::new(resolve_expr(env, a, errors)),
        ),
        Expr::BinOp(op, l, r) => CExpr::BinOp(
            *op,
            Box::new(resolve_expr(env, l, errors)),
            Box::new(resolve_expr(env, r, errors)),
        ),
        Expr::OpRef(op) => CExpr::OpRef(*op),
        Expr::Negate(inner) => CExpr::Negate(Box::new(resolve_expr(env, inner, errors))),
        Expr::If(c, t, e) => CExpr::If(
            Box::new(resolve_expr(env, c, errors)),
            Box::new(resolve_expr(env, t, errors)),
            Box::new(resolve_expr(env, e, errors)),
        ),
        Expr::Let(bindings, body) => {
            env.push_scope();
            let mut bound = std::collections::HashSet::new();
            let patterns: Vec<_> = bindings
                .iter()
                .map(|(pat, _)| resolve_pattern_shared(env, pat, &mut bound, errors))
                .collect();
            let values: Vec<_> = bindings
                .iter()
                .map(|(_, value)| resolve_expr(env, value, errors))
                .collect();
            let cbindings = patterns.into_iter().zip(values).collect();
            let cbody = resolve_expr(env, body, errors);
            env.pop_scope();
            CExpr::Let(cbindings, Box::new(cbody))
        }
        Expr::Case(scrutinee, arms) => {
            let cscrutinee = resolve_expr(env, scrutinee, errors);
            let carms = arms
                .iter()
                .map(|(pat, body)| {
                    env.push_scope();
                    let cpat = resolve_pattern(env, pat, errors);
                    let cbody = resolve_expr(env, body, errors);
                    env.pop_scope();
                    (cpat, cbody)
                })
                .collect();
            CExpr::Case(Box::new(cscrutinee), carms)
        }
        Expr::Do(stmts, final_expr) => {
            env.push_scope();
            let cstmts = stmts
                .iter()
                .map(|stmt| resolve_do_stmt(env, stmt, errors))
                .collect();
            let cfinal = resolve_expr(env, final_expr, errors);
            env.pop_scope();
            CExpr::Do(cstmts, Box::new(cfinal))
        }
        Expr::List(items) => {
            CExpr::List(items.iter().map(|e| resolve_expr(env, e, errors)).collect())
        }
        Expr::Tuple(items) => {
            CExpr::Tuple(items.iter().map(|e| resolve_expr(env, e, errors)).collect())
        }
        Expr::Record(fields) => CExpr::Record(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), resolve_expr(env, value, errors)))
                .collect(),
        ),
        Expr::RecordUpdate(base, fields) => CExpr::RecordUpdate(
            Box::new(resolve_expr(env, base, errors)),
            fields
                .iter()
                .map(|(name, value)| (name.clone(), resolve_expr(env, value, errors)))
                .collect(),
        ),
        Expr::FieldAccess(base, field) => {
            CExpr::FieldAccess(Box::new(resolve_expr(env, base, errors)), field.clone())
        }
        Expr::Annotated(annotations, target) => CExpr::Annotated(
            annotations
                .iter()
                .map(|a| CAnnotation {
                    key: a.key.clone(),
                    value: resolve_expr(env, &a.value, errors),
                })
                .collect(),
            Box::new(resolve_expr(env, target, errors)),
        ),
    };
    Spanned::new(span, node)
}

/// `<-` binds resolve their right-hand expression *before* binding their own
/// pattern (a later statement's monadic bind can't see itself or forward
/// statements — same order Haskell's `>>=` desugaring implies), and
/// legitimately shadow an earlier bind of the same name (sequential, not a
/// simultaneous group — see module docs).
fn resolve_do_stmt(env: &mut Env, stmt: &DoStmt, errors: &mut Vec<CanonError>) -> CDoStmt {
    match stmt {
        DoStmt::Bind(pat, value) => {
            let cvalue = resolve_expr(env, value, errors);
            let cpat = resolve_pattern(env, pat, errors);
            CDoStmt::Bind(cpat, cvalue)
        }
        DoStmt::Expr(e) => CDoStmt::Expr(resolve_expr(env, e, errors)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knot_syntax::ast::pattern::Pattern;
    use knot_syntax::span::Span;

    fn sp<T>(node: T) -> Spanned<T> {
        Spanned::new(Span::new(0, 1), node)
    }

    #[test]
    fn unbound_variable_is_an_error() {
        let mut env = Env::for_decls();
        let mut errors = Vec::new();
        resolve_expr(&mut env, &sp(Expr::Var("mystery".to_string())), &mut errors);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn builtin_ctor_resolves() {
        let mut env = Env::for_decls();
        let mut errors = Vec::new();
        let c = resolve_expr(&mut env, &sp(Expr::Ctor("True".to_string())), &mut errors);
        assert!(errors.is_empty());
        assert!(matches!(c.node, CExpr::Ctor(crate::ast::Ref::Builtin(_))));
    }

    #[test]
    fn lambda_param_is_visible_in_body() {
        let mut env = Env::for_decls();
        let mut errors = Vec::new();
        let lambda = sp(Expr::Lambda(
            vec![sp(Pattern::Var("x".to_string()))],
            Box::new(sp(Expr::Var("x".to_string()))),
        ));
        resolve_expr(&mut env, &lambda, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn lambda_duplicate_params_is_an_error() {
        let mut env = Env::for_decls();
        let mut errors = Vec::new();
        let lambda = sp(Expr::Lambda(
            vec![
                sp(Pattern::Var("x".to_string())),
                sp(Pattern::Var("x".to_string())),
            ],
            Box::new(sp(Expr::IntLit(1))),
        ));
        resolve_expr(&mut env, &lambda, &mut errors);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn let_bindings_are_mutually_recursive() {
        // let isEven n = if n == 0 then True else isOdd (n - 1)
        //     isOdd n  = if n == 0 then False else isEven (n - 1)
        // in isEven
        let src_bindings = vec![
            (
                sp(Pattern::Var("isEven".to_string())),
                sp(Expr::Var("isOdd".to_string())),
            ),
            (
                sp(Pattern::Var("isOdd".to_string())),
                sp(Expr::Var("isEven".to_string())),
            ),
        ];
        let let_expr = sp(Expr::Let(
            src_bindings,
            Box::new(sp(Expr::Var("isEven".to_string()))),
        ));
        let mut env = Env::for_decls();
        let mut errors = Vec::new();
        resolve_expr(&mut env, &let_expr, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn let_duplicate_binding_across_block_is_an_error() {
        let let_expr = sp(Expr::Let(
            vec![
                (sp(Pattern::Var("x".to_string())), sp(Expr::IntLit(1))),
                (sp(Pattern::Var("x".to_string())), sp(Expr::IntLit(2))),
            ],
            Box::new(sp(Expr::Var("x".to_string()))),
        ));
        let mut env = Env::for_decls();
        let mut errors = Vec::new();
        resolve_expr(&mut env, &let_expr, &mut errors);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn case_arms_may_reuse_the_same_pattern_variable_name() {
        let case_expr = sp(Expr::Case(
            Box::new(sp(Expr::IntLit(0))),
            vec![
                (
                    sp(Pattern::Var("r".to_string())),
                    sp(Expr::Var("r".to_string())),
                ),
                (
                    sp(Pattern::Var("r".to_string())),
                    sp(Expr::Var("r".to_string())),
                ),
            ],
        ));
        let mut env = Env::for_decls();
        let mut errors = Vec::new();
        resolve_expr(&mut env, &case_expr, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn do_bind_rhs_cannot_see_its_own_pattern() {
        // do { x <- x; pure x } -- the `x` being bound can't appear in its own RHS
        let do_expr = sp(Expr::Do(
            vec![DoStmt::Bind(
                sp(Pattern::Var("x".to_string())),
                sp(Expr::Var("x".to_string())),
            )],
            Box::new(sp(Expr::Var("x".to_string()))),
        ));
        let mut env = Env::for_decls();
        let mut errors = Vec::new();
        resolve_expr(&mut env, &do_expr, &mut errors);
        assert_eq!(errors.len(), 1, "{errors:?}");
    }

    #[test]
    fn do_bind_may_shadow_an_earlier_bind_sequentially() {
        let do_expr = sp(Expr::Do(
            vec![
                DoStmt::Bind(sp(Pattern::Var("x".to_string())), sp(Expr::IntLit(1))),
                DoStmt::Bind(
                    sp(Pattern::Var("x".to_string())),
                    sp(Expr::Var("x".to_string())),
                ),
            ],
            Box::new(sp(Expr::Var("x".to_string()))),
        ));
        let mut env = Env::for_decls();
        let mut errors = Vec::new();
        resolve_expr(&mut env, &do_expr, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn annotation_value_is_resolved_too() {
        use knot_syntax::ast::expr::Annotation;
        let expr = sp(Expr::Annotated(
            vec![Annotation {
                key: "nodeId".to_string(),
                value: sp(Expr::Var("mystery".to_string())),
            }],
            Box::new(sp(Expr::Var("f".to_string()))),
        ));
        let mut env = Env::for_decls();
        let mut errors = Vec::new();
        resolve_expr(&mut env, &expr, &mut errors);
        // both `mystery` (annotation value) and `f` (target) are unbound
        assert_eq!(errors.len(), 2, "{errors:?}");
    }
}
