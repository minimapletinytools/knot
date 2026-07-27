//! `Pattern` -> `CPattern`: binds every variable it introduces into `Env`,
//! rejects the same name being bound twice within one *group* of patterns
//! (e.g. `(x, x)`, or `\x x -> ...` across two separate lambda params), and
//! checks constructor patterns against the constructor's declared arity.
//!
//! Named holes (`_name`) are deliberately never bound here, matching spec
//! §12.3's "the name carries no compiler-checked meaning" — registering one as
//! a resolvable local would contradict that, so `_debugValue` used later in a
//! body correctly still resolves as unbound.

use std::collections::HashSet;

use knot_syntax::ast::pattern::Pattern;
use knot_syntax::span::Spanned;

use crate::ast::CPattern;
use crate::env::Env;
use crate::error::{CanonError, CanonErrorKind};
use crate::resolve::{unresolved_to_ref, NameKind};

/// Resolves one standalone pattern (a `let` binding's LHS, a `case` arm's
/// pattern) — nothing else is bound alongside it, so duplicate-binding
/// checking only needs to span this one pattern's own sub-patterns.
pub fn resolve_pattern(
    env: &mut Env,
    pattern: &Spanned<Pattern>,
    errors: &mut Vec<CanonError>,
) -> Spanned<CPattern> {
    let mut bound = HashSet::new();
    resolve_inner(env, pattern, &mut bound, errors)
}

/// Resolves a group of patterns that bind names *together*, sharing one
/// duplicate-binding check across all of them — a lambda's parameter list is
/// the motivating case (`\x x -> ...` must be rejected even though `x` never
/// repeats within either individual parameter pattern).
pub fn resolve_pattern_group(
    env: &mut Env,
    patterns: &[Spanned<Pattern>],
    errors: &mut Vec<CanonError>,
) -> Vec<Spanned<CPattern>> {
    let mut bound = HashSet::new();
    patterns
        .iter()
        .map(|p| resolve_inner(env, p, &mut bound, errors))
        .collect()
}

/// Resolves one pattern within a caller-threaded shared `bound` set — for a
/// `let` block, whose bindings are simultaneous/mutually recursive (matching
/// Haskell/Elm `let`), so a name repeated across *different* bindings in the
/// same block is exactly as wrong as repeating it within one pattern (unlike
/// a `do` block's sequential `<-` binds, which legitimately allow shadowing —
/// see `resolve::expr`).
pub fn resolve_pattern_shared(
    env: &mut Env,
    pattern: &Spanned<Pattern>,
    bound: &mut HashSet<String>,
    errors: &mut Vec<CanonError>,
) -> Spanned<CPattern> {
    resolve_inner(env, pattern, bound, errors)
}

fn bind_checked(
    env: &mut Env,
    name: &str,
    bound: &mut HashSet<String>,
    span: knot_syntax::span::Span,
    errors: &mut Vec<CanonError>,
) {
    if !bound.insert(name.to_string()) {
        errors.push(CanonError::new(
            CanonErrorKind::DuplicateBindingInPattern(name.to_string()),
            span,
        ));
    }
    env.bind_local(name);
}

fn resolve_inner(
    env: &mut Env,
    pattern: &Spanned<Pattern>,
    bound: &mut HashSet<String>,
    errors: &mut Vec<CanonError>,
) -> Spanned<CPattern> {
    let span = pattern.span;
    let node = match &pattern.node {
        Pattern::Wildcard(name) => CPattern::Wildcard(name.clone()),
        Pattern::Var(name) => {
            bind_checked(env, name, bound, span, errors);
            CPattern::Var(name.clone())
        }
        Pattern::Literal(lit) => CPattern::Literal(lit.clone()),
        Pattern::Ctor(name, args) => {
            let (r, info) = match env.resolve_ctor(name) {
                Ok(ok) => ok,
                Err(kind) => (
                    unresolved_to_ref(kind, name, NameKind::Ctor, span, errors),
                    None,
                ),
            };
            if let Some(info) = &info {
                if info.arity != args.len() {
                    errors.push(CanonError::new(
                        CanonErrorKind::ConstructorArityMismatch {
                            name: name.clone(),
                            expected: info.arity,
                            found: args.len(),
                        },
                        span,
                    ));
                }
            }
            let cargs = args
                .iter()
                .map(|a| resolve_inner(env, a, bound, errors))
                .collect();
            CPattern::Ctor(r, cargs)
        }
        Pattern::Tuple(items) => CPattern::Tuple(
            items
                .iter()
                .map(|p| resolve_inner(env, p, bound, errors))
                .collect(),
        ),
        Pattern::Cons(head, tail) => CPattern::Cons(
            Box::new(resolve_inner(env, head, bound, errors)),
            Box::new(resolve_inner(env, tail, bound, errors)),
        ),
        Pattern::Nil => CPattern::Nil,
        Pattern::As(inner, alias) => {
            let cinner = resolve_inner(env, inner, bound, errors);
            bind_checked(env, alias, bound, span, errors);
            CPattern::As(Box::new(cinner), alias.clone())
        }
        Pattern::Unit => CPattern::Unit,
    };
    Spanned::new(span, node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use knot_syntax::ast::pattern::PatternLiteral;
    use knot_syntax::span::Span;

    fn sp(p: Pattern) -> Spanned<Pattern> {
        Spanned::new(Span::new(0, 1), p)
    }

    #[test]
    fn var_pattern_binds_locally() {
        let mut env = Env::for_decls();
        env.push_scope();
        let mut errors = Vec::new();
        let c = resolve_pattern(&mut env, &sp(Pattern::Var("x".to_string())), &mut errors);
        assert!(errors.is_empty());
        assert_eq!(c.node, CPattern::Var("x".to_string()));
        assert_eq!(
            env.resolve_value("x").ok(),
            Some(crate::ast::Ref::Local("x".to_string()))
        );
    }

    #[test]
    fn duplicate_var_within_one_tuple_pattern_is_an_error() {
        let mut env = Env::for_decls();
        env.push_scope();
        let mut errors = Vec::new();
        let pat = sp(Pattern::Tuple(vec![
            sp(Pattern::Var("x".to_string())),
            sp(Pattern::Var("x".to_string())),
        ]));
        resolve_pattern(&mut env, &pat, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            errors[0].kind,
            CanonErrorKind::DuplicateBindingInPattern(_)
        ));
    }

    #[test]
    fn duplicate_var_across_a_pattern_group_is_an_error() {
        // \x x -> ... : each param is individually fine, but the pair isn't.
        let mut env = Env::for_decls();
        env.push_scope();
        let mut errors = Vec::new();
        let params = vec![
            sp(Pattern::Var("x".to_string())),
            sp(Pattern::Var("x".to_string())),
        ];
        resolve_pattern_group(&mut env, &params, &mut errors);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn named_hole_is_never_bound() {
        let mut env = Env::for_decls();
        env.push_scope();
        let mut errors = Vec::new();
        resolve_pattern(
            &mut env,
            &sp(Pattern::Wildcard(Some("debugValue".to_string()))),
            &mut errors,
        );
        assert!(errors.is_empty());
        assert!(env.resolve_value("_debugValue").is_err());
    }

    #[test]
    fn builtin_ctor_arity_checked() {
        let mut env = Env::for_decls();
        env.push_scope();
        let mut errors = Vec::new();
        // `Some` takes exactly 1 field; giving it 2 is a structural error that
        // needs no type information to catch.
        let pat = sp(Pattern::Ctor(
            "Some".to_string(),
            vec![
                sp(Pattern::Var("a".to_string())),
                sp(Pattern::Var("b".to_string())),
            ],
        ));
        resolve_pattern(&mut env, &pat, &mut errors);
        assert!(matches!(
            errors[0].kind,
            CanonErrorKind::ConstructorArityMismatch { .. }
        ));
    }

    #[test]
    fn unknown_ctor_pattern_is_an_error() {
        let mut env = Env::for_decls();
        env.push_scope();
        let mut errors = Vec::new();
        resolve_pattern(
            &mut env,
            &sp(Pattern::Ctor("Nonexistent".to_string(), vec![])),
            &mut errors,
        );
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            errors[0].kind,
            CanonErrorKind::UnboundConstructor(_)
        ));
    }

    #[test]
    fn string_literal_pattern_passes_through() {
        let mut env = Env::for_decls();
        env.push_scope();
        let mut errors = Vec::new();
        let c = resolve_pattern(
            &mut env,
            &sp(Pattern::Literal(PatternLiteral::Str("hi".to_string()))),
            &mut errors,
        );
        assert!(errors.is_empty());
        assert_eq!(
            c.node,
            CPattern::Literal(PatternLiteral::Str("hi".to_string()))
        );
    }
}
