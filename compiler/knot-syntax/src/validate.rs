//! Post-parse structural checks that don't need type information: tuple arity
//! ≤ 3, and duplicate top-level bindings (this is how "no multi-clause
//! functions" gets enforced — it's inherently a whole-module check across
//! sibling declarations, not something a single grammar production can catch
//! on its own). Mirrors what Elm actually does for tuple arity: parse
//! permissively, reject with a clear message afterward.

use crate::ast::decl::{Decl, FnDef, InstanceDecl, Module};
use crate::ast::expr::{DoStmt, Expr};
use crate::ast::pattern::Pattern;
use crate::ast::ty::Type;
use crate::span::Span;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    pub message: String,
    pub span: Span,
}

pub fn validate_module(module: &Module) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    check_duplicate_fn_names(module, &mut errors);
    for decl in &module.decls {
        check_decl_tuple_arity(&decl.node, decl.span, &mut errors);
    }
    errors
}

fn check_duplicate_fn_names(module: &Module, errors: &mut Vec<ValidationError>) {
    let mut seen: HashMap<&str, Span> = HashMap::new();
    for decl in &module.decls {
        if let Decl::Fn(f) = &decl.node {
            if seen.contains_key(f.name.as_str()) {
                errors.push(ValidationError {
                    message: format!(
                        "`{}` is defined more than once -- Knot doesn't support multi-clause \
                         function definitions; use `case` instead",
                        f.name
                    ),
                    span: decl.span,
                });
            } else {
                seen.insert(&f.name, decl.span);
            }
        }
    }
}

fn check_decl_tuple_arity(decl: &Decl, span: Span, errors: &mut Vec<ValidationError>) {
    match decl {
        Decl::Fn(f) => check_fn_def_tuple_arity(f, span, errors),
        Decl::TypeAlias(_, _, ty) => check_type_tuple_arity(ty, span, errors),
        Decl::TypeDecl(_, _, variants) => {
            for (_, args) in variants {
                for ty in args {
                    check_type_tuple_arity(ty, span, errors);
                }
            }
        }
        Decl::Instance(InstanceDecl {
            target, methods, ..
        }) => {
            check_type_tuple_arity(target, span, errors);
            for m in methods {
                check_fn_def_tuple_arity(m, span, errors);
            }
        }
    }
}

fn check_fn_def_tuple_arity(f: &FnDef, fallback_span: Span, errors: &mut Vec<ValidationError>) {
    if let Some(sig) = &f.signature {
        check_type_tuple_arity(&sig.node.ty, sig.span, errors);
    }
    for p in &f.params {
        check_pattern_tuple_arity(&p.node, p.span, errors);
    }
    let _ = fallback_span; // body/param spans are always precise; kept for symmetry with the Type case
    check_expr_tuple_arity(&f.body.node, f.body.span, errors);
}

/// `Type` carries no per-node span (see `knot-ast-parser-plan.md` §4), so every
/// violation found while walking one is attributed to `span` — the span of the
/// enclosing signature/alias/variant-field-list as a whole, not the specific
/// offending tuple within it. A coarser location, not a missing one.
fn check_type_tuple_arity(ty: &Type, span: Span, errors: &mut Vec<ValidationError>) {
    match ty {
        Type::Tuple(elems) => {
            if elems.len() > 3 {
                errors.push(ValidationError {
                    message: format!(
                        "tuple type has {} elements; Knot caps tuples at 3 (pairs and triples only)",
                        elems.len()
                    ),
                    span,
                });
            }
            for e in elems {
                check_type_tuple_arity(e, span, errors);
            }
        }
        Type::Named(_, args) => {
            for a in args {
                check_type_tuple_arity(a, span, errors);
            }
        }
        Type::Fn(a, b) => {
            check_type_tuple_arity(a, span, errors);
            check_type_tuple_arity(b, span, errors);
        }
        Type::Record(fields, _) => {
            for (_, t) in fields {
                check_type_tuple_arity(t, span, errors);
            }
        }
        Type::Var(_) | Type::Unit => {}
    }
}

fn check_pattern_tuple_arity(pat: &Pattern, span: Span, errors: &mut Vec<ValidationError>) {
    match pat {
        Pattern::Tuple(elems) => {
            if elems.len() > 3 {
                errors.push(ValidationError {
                    message: format!(
                        "tuple pattern has {} elements; Knot caps tuples at 3",
                        elems.len()
                    ),
                    span,
                });
            }
            for e in elems {
                check_pattern_tuple_arity(&e.node, e.span, errors);
            }
        }
        Pattern::Ctor(_, args) => {
            for a in args {
                check_pattern_tuple_arity(&a.node, a.span, errors);
            }
        }
        Pattern::Cons(h, t) => {
            check_pattern_tuple_arity(&h.node, h.span, errors);
            check_pattern_tuple_arity(&t.node, t.span, errors);
        }
        Pattern::As(p, _) => check_pattern_tuple_arity(&p.node, p.span, errors),
        Pattern::Wildcard(_)
        | Pattern::Var(_)
        | Pattern::Literal(_)
        | Pattern::Nil
        | Pattern::Unit => {}
    }
}

fn check_expr_tuple_arity(expr: &Expr, span: Span, errors: &mut Vec<ValidationError>) {
    match expr {
        Expr::Tuple(elems) => {
            if elems.len() > 3 {
                errors.push(ValidationError {
                    message: format!("tuple has {} elements; Knot caps tuples at 3", elems.len()),
                    span,
                });
            }
            for e in elems {
                check_expr_tuple_arity(&e.node, e.span, errors);
            }
        }
        Expr::App(f, a) => {
            check_expr_tuple_arity(&f.node, f.span, errors);
            check_expr_tuple_arity(&a.node, a.span, errors);
        }
        Expr::BinOp(_, l, r) => {
            check_expr_tuple_arity(&l.node, l.span, errors);
            check_expr_tuple_arity(&r.node, r.span, errors);
        }
        Expr::Negate(e) => check_expr_tuple_arity(&e.node, e.span, errors),
        Expr::If(c, t, el) => {
            check_expr_tuple_arity(&c.node, c.span, errors);
            check_expr_tuple_arity(&t.node, t.span, errors);
            check_expr_tuple_arity(&el.node, el.span, errors);
        }
        Expr::Let(bindings, body) => {
            for (p, v) in bindings {
                check_pattern_tuple_arity(&p.node, p.span, errors);
                check_expr_tuple_arity(&v.node, v.span, errors);
            }
            check_expr_tuple_arity(&body.node, body.span, errors);
        }
        Expr::Case(scrutinee, arms) => {
            check_expr_tuple_arity(&scrutinee.node, scrutinee.span, errors);
            for (p, b) in arms {
                check_pattern_tuple_arity(&p.node, p.span, errors);
                check_expr_tuple_arity(&b.node, b.span, errors);
            }
        }
        Expr::Do(stmts, final_expr) => {
            for s in stmts {
                match s {
                    DoStmt::Bind(p, v) => {
                        check_pattern_tuple_arity(&p.node, p.span, errors);
                        check_expr_tuple_arity(&v.node, v.span, errors);
                    }
                    DoStmt::Expr(e) => check_expr_tuple_arity(&e.node, e.span, errors),
                }
            }
            check_expr_tuple_arity(&final_expr.node, final_expr.span, errors);
        }
        Expr::List(elems) => {
            for e in elems {
                check_expr_tuple_arity(&e.node, e.span, errors);
            }
        }
        Expr::Record(fields) => {
            for (_, v) in fields {
                check_expr_tuple_arity(&v.node, v.span, errors);
            }
        }
        Expr::RecordUpdate(base, fields) => {
            check_expr_tuple_arity(&base.node, base.span, errors);
            for (_, v) in fields {
                check_expr_tuple_arity(&v.node, v.span, errors);
            }
        }
        Expr::FieldAccess(e, _) => check_expr_tuple_arity(&e.node, e.span, errors),
        Expr::Annotated(_, e) => check_expr_tuple_arity(&e.node, e.span, errors),
        Expr::Lambda(params, body) => {
            for p in params {
                check_pattern_tuple_arity(&p.node, p.span, errors);
            }
            check_expr_tuple_arity(&body.node, body.span, errors);
        }
        Expr::IntLit(_)
        | Expr::FloatLit(_)
        | Expr::StringLit(_)
        | Expr::Unit
        | Expr::Var(_)
        | Expr::Ctor(_)
        | Expr::Hole => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate_src(src: &str) -> Vec<ValidationError> {
        let module_src = format!("module Test exposing (..)\n\n{src}");
        let module = crate::ParseState::new(&module_src)
            .parse_module()
            .expect("fixture failed to parse");
        validate_module(&module)
    }

    #[test]
    fn no_errors_for_well_formed_module() {
        assert!(validate_src("x = (1, 2, 3)").is_empty());
    }

    #[test]
    fn duplicate_top_level_binding_is_rejected() {
        let errors = validate_src("f 0 = 1\nf n = n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("defined more than once"));
    }

    #[test]
    fn tuple_expr_arity_over_three_is_rejected() {
        let errors = validate_src("x = (1, 2, 3, 4)");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("Knot caps tuples at 3"));
    }

    #[test]
    fn tuple_type_arity_over_three_is_rejected() {
        let errors = validate_src("quad :: (Int, Int, Int, Int)\nquad = (1, 2, 3, 4)");
        // Both the signature's tuple type AND the body's tuple value are over
        // arity here, so two independent violations are expected.
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn nested_tuple_arity_violation_is_found() {
        let errors = validate_src("x = [(1, 2, 3, 4)]");
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn tuple_pattern_arity_over_three_is_rejected() {
        let errors = validate_src("f (a, b, c, d) = a");
        assert_eq!(errors.len(), 1);
    }
}
