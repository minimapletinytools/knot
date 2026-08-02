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
//!
//! **Record spreads** (`{ ..Name, field : Type }`, the record-spread
//! proposal) reuse this same pass rather than needing their own: a spread
//! target is just another alias reference (`collect_alias_refs` records it
//! as a dependency edge exactly like an ordinary one, so `topo_order`
//! always processes it first), except it merges its own fields into the
//! *surrounding* record instead of replacing a whole type occurrence — see
//! `resolve_spreads`. A spread target must be closed (no declared params,
//! no open row of its own — spread syntax takes no arguments, so there's
//! nothing to make it concrete with) and must resolve to an actual record;
//! `SpreadTargetNotALocalAlias`/`SpreadTargetNotARecord`/
//! `SpreadTargetNotClosed`/`SpreadFieldConflict` cover the ways that can
//! fail. `CType::Record`'s own `spreads` list is always empty by the time
//! `expand_aliases` returns — see that field's own doc comment on `ast.rs`.

use std::collections::{HashMap, HashSet};

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
/// (not even a wasted pass) when the module declares no aliases at all —
/// *and* uses no spread either: a spread with zero aliases in the whole
/// module is unconditionally invalid (there's nothing it could possibly
/// name), but that's still a real error to report, not something this fast
/// path may silently skip.
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
    if raw.is_empty() && !decls.iter().any(|d| decl_has_spread(&d.node)) {
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

/// True if `decl` contains a record spread (`{ ..Name, ... }`) anywhere —
/// `expand_aliases`'s own fast-path guard needs this so a module with zero
/// `type alias` declarations but a stray spread still gets a real error
/// instead of silently keeping an unresolved `spreads` entry (see that
/// function's own doc comment).
fn decl_has_spread(decl: &CDecl) -> bool {
    match decl {
        CDecl::Fn(fndef) => fndef_has_spread(fndef),
        CDecl::TypeAlias(_, _, ty) => ty_has_spread(ty),
        CDecl::TypeDecl(_, _, variants) => variants
            .iter()
            .any(|(_, args)| args.iter().any(ty_has_spread)),
        CDecl::Instance(inst) => {
            ty_has_spread(&inst.target) || inst.methods.iter().any(fndef_has_spread)
        }
    }
}

fn fndef_has_spread(fndef: &CFnDef) -> bool {
    fndef
        .signature
        .as_ref()
        .is_some_and(|sig| ty_has_spread(&sig.node.ty))
}

fn ty_has_spread(ty: &CType) -> bool {
    match ty {
        CType::Record(fields, spreads, _) => {
            !spreads.is_empty() || fields.iter().any(|(_, t)| ty_has_spread(t))
        }
        CType::Named(_, args) | CType::Tuple(args) => args.iter().any(ty_has_spread),
        CType::Fn(a, b) => ty_has_spread(a) || ty_has_spread(b),
        CType::Var(_) | CType::Unit => false,
    }
}

/// Every known alias name `ty` refers to, one level of `Named` at a time —
/// the dependency edges `topo_order` needs. Doesn't need to recurse into an
/// already-*found* alias's own body (that's `topo_order`'s own job, walking
/// the dependency graph itself); this only ever looks at `ty`'s own shape.
/// A record spread is exactly the same kind of dependency edge as an
/// ordinary alias reference (`resolve_spreads` needs its target's own body
/// already fully expanded, same as `substitute` needs for an inlined
/// reference), so it's collected here too.
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
        CType::Record(fields, spreads, _) => {
            for (_, t) in fields {
                collect_alias_refs(t, defs, out);
            }
            for spread_ref in spreads {
                if let Ref::TopLevel(name) = spread_ref {
                    if defs.contains_key(name) {
                        out.push(name.clone());
                    }
                }
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
        CType::Record(fields, spreads, ext) => {
            debug_assert!(
                spreads.is_empty(),
                "substitute_vars only ever runs on an alias body substitute() \
                 already fully expanded, so its own spreads are already resolved"
            );
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
        return CType::Record(own_fields, Vec::new(), None);
    };
    match mapping.get(name) {
        None => CType::Record(own_fields, Vec::new(), Some(name.clone())),
        Some(CType::Var(other)) => CType::Record(own_fields, Vec::new(), Some(other.clone())),
        Some(CType::Record(other_fields, other_spreads, other_ext)) => {
            debug_assert!(
                other_spreads.is_empty(),
                "a substituted-in record argument is already fully alias-expanded"
            );
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
            CType::Record(merged, Vec::new(), other_ext.clone())
        }
        Some(_) => {
            errors.push(CanonError::new(
                CanonErrorKind::RecordExtensionNotARecord {
                    alias: alias_name.to_string(),
                    param: name.clone(),
                },
                span,
            ));
            CType::Record(own_fields, Vec::new(), Some(name.clone()))
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
        CType::Record(fields, spreads, ext) => {
            let own_fields: Vec<(String, CType)> = fields
                .iter()
                .map(|(n, t)| (n.clone(), substitute(t, expanded, errors, span)))
                .collect();
            let merged = resolve_spreads(own_fields, spreads, expanded, errors, span);
            CType::Record(merged, Vec::new(), ext.clone())
        }
    }
}

/// Merges every spread target's own (already fully expanded) fields into
/// `own_fields`, in `spreads` order. Each target is looked up in `expanded`
/// directly by name — spread syntax takes no arguments, so there's never a
/// `mapping` to build the way an ordinary alias *reference* needs one — and
/// is guaranteed to already be present with its own `spreads` list empty,
/// since `collect_alias_refs` records a spread as a dependency edge exactly
/// like an ordinary alias reference, so `topo_order` always processes a
/// spread's target before whatever spreads it.
fn resolve_spreads(
    mut own_fields: Vec<(String, CType)>,
    spreads: &[Ref],
    expanded: &HashMap<String, AliasDef>,
    errors: &mut Vec<CanonError>,
    span: Span,
) -> Vec<(String, CType)> {
    // Spreading the exact same target twice in one literal (`{ ..A, ..A,
    // x : Float }`) is a no-op on the repeat, not a self-conflict --
    // per the proposal's own §8, which flagged this as worth deciding
    // explicitly rather than leaving implicit.
    let mut already_spread: HashSet<&str> = HashSet::new();
    for spread_ref in spreads {
        let Ref::TopLevel(name) = spread_ref else {
            // `Ref::Unresolved` already has its own error from name
            // resolution; anything else (Builtin/Imported) has no local
            // field list to look at at all.
            if !matches!(spread_ref, Ref::Unresolved(_)) {
                errors.push(CanonError::new(
                    CanonErrorKind::SpreadTargetNotALocalAlias {
                        name: ref_name(spread_ref),
                    },
                    span,
                ));
            }
            continue;
        };
        if !already_spread.insert(name.as_str()) {
            continue;
        }
        let Some(def) = expanded.get(name) else {
            // A real `Ref::TopLevel` that isn't a `type alias` at all --
            // an ADT `type` name, which has variants, not a field list.
            errors.push(CanonError::new(
                CanonErrorKind::SpreadTargetNotARecord { name: name.clone() },
                span,
            ));
            continue;
        };
        if !def.params.is_empty() {
            errors.push(CanonError::new(
                CanonErrorKind::SpreadTargetNotClosed { name: name.clone() },
                span,
            ));
            continue;
        }
        match &def.body {
            CType::Record(target_fields, target_spreads, None) => {
                debug_assert!(
                    target_spreads.is_empty(),
                    "a spread target's own spreads are already resolved by topo order"
                );
                for (field_name, field_ty) in target_fields {
                    if own_fields.iter().any(|(n, _)| n == field_name) {
                        errors.push(CanonError::new(
                            CanonErrorKind::SpreadFieldConflict {
                                name: name.clone(),
                                field: field_name.clone(),
                            },
                            span,
                        ));
                    } else {
                        own_fields.push((field_name.clone(), field_ty.clone()));
                    }
                }
            }
            CType::Record(_, _, Some(_)) => {
                errors.push(CanonError::new(
                    CanonErrorKind::SpreadTargetNotClosed { name: name.clone() },
                    span,
                ));
            }
            _ => {
                errors.push(CanonError::new(
                    CanonErrorKind::SpreadTargetNotARecord { name: name.clone() },
                    span,
                ));
            }
        }
    }
    own_fields
}

fn ref_name(r: &Ref) -> String {
    match r {
        Ref::TopLevel(n) | Ref::Builtin(n) | Ref::Unresolved(n) | Ref::Local(n) => n.clone(),
        Ref::Imported { name, .. } => name.clone(),
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
                CType::Record(fields, spreads, ext) => {
                    assert!(spreads.is_empty());
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
                CType::Record(fields, spreads, ext) => {
                    assert!(spreads.is_empty());
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

    // -- record spread (`{ ..Name, field : Type }`) --

    #[test]
    fn a_single_spread_merges_the_targets_fields() {
        let cs = decls(
            "type alias GraphicsElement = { id : Int, fill : String }\n\
             type alias Circle = { ..GraphicsElement, cx : Float, cy : Float }\n\
             useCircle :: Circle -> Float\n\
             useCircle c = c.cx\n",
        );
        let ty = fn_sig_ty(&cs, "useCircle");
        match ty {
            CType::Fn(a, _) => match *a {
                CType::Record(fields, spreads, ext) => {
                    assert!(spreads.is_empty());
                    assert_eq!(ext, None);
                    let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
                    assert_eq!(names.len(), 4, "{names:?}");
                    for expected in ["id", "fill", "cx", "cy"] {
                        assert!(names.contains(&expected), "missing {expected}: {names:?}");
                    }
                }
                other => panic!("expected a merged Record, got {other:?}"),
            },
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn multiple_spreads_in_one_literal_all_merge() {
        let cs = decls(
            "type alias Fills = { fill : String }\n\
             type alias Strokes = { stroke : String }\n\
             type alias Combined = { ..Fills, ..Strokes, extra : Int }\n\
             useCombined :: Combined -> Int\n\
             useCombined c = c.extra\n",
        );
        let ty = fn_sig_ty(&cs, "useCombined");
        match ty {
            CType::Fn(a, _) => match *a {
                CType::Record(fields, spreads, _) => {
                    assert!(spreads.is_empty());
                    let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
                    assert_eq!(names.len(), 3, "{names:?}");
                    for expected in ["fill", "stroke", "extra"] {
                        assert!(names.contains(&expected), "missing {expected}: {names:?}");
                    }
                }
                other => panic!("expected a merged Record, got {other:?}"),
            },
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn spreading_the_same_target_twice_is_a_no_op_not_a_conflict() {
        // Proposal §8's own open question, resolved: `{ ..A, ..A, x : T }`
        // must not self-conflict on A's own fields the second time.
        let cs = decls(
            "type alias A = { x : Int }\n\
             type alias Combined = { ..A, ..A, y : Bool }\n\
             useCombined :: Combined -> Int\n\
             useCombined c = c.x\n",
        );
        let ty = fn_sig_ty(&cs, "useCombined");
        match ty {
            CType::Fn(a, _) => match *a {
                CType::Record(fields, spreads, _) => {
                    assert!(spreads.is_empty());
                    let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
                    assert_eq!(names.len(), 2, "{names:?}");
                    assert!(names.contains(&"x"));
                    assert!(names.contains(&"y"));
                }
                other => panic!("expected a merged Record, got {other:?}"),
            },
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn a_spread_composes_with_the_records_own_open_extension_variable() {
        // { a | ..GraphicsElement, label : String } -- the spread's own
        // fields merge in, but the row stays open on `a`.
        let cs = decls(
            "type alias GraphicsElement = { id : Int }\n\
             type alias Named a = { a | ..GraphicsElement, label : String }\n\
             useNamed :: Named a -> String\n\
             useNamed n = n.label\n",
        );
        let ty = fn_sig_ty(&cs, "useNamed");
        match ty {
            CType::Fn(a, _) => match *a {
                CType::Record(fields, spreads, ext) => {
                    assert!(spreads.is_empty());
                    assert!(ext.is_some(), "row should still be open");
                    let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
                    assert_eq!(names.len(), 2, "{names:?}");
                    assert!(names.contains(&"id"));
                    assert!(names.contains(&"label"));
                }
                other => panic!("expected an open, spread-merged Record, got {other:?}"),
            },
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn a_spread_transitively_pulls_in_a_spread_of_a_spread() {
        // Circle spreads GraphicsElement, which itself spreads Base --
        // Circle should end up with fields from all three.
        let cs = decls(
            "type alias Base = { id : Int }\n\
             type alias GraphicsElement = { ..Base, fill : String }\n\
             type alias Circle = { ..GraphicsElement, cx : Float }\n\
             useCircle :: Circle -> Float\n\
             useCircle c = c.cx\n",
        );
        let ty = fn_sig_ty(&cs, "useCircle");
        match ty {
            CType::Fn(a, _) => match *a {
                CType::Record(fields, spreads, _) => {
                    assert!(spreads.is_empty());
                    let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
                    assert_eq!(names.len(), 3, "{names:?}");
                    for expected in ["id", "fill", "cx"] {
                        assert!(names.contains(&expected), "missing {expected}: {names:?}");
                    }
                }
                other => panic!("expected a merged Record, got {other:?}"),
            },
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn a_spread_used_directly_in_a_function_signature_still_expands() {
        // No enclosing `type alias` at all -- confirms this doesn't only
        // work when a spread happens to sit inside an alias body.
        let cs = decls(
            "type alias GraphicsElement = { id : Int }\n\
             useIt :: { ..GraphicsElement, cx : Float } -> Float\n\
             useIt r = r.cx\n",
        );
        let ty = fn_sig_ty(&cs, "useIt");
        match ty {
            CType::Fn(a, _) => match *a {
                CType::Record(fields, spreads, _) => {
                    assert!(spreads.is_empty());
                    assert_eq!(fields.len(), 2);
                }
                other => panic!("expected a merged Record, got {other:?}"),
            },
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn conflicting_field_between_spread_and_explicit_field_is_reported() {
        let mut state = knot_syntax::ParseState::new(
            "type alias GraphicsElement = { id : Int }\n\
             type alias Circle = { ..GraphicsElement, id : Float }\n\
             useCircle :: Circle -> Float\n\
             useCircle c = c.id\n",
        );
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            CanonErrorKind::SpreadFieldConflict { name, field }
                if name == "GraphicsElement" && field == "id"
        )));
    }

    #[test]
    fn conflicting_field_between_two_spreads_is_reported() {
        let mut state = knot_syntax::ParseState::new(
            "type alias A = { x : Int }\n\
             type alias B = { x : Float }\n\
             type alias Combined = { ..A, ..B }\n\
             useCombined :: Combined -> Int\n\
             useCombined c = 1\n",
        );
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            CanonErrorKind::SpreadFieldConflict { name, field }
                if name == "B" && field == "x"
        )));
    }

    #[test]
    fn spreading_a_parametric_alias_is_not_closed() {
        let mut state = knot_syntax::ParseState::new(
            "type alias Pair a = { x : a }\n\
             type alias Bad = { ..Pair, y : Int }\n\
             useBad :: Bad -> Int\n\
             useBad b = b.y\n",
        );
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            CanonErrorKind::SpreadTargetNotClosed { name } if name == "Pair"
        )));
    }

    #[test]
    fn spreading_an_alias_with_its_own_open_row_is_not_closed() {
        let mut state = knot_syntax::ParseState::new(
            "type alias Selectable a = { a | isSelected : Bool }\n\
             type alias Bad = { ..Selectable, y : Int }\n\
             useBad :: Bad -> Int\n\
             useBad b = b.y\n",
        );
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            CanonErrorKind::SpreadTargetNotClosed { name } if name == "Selectable"
        )));
    }

    #[test]
    fn spreading_a_non_record_alias_is_reported() {
        let mut state = knot_syntax::ParseState::new(
            "type alias NotARecord = Int\n\
             type alias Bad = { ..NotARecord, y : Int }\n\
             useBad :: Bad -> Int\n\
             useBad b = b.y\n",
        );
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            CanonErrorKind::SpreadTargetNotARecord { name } if name == "NotARecord"
        )));
    }

    #[test]
    fn spreading_an_adt_type_name_is_reported() {
        // Shape has variants, not a field list -- structurally not a
        // record no matter how you look at it.
        let mut state = knot_syntax::ParseState::new(
            "type Shape = Circle Float\n\
             type alias Bad = { ..Shape, y : Int }\n\
             useBad :: Bad -> Int\n\
             useBad b = b.y\n",
        );
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            CanonErrorKind::SpreadTargetNotARecord { name } if name == "Shape"
        )));
    }

    #[test]
    fn spreading_a_builtin_type_is_reported() {
        let mut state = knot_syntax::ParseState::new(
            "type alias Bad = { ..Int, y : Int }\n\
             useBad :: Bad -> Int\n\
             useBad b = b.y\n",
        );
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            CanonErrorKind::SpreadTargetNotALocalAlias { name } if name == "Int"
        )));
    }

    #[test]
    fn a_self_spread_is_a_cyclic_error_not_an_infinite_loop() {
        let mut state = knot_syntax::ParseState::new("type alias A = { ..A, x : Int }\n");
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors
            .iter()
            .any(|e| matches!(&e.kind, CanonErrorKind::CyclicTypeAlias(name) if name == "A")));
    }

    #[test]
    fn a_stray_spread_with_zero_aliases_in_the_module_is_still_an_error() {
        // Regression test for expand_aliases's own early-return: with no
        // `type alias` declarations at all (so `raw.is_empty()`), the old
        // fast path would return before ever looking at this spread,
        // silently leaving it unresolved instead of reporting it. `Int`
        // resolves fine as an ordinary name (no *name-resolution* error) --
        // the only way this can fail is `expand_aliases`'s own spread
        // handling actually running.
        let mut state =
            knot_syntax::ParseState::new("useBad :: { ..Int, y : Int } -> Int\nuseBad b = b.y\n");
        let raw = state.parse_decls().unwrap();
        let (_cdecls, errors) = resolve_decls(&raw);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            CanonErrorKind::SpreadTargetNotALocalAlias { name } if name == "Int"
        )));
    }
}
