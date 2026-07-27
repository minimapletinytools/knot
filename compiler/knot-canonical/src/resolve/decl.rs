//! `Decl`/`Module` -> `CDecl`/`CModule`: the entry point that ties every other
//! `resolve::*` module together.
//!
//! Top-level names are collected in a first pass over *every* declaration
//! before any declaration's body is resolved, in `collect_top_level` — this is
//! what lets top-level bindings (and ADT constructors) be mutually recursive
//! and referenced regardless of source order, matching a whole module's `let`-
//! like semantics (spec §1's laziness makes this sound the same way it makes
//! recursive `let` bindings sound).
//!
//! Duplicate top-level names are *not* re-checked here — that's already a
//! post-parse check in `knot_syntax::validate` (tuple arity's sibling check),
//! so both entry points below assume their input already passed it (which
//! `knot_syntax::parse` already runs automatically; a caller using
//! `parse_decls` directly, as `knot-syntax`'s own corpus harness does, should
//! run `knot_syntax::validate::validate_module` itself first).

use std::collections::HashSet;

use knot_syntax::ast::decl::{Decl, FnDef, InstanceDecl, Module};
use knot_syntax::span::{Span, Spanned};

use crate::ast::{CAnnotation, CDecl, CFnDef, CInstanceDecl, CModule};
use crate::env::{Env, ModuleRegistry};
use crate::error::{CanonError, CanonErrorKind};
use crate::prelude;
use crate::resolve::expr::resolve_expr;
use crate::resolve::pattern::resolve_pattern_group;
use crate::resolve::ty;

/// Full module, with a real import list to check qualifiers against — see
/// `env.rs`'s module docs on strict vs. permissive qualifier resolution.
pub fn resolve_module(
    module: &Module,
    registry: Option<&dyn ModuleRegistry>,
) -> (CModule, Vec<CanonError>) {
    let mut env = Env::for_module(&module.imports, registry);
    let mut errors = Vec::new();
    collect_top_level(&mut env, &module.decls);
    let decls = module
        .decls
        .iter()
        .map(|d| resolve_decl(&mut env, d, &mut errors))
        .collect();
    (
        CModule {
            name: module.name.clone(),
            exposing: module.exposing.clone(),
            imports: module.imports.clone(),
            decls,
        },
        errors,
    )
}

/// A bare declaration list with no module header/imports (`parse_decls`'s
/// world) — qualified references are trusted at face value; see `env.rs`.
pub fn resolve_decls(decls: &[Spanned<Decl>]) -> (Vec<Spanned<CDecl>>, Vec<CanonError>) {
    let mut env = Env::for_decls();
    let mut errors = Vec::new();
    collect_top_level(&mut env, decls);
    let cdecls = decls
        .iter()
        .map(|d| resolve_decl(&mut env, d, &mut errors))
        .collect();
    (cdecls, errors)
}

fn collect_top_level(env: &mut Env, decls: &[Spanned<Decl>]) {
    for decl in decls {
        match &decl.node {
            Decl::Fn(fndef) => env.declare_top_level_value(&fndef.name),
            Decl::TypeAlias(name, _, _) => env.declare_top_level_type(name),
            Decl::TypeDecl(name, _, variants) => {
                env.declare_top_level_type(name);
                for (ctor_name, arg_types) in variants {
                    env.declare_ctor(ctor_name, arg_types.len(), name);
                }
            }
            // An instance declaration introduces no new top-level name of its
            // own -- its methods (`(==)`, ...) are interface methods dispatched
            // later via dictionary passing, never referenced as a plain `Var`.
            Decl::Instance(_) => {}
        }
    }
}

fn resolve_decl(
    env: &mut Env,
    decl: &Spanned<Decl>,
    errors: &mut Vec<CanonError>,
) -> Spanned<CDecl> {
    let span = decl.span;
    let node = match &decl.node {
        Decl::Fn(fndef) => CDecl::Fn(resolve_fndef(env, fndef, errors)),
        Decl::TypeAlias(name, params, ty) => {
            let mut used = HashSet::new();
            ty::collect_type_vars(ty, &mut used);
            check_unbound_type_vars(&used, params, name, span, errors);
            let cty = ty::resolve_type(env, ty, span, errors);
            CDecl::TypeAlias(name.clone(), params.clone(), cty)
        }
        Decl::TypeDecl(name, params, variants) => {
            let mut used = HashSet::new();
            for (_, arg_types) in variants {
                for t in arg_types {
                    ty::collect_type_vars(t, &mut used);
                }
            }
            check_unbound_type_vars(&used, params, name, span, errors);
            let cvariants = variants
                .iter()
                .map(|(vname, arg_types)| {
                    let ctys = arg_types
                        .iter()
                        .map(|t| ty::resolve_type(env, t, span, errors))
                        .collect();
                    (vname.clone(), ctys)
                })
                .collect();
            CDecl::TypeDecl(name.clone(), params.clone(), cvariants)
        }
        Decl::Instance(inst) => CDecl::Instance(resolve_instance(env, inst, span, errors)),
    };
    Spanned::new(span, node)
}

fn check_unbound_type_vars(
    used: &HashSet<String>,
    params: &[String],
    decl_name: &str,
    span: Span,
    errors: &mut Vec<CanonError>,
) {
    let declared: HashSet<&String> = params.iter().collect();
    let mut unbound: Vec<&String> = used.iter().filter(|v| !declared.contains(v)).collect();
    unbound.sort();
    for var in unbound {
        errors.push(CanonError::new(
            CanonErrorKind::UnboundTypeVariable {
                name: var.clone(),
                decl: decl_name.to_string(),
            },
            span,
        ));
    }
}

/// Annotations are resolved *before* the function's own parameter scope opens:
/// they're metadata about the binding, evaluated at graph-construction time
/// (spec §10) in the enclosing scope, not inside the function body's scope —
/// an unravel function given as an annotation value manages its own scope via
/// the ordinary `Lambda` case instead.
fn resolve_fndef(env: &mut Env, fndef: &FnDef, errors: &mut Vec<CanonError>) -> CFnDef {
    let signature = fndef.signature.as_ref().map(|sig| {
        Spanned::new(
            sig.span,
            ty::resolve_signature(env, &sig.node, sig.span, errors),
        )
    });
    let annotations = fndef
        .annotations
        .iter()
        .map(|a| CAnnotation {
            key: a.key.clone(),
            value: resolve_expr(env, &a.value, errors),
        })
        .collect();
    env.push_scope();
    let params = resolve_pattern_group(env, &fndef.params, errors);
    let body = resolve_expr(env, &fndef.body, errors);
    env.pop_scope();
    CFnDef {
        name: fndef.name.clone(),
        signature,
        params,
        body,
        annotations,
    }
}

fn resolve_instance(
    env: &mut Env,
    inst: &InstanceDecl,
    span: Span,
    errors: &mut Vec<CanonError>,
) -> CInstanceDecl {
    if !prelude::is_builtin_interface(&inst.interface) {
        errors.push(CanonError::new(
            CanonErrorKind::UnknownInterface(inst.interface.clone()),
            span,
        ));
    }
    let constraints = inst
        .constraints
        .iter()
        .map(|c| ty::resolve_constraint(c, span, errors))
        .collect();
    let target = ty::resolve_type(env, &inst.target, span, errors);
    let methods = inst
        .methods
        .iter()
        .map(|m| resolve_fndef(env, m, errors))
        .collect();
    CInstanceDecl {
        interface: inst.interface.clone(),
        constraints,
        target,
        methods,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knot_syntax::ast::decl::Exposing;

    fn parse(src: &str) -> Vec<Spanned<Decl>> {
        let mut state = knot_syntax::ParseState::new(src);
        let decls = state.parse_decls().unwrap();
        assert!(state.is_eof());
        decls
    }

    #[test]
    fn self_contained_module_resolves_cleanly() {
        let src = "\
type Shape
  = Circle Float
  | Rectangle Float Float

area :: Shape -> Float
area shape =
  case shape of
    Circle r      -> 3.14159 * r * r
    Rectangle w h -> w * h
";
        let decls = parse(src);
        let (_cdecls, errors) = resolve_decls(&decls);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn top_level_bindings_are_mutually_recursive_regardless_of_order() {
        let src = "\
isOdd n = if n == 0 then False else isEven (n - 1)
isEven n = if n == 0 then True else isOdd (n - 1)
";
        let decls = parse(src);
        let (_cdecls, errors) = resolve_decls(&decls);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn ctor_usable_before_its_type_decl_in_source_order() {
        let src = "\
originShape = Circle 1.0

type Shape = Circle Float
";
        let decls = parse(src);
        let (_cdecls, errors) = resolve_decls(&decls);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn unbound_type_variable_in_adt_is_an_error() {
        let src = "type Option a\n  = Some b\n  | None\n";
        let decls = parse(src);
        let (_cdecls, errors) = resolve_decls(&decls);
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            errors[0].kind,
            CanonErrorKind::UnboundTypeVariable { .. }
        ));
    }

    #[test]
    fn unbound_type_variable_in_alias_is_an_error() {
        let src = "type alias Box a = { value : b }\n";
        let decls = parse(src);
        let (_cdecls, errors) = resolve_decls(&decls);
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            errors[0].kind,
            CanonErrorKind::UnboundTypeVariable { .. }
        ));
    }

    #[test]
    fn instance_declaration_resolves_and_checks_interface_name() {
        let src = "\
type Shape = Circle Float

instance Eq Shape where
  (==) a b =
    case (a, b) of
      (Circle r1, Circle r2) -> r1 == r2
";
        let decls = parse(src);
        let (_cdecls, errors) = resolve_decls(&decls);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn instance_with_unknown_interface_is_an_error() {
        let src = "\
type Shape = Circle Float

instance Frobnicable Shape where
  frob a = a
";
        let decls = parse(src);
        let (_cdecls, errors) = resolve_decls(&decls);
        assert!(errors
            .iter()
            .any(|e| matches!(e.kind, CanonErrorKind::UnknownInterface(_))));
    }

    #[test]
    fn constrained_signature_with_unknown_interface_is_an_error() {
        let src = "myMax :: Frobnicable a => a -> a -> a\nmyMax x y = x\n";
        let decls = parse(src);
        let (_cdecls, errors) = resolve_decls(&decls);
        assert!(errors
            .iter()
            .any(|e| matches!(e.kind, CanonErrorKind::UnknownInterface(_))));
    }

    #[test]
    fn annotation_value_can_reference_a_sibling_top_level_binding() {
        let src = "\
@{ unravel = myCustomUnraveler }
myFunc :: Int -> Int
myFunc x = x + 1

myCustomUnraveler = 0
";
        let decls = parse(src);
        let (_cdecls, errors) = resolve_decls(&decls);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn qualified_reference_is_trusted_at_face_value_with_no_import_list() {
        // parse_decls (bare snippets) has no import list at all, matching the
        // existing syntax corpus's non-`modules/` fixtures -- Map.fromList must
        // still resolve even though nothing "imported" Map here.
        let src = "scores = Map.fromList [(\"a\", 1)]\n";
        let decls = parse(src);
        let (_cdecls, errors) = resolve_decls(&decls);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn full_module_checks_qualifier_against_real_imports() {
        let src = "\
module Example exposing (..)

import List

double xs = List.map (\\x -> x * 2) xs
";
        let module = knot_syntax::parse(src).unwrap();
        let (_cmodule, errors) = resolve_module(&module, None);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn full_module_rejects_an_unknown_qualifier() {
        let src = "\
module Example exposing (..)

import List

double xs = Sting.reverse xs
";
        let module = knot_syntax::parse(src).unwrap();
        let (_cmodule, errors) = resolve_module(&module, None);
        assert!(errors
            .iter()
            .any(|e| matches!(e.kind, CanonErrorKind::UnknownQualifier(_))));
    }

    #[test]
    fn full_module_resolves_unqualified_exposed_name() {
        let src = "\
module Example exposing (..)

import String exposing (length, concat)

shout s = length s
";
        let module = knot_syntax::parse(src).unwrap();
        let (_cmodule, errors) = resolve_module(&module, None);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn full_module_permissively_resolves_via_wildcard_exposing_with_no_registry() {
        let src = "\
module Example exposing (..)

import String exposing (..)

shout s = length (reverse s)
";
        let module = knot_syntax::parse(src).unwrap();
        let (_cmodule, errors) = resolve_module(&module, None);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn module_name_and_exposing_survive_unchanged() {
        let src = "module Geometry exposing (..)\n\ntype alias Point = { x : Float, y : Float }\n";
        let module = knot_syntax::parse(src).unwrap();
        let (cmodule, errors) = resolve_module(&module, None);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(cmodule.name, vec!["Geometry".to_string()]);
        assert_eq!(cmodule.exposing, Exposing::All);
    }
}
