//! Type alias expansion: a `type alias` has no identity of its own — it's
//! purely a name for another type — so every reference to one is replaced
//! with what it actually stands for. Runs as a whole-module post-pass,
//! after ordinary name resolution (`resolve::decl`'s own `.map()` over every
//! declaration): by that point every `CType::Named` already carries a
//! fully-resolved `Ref`, so this pass only needs to decide *which*
//! `Ref::TopLevel`s name an alias (as opposed to a genuine nominal ADT —
//! both share the same namespace, see `env::Env::resolve_type`) and
//! substitute accordingly. It never needs to re-resolve a name itself.
//!
//! **Cycles are a hard error, unlike a recursive ADT.** `type List a = Cons
//! a (List a) | Nil` is fine — a constructor is a real level of indirection
//! (a boxed/tagged value), so the recursion terminates at runtime, one
//! constructor application at a time. An alias is pure substitution with no
//! such indirection: `type alias Bad = Bad` (or a longer mutual cycle
//! through several aliases) has no finite expansion at all, so it's
//! detected and reported rather than expanded into an infinite loop.
//!
//! **Only local aliases** (`Ref::TopLevel`) are expanded — an *imported*
//! alias's own definition isn't available here at all (this crate has no
//! project-wide module loader yet, per `lib.rs`'s own doc comment), so
//! there's nothing to substitute in for one; it's left as an opaque nominal
//! reference, same as it was before this pass existed. A real gap once
//! cross-module linking exists, not one this pass can close on its own.

use std::collections::HashMap;

use knot_syntax::span::{Span, Spanned};

use crate::ast::{CDecl, CFnDef, CType, Ref};
use crate::error::{CanonError, CanonErrorKind};

/// One alias's own declaration: its parameters and body. `params.len()` is
/// this alias's arity — every use site's own argument list is checked
/// against it (`substitute`'s own arity-mismatch check) before substituting.
#[derive(Debug, Clone)]
struct AliasDef {
    params: Vec<String>,
    body: CType,
}

/// Expands every local type alias reference in `decls` in place. A no-op
/// (not even a wasted pass) when the module declares no aliases at all.
pub fn expand_aliases(decls: &mut [Spanned<CDecl>], errors: &mut Vec<CanonError>) {
    let mut raw: HashMap<String, (Span, AliasDef)> = HashMap::new();
    for d in decls.iter() {
        if let CDecl::TypeAlias(name, params, body) = &d.node {
            raw.insert(
                name.clone(),
                (
                    d.span,
                    AliasDef {
                        params: params.clone(),
                        body: body.clone(),
                    },
                ),
            );
        }
    }
    if raw.is_empty() {
        return;
    }

    let defs: HashMap<String, AliasDef> = raw
        .iter()
        .map(|(k, (_, def))| (k.clone(), def.clone()))
        .collect();
    let Some(order) = topo_order(&defs, &raw, errors) else {
        // A cycle was found and reported -- nothing sound to expand, and
        // substituting anyway would either loop forever or paper over the
        // error with a made-up shape. Leave everything as name-resolution
        // produced it; the reported error is what matters from here.
        return;
    };

    // Expand each alias's own body in dependency order, so a later alias
    // only ever needs to substitute an *already fully expanded* earlier
    // one, never a raw, alias-referencing body -- one pass per alias, not
    // one pass per level of nesting.
    let mut expanded: HashMap<String, AliasDef> = HashMap::new();
    for name in &order {
        let (span, def) = &raw[name];
        let body = substitute(&def.body, &expanded, errors, *span);
        expanded.insert(
            name.clone(),
            AliasDef {
                params: def.params.clone(),
                body,
            },
        );
    }

    // Rewrite every CType in the module, including each alias's own stored
    // body -- so anything looking at `CDecl::TypeAlias` directly afterward
    // also sees a fully-expanded shape, not just everyone else's uses of it.
    for d in decls.iter_mut() {
        let span = d.span;
        rewrite_decl(&mut d.node, &expanded, errors, span);
    }
}

/// Every known alias name `ty` refers to, one level of `Named` at a time —
/// the dependency edges `topo_order` needs. Doesn't need to recurse into an
/// already-*found* alias's own body (that's `topo_order`'s own job, walking
/// the dependency graph itself); this only ever looks at `ty`'s own shape.
fn collect_alias_refs(ty: &CType, defs: &HashMap<String, AliasDef>, out: &mut Vec<String>) {
    match ty {
        CType::Named(Ref::TopLevel(name), args) => {
            if defs.contains_key(name) {
                out.push(name.clone());
            }
            for a in args {
                collect_alias_refs(a, defs, out);
            }
        }
        CType::Named(_, args) => {
            for a in args {
                collect_alias_refs(a, defs, out);
            }
        }
        CType::Var(_) | CType::Unit => {}
        CType::Fn(a, b) => {
            collect_alias_refs(a, defs, out);
            collect_alias_refs(b, defs, out);
        }
        CType::Tuple(ts) => {
            for t in ts {
                collect_alias_refs(t, defs, out);
            }
        }
        CType::Record(fields, _) => {
            for (_, t) in fields {
                collect_alias_refs(t, defs, out);
            }
        }
    }
}

/// A dependency-ordered (dependencies before dependents) list of every alias
/// name in `defs`, or `None` if any alias participates in a cycle (each one
/// on a cycle gets its own `CyclicTypeAlias` error, using that alias's own
/// declaration span from `spans`).
fn topo_order(
    defs: &HashMap<String, AliasDef>,
    spans: &HashMap<String, (Span, AliasDef)>,
    errors: &mut Vec<CanonError>,
) -> Option<Vec<String>> {
    #[derive(PartialEq, Eq, Clone, Copy)]
    enum Mark {
        Visiting,
        Done,
    }

    fn visit(
        name: &str,
        defs: &HashMap<String, AliasDef>,
        marks: &mut HashMap<String, Mark>,
        order: &mut Vec<String>,
        cyclic: &mut std::collections::HashSet<String>,
    ) {
        match marks.get(name) {
            Some(Mark::Done) => return,
            Some(Mark::Visiting) => {
                cyclic.insert(name.to_string());
                return;
            }
            None => {}
        }
        marks.insert(name.to_string(), Mark::Visiting);
        let mut deps = Vec::new();
        collect_alias_refs(&defs[name].body, defs, &mut deps);
        for dep in deps {
            visit(&dep, defs, marks, order, cyclic);
            if cyclic.contains(&dep) {
                cyclic.insert(name.to_string());
            }
        }
        marks.insert(name.to_string(), Mark::Done);
        order.push(name.to_string());
    }

    let mut marks = HashMap::new();
    let mut order = Vec::new();
    let mut cyclic = std::collections::HashSet::new();
    let mut names: Vec<&String> = defs.keys().collect();
    names.sort();
    for name in names {
        visit(name, defs, &mut marks, &mut order, &mut cyclic);
    }

    if cyclic.is_empty() {
        Some(order)
    } else {
        let mut cyclic: Vec<String> = cyclic.into_iter().collect();
        cyclic.sort();
        for name in cyclic {
            let span = spans[&name].0;
            errors.push(CanonError::new(CanonErrorKind::CyclicTypeAlias(name), span));
        }
        None
    }
}

/// Replaces every `CType::Var` named in `mapping` — an alias's own body,
/// substituting its declared parameters for the real type arguments at one
/// particular use site. Plain structural recursion otherwise, except for a
/// record's own row-extension slot (see `substitute_record_ext`) — that one
/// isn't an ordinary `CType` position, since `CType::Record`'s extension is
/// just a variable *name*, not a nested `CType`, so substituting a concrete
/// type into it (`type alias Selectable a = { a | isSelected : Bool }`
/// applied to `Selectable Foo`) needs its own merge logic instead of the
/// plain `CType::Var` case above.
fn substitute_vars(
    ty: &CType,
    mapping: &HashMap<String, CType>,
    alias_name: &str,
    errors: &mut Vec<CanonError>,
    span: Span,
) -> CType {
    match ty {
        CType::Var(v) => mapping.get(v).cloned().unwrap_or_else(|| ty.clone()),
        CType::Named(r, args) => CType::Named(
            r.clone(),
            args.iter()
                .map(|a| substitute_vars(a, mapping, alias_name, errors, span))
                .collect(),
        ),
        CType::Fn(a, b) => CType::Fn(
            Box::new(substitute_vars(a, mapping, alias_name, errors, span)),
            Box::new(substitute_vars(b, mapping, alias_name, errors, span)),
        ),
        CType::Tuple(ts) => CType::Tuple(
            ts.iter()
                .map(|t| substitute_vars(t, mapping, alias_name, errors, span))
                .collect(),
        ),
        CType::Record(fields, ext) => {
            let own_fields: Vec<(String, CType)> = fields
                .iter()
                .map(|(n, t)| {
                    (
                        n.clone(),
                        substitute_vars(t, mapping, alias_name, errors, span),
                    )
                })
                .collect();
            substitute_record_ext(own_fields, ext, mapping, alias_name, errors, span)
        }
        CType::Unit => CType::Unit,
    }
}

/// Resolves a record's own row-extension slot once its declared fields
/// (`own_fields`) are already substituted. `ext` is just a variable name —
/// three things can happen once it's looked up in `mapping`:
/// - not one of the alias's own parameters (or no extension at all): left
///   untouched, exactly as before this fix existed.
/// - substituted with another still-free variable (`CType::Var`): the
///   extension is still genuinely open, just renamed to that variable —
///   e.g. a wrapping alias forwarding its own parameter along.
/// - substituted with a concrete record (`CType::Record`): the extension
///   is resolved *now* — merge that record's own fields in and adopt its
///   own extension (closed if it had none), so e.g. `Selectable Foo`
///   becomes the closed `{ name : String, isSelected : Bool }` rather than
///   staying dangling on the unsubstituted `a`. A field declared by both
///   sides is a `RecordExtensionFieldConflict`, not silently overwritten.
/// - substituted with anything else (a nominal type, tuple, function, unit
///   — none of which are record-shaped): `RecordExtensionNotARecord`, and
///   the extension is left as-is (best-effort recovery, matching this
///   file's other error-then-proceed cases) since there's no sound record
///   to produce instead.
fn substitute_record_ext(
    own_fields: Vec<(String, CType)>,
    ext: &Option<String>,
    mapping: &HashMap<String, CType>,
    alias_name: &str,
    errors: &mut Vec<CanonError>,
    span: Span,
) -> CType {
    let Some(name) = ext else {
        return CType::Record(own_fields, None);
    };
    match mapping.get(name) {
        None => CType::Record(own_fields, Some(name.clone())),
        Some(CType::Var(other)) => CType::Record(own_fields, Some(other.clone())),
        Some(CType::Record(other_fields, other_ext)) => {
            let mut merged = own_fields;
            for (field_name, field_ty) in other_fields {
                if merged.iter().any(|(n, _)| n == field_name) {
                    errors.push(CanonError::new(
                        CanonErrorKind::RecordExtensionFieldConflict {
                            alias: alias_name.to_string(),
                            field: field_name.clone(),
                        },
                        span,
                    ));
                } else {
                    merged.push((field_name.clone(), field_ty.clone()));
                }
            }
            CType::Record(merged, other_ext.clone())
        }
        Some(_) => {
            errors.push(CanonError::new(
                CanonErrorKind::RecordExtensionNotARecord {
                    alias: alias_name.to_string(),
                    param: name.clone(),
                },
                span,
            ));
            CType::Record(own_fields, Some(name.clone()))
        }
    }
}

/// Replaces every alias reference in `ty` with its (already fully expanded,
/// per `expanded`) definition, substituting the alias's own declared
/// parameters for the real arguments given at this use site. A use site
/// with the wrong number of arguments gets a `TypeAliasArityMismatch`
/// error (same spirit as `resolve::decl`'s own `ConstructorArityMismatch`)
/// — substitution still proceeds as best it can (`substitute_vars` simply
/// leaves an unmatched parameter as a free variable), since leaving the
/// rest of the tree unresolved would hide other, unrelated errors.
fn substitute(
    ty: &CType,
    expanded: &HashMap<String, AliasDef>,
    errors: &mut Vec<CanonError>,
    span: Span,
) -> CType {
    match ty {
        CType::Named(Ref::TopLevel(name), args) if expanded.contains_key(name) => {
            let def = &expanded[name];
            let sub_args: Vec<CType> = args
                .iter()
                .map(|a| substitute(a, expanded, errors, span))
                .collect();
            if sub_args.len() != def.params.len() {
                errors.push(CanonError::new(
                    CanonErrorKind::TypeAliasArityMismatch {
                        name: name.clone(),
                        expected: def.params.len(),
                        found: sub_args.len(),
                    },
                    span,
                ));
            }
            let mapping: HashMap<String, CType> =
                def.params.iter().cloned().zip(sub_args).collect();
            substitute_vars(&def.body, &mapping, name, errors, span)
        }
        CType::Named(r, args) => CType::Named(
            r.clone(),
            args.iter()
                .map(|a| substitute(a, expanded, errors, span))
                .collect(),
        ),
        CType::Var(_) | CType::Unit => ty.clone(),
        CType::Fn(a, b) => CType::Fn(
            Box::new(substitute(a, expanded, errors, span)),
            Box::new(substitute(b, expanded, errors, span)),
        ),
        CType::Tuple(ts) => CType::Tuple(
            ts.iter()
                .map(|t| substitute(t, expanded, errors, span))
                .collect(),
        ),
        CType::Record(fields, ext) => CType::Record(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), substitute(t, expanded, errors, span)))
                .collect(),
            ext.clone(),
        ),
    }
}

fn rewrite_decl(
    decl: &mut CDecl,
    expanded: &HashMap<String, AliasDef>,
    errors: &mut Vec<CanonError>,
    span: Span,
) {
    match decl {
        CDecl::Fn(fndef) => rewrite_fndef(fndef, expanded, errors),
        CDecl::TypeAlias(_, _, ty) => *ty = substitute(ty, expanded, errors, span),
        CDecl::TypeDecl(_, _, variants) => {
            for (_, arg_types) in variants.iter_mut() {
                for t in arg_types.iter_mut() {
                    *t = substitute(t, expanded, errors, span);
                }
            }
        }
        CDecl::Instance(inst) => {
            inst.target = substitute(&inst.target, expanded, errors, span);
            for m in inst.methods.iter_mut() {
                rewrite_fndef(m, expanded, errors);
            }
        }
    }
}

fn rewrite_fndef(
    fndef: &mut CFnDef,
    expanded: &HashMap<String, AliasDef>,
    errors: &mut Vec<CanonError>,
) {
    if let Some(sig) = &mut fndef.signature {
        let span = sig.span;
        sig.node.ty = substitute(&sig.node.ty, expanded, errors, span);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::CDecl;
    use crate::resolve::decl::resolve_decls;
    use knot_syntax::ast::decl::Decl;

    fn decls(src: &str) -> Vec<Spanned<CDecl>> {
        let mut state = knot_syntax::ParseState::new(src);
        let raw: Vec<Spanned<Decl>> = state.parse_decls().unwrap();
        assert!(state.is_eof(), "leftover input: {src}");
        let (cdecls, errors) = resolve_decls(&raw);
        assert!(errors.is_empty(), "{errors:?}");
        cdecls
    }

    fn fn_sig_ty(decls: &[Spanned<CDecl>], name: &str) -> CType {
        decls
            .iter()
            .find_map(|d| match &d.node {
                CDecl::Fn(f) if f.name == name => {
                    Some(f.signature.as_ref().unwrap().node.ty.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no signed fn named {name}"))
    }

    #[test]
    fn a_non_parametric_alias_expands_to_its_own_body() {
        let cs = decls("type alias IntPair = (Int, Int)\nswap :: IntPair -> IntPair\nswap p = p\n");
        let ty = fn_sig_ty(&cs, "swap");
        match ty {
            CType::Fn(a, b) => {
                assert!(matches!(*a, CType::Tuple(_)), "expected a Tuple, got {a:?}");
                assert!(matches!(*b, CType::Tuple(_)), "expected a Tuple, got {b:?}");
            }
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn a_parametric_alias_substitutes_its_own_argument() {
        let cs = decls("type alias Pair a = (a, a)\nfirst :: Pair Int -> Int\nfirst p = 1\n");
        let ty = fn_sig_ty(&cs, "first");
        match ty {
            CType::Fn(a, _) => match *a {
                CType::Tuple(elems) => {
                    assert_eq!(elems.len(), 2);
                    for e in elems {
                        assert!(
                            matches!(e, CType::Named(Ref::Builtin(ref n), _) if n == "Int"),
                            "expected each element substituted with Int, got {e:?}"
                        );
                    }
                }
                other => panic!("expected a Tuple, got {other:?}"),
            },
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn an_alias_referencing_another_alias_is_fully_expanded() {
        let cs = decls("type alias B = Int\ntype alias A = B\nuseA :: A -> A\nuseA x = x\n");
        let ty = fn_sig_ty(&cs, "useA");
        match ty {
            CType::Fn(a, b) => {
                assert!(matches!(*a, CType::Named(Ref::Builtin(ref n), _) if n == "Int"));
                assert!(matches!(*b, CType::Named(Ref::Builtin(ref n), _) if n == "Int"));
            }
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn a_self_referential_alias_is_a_cyclic_error() {
        let mut state = knot_syntax::ParseState::new("type alias Bad = Bad\n");
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors
            .iter()
            .any(|e| matches!(&e.kind, CanonErrorKind::CyclicTypeAlias(name) if name == "Bad")));
    }

    #[test]
    fn a_mutually_cyclic_alias_pair_is_reported_without_hanging() {
        let mut state =
            knot_syntax::ParseState::new("type alias A = { x : B }\ntype alias B = { y : A }\n");
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        let cyclic: Vec<&str> = errors
            .iter()
            .filter_map(|e| match &e.kind {
                CanonErrorKind::CyclicTypeAlias(name) => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(cyclic, vec!["A", "B"]);
    }

    #[test]
    fn an_alias_used_with_too_few_arguments_is_an_arity_mismatch() {
        let mut state = knot_syntax::ParseState::new(
            "type alias Pair a = (a, a)\nbad :: Pair -> Int\nbad p = 1\n",
        );
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            CanonErrorKind::TypeAliasArityMismatch { name, expected: 1, found: 0 }
                if name == "Pair"
        )));
    }

    #[test]
    fn an_adts_own_variant_field_type_expands_an_alias_too() {
        let cs = decls("type alias Point = { x : Float, y : Float }\ntype Shape = Circle Point\n");
        let variant_ty = cs
            .iter()
            .find_map(|d| match &d.node {
                CDecl::TypeDecl(name, _, variants) if name == "Shape" => {
                    Some(variants[0].1[0].clone())
                }
                _ => None,
            })
            .unwrap();
        assert!(matches!(variant_ty, CType::Record(..)));
    }

    #[test]
    fn an_extensible_record_alias_merges_a_concrete_records_fields() {
        // `Selectable Foo` should become the closed `{ name : String,
        // isSelected : Bool }`, not leave its own row variable dangling.
        let cs = decls(
            "type alias Foo = { name : String }\n\
             type alias Selectable a = { a | isSelected : Bool }\n\
             useSelectableFoo :: Selectable Foo -> Bool\n\
             useSelectableFoo s = s.isSelected\n",
        );
        let ty = fn_sig_ty(&cs, "useSelectableFoo");
        match ty {
            CType::Fn(a, _) => match *a {
                CType::Record(fields, ext) => {
                    assert_eq!(ext, None, "merged result should be closed");
                    assert!(fields.iter().any(|(n, t)| n == "isSelected"
                        && matches!(t, CType::Named(Ref::Builtin(b), _) if b == "Bool")));
                    assert!(fields.iter().any(|(n, t)| n == "name"
                        && matches!(t, CType::Named(Ref::Builtin(b), _) if b == "String")));
                    assert_eq!(fields.len(), 2);
                }
                other => panic!("expected a merged Record, got {other:?}"),
            },
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn an_extensible_record_alias_forwarded_by_another_alias_stays_open() {
        // `Wrap a = Selectable a` just forwards its own parameter along --
        // the row stays a genuinely free variable, not merged into anything.
        let cs = decls(
            "type alias Selectable a = { a | isSelected : Bool }\n\
             type alias Wrap a = Selectable a\n\
             useWrap :: Wrap a -> Bool\n\
             useWrap w = w.isSelected\n",
        );
        let ty = fn_sig_ty(&cs, "useWrap");
        match ty {
            CType::Fn(a, _) => match *a {
                CType::Record(fields, ext) => {
                    assert!(ext.is_some(), "row should still be open");
                    assert_eq!(fields.len(), 1);
                }
                other => panic!("expected an open Record, got {other:?}"),
            },
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn extending_a_non_record_type_is_a_record_extension_error() {
        let mut state = knot_syntax::ParseState::new(
            "type alias Selectable a = { a | isSelected : Bool }\n\
             bad :: Selectable Int -> Bool\n\
             bad s = s.isSelected\n",
        );
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            CanonErrorKind::RecordExtensionNotARecord { alias, param }
                if alias == "Selectable" && param == "a"
        )));
    }

    #[test]
    fn conflicting_field_names_between_alias_and_extension_are_reported() {
        let mut state = knot_syntax::ParseState::new(
            "type alias Foo = { name : String }\n\
             type alias Selectable a = { a | name : Bool }\n\
             bad :: Selectable Foo -> Bool\n\
             bad s = True\n",
        );
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            CanonErrorKind::RecordExtensionFieldConflict { alias, field }
                if alias == "Selectable" && field == "name"
        )));
    }

    #[test]
    fn an_instances_own_target_expands_an_alias_too() {
        let cs = decls(
            "type alias IntPair = (Int, Int)\ninstance Eq IntPair where\n  (==) a b = True\n",
        );
        let target = cs
            .iter()
            .find_map(|d| match &d.node {
                CDecl::Instance(inst) => Some(inst.target.clone()),
                _ => None,
            })
            .unwrap();
        assert!(matches!(target, CType::Tuple(_)));
    }
}
