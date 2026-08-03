//! The driver that ties constraint generation to `unify.rs`: walks a
//! `Constraint` tree, actually calling `unify` for every `Equal`, building
//! the scheme environment `Constraint::Lookup` needs, and running
//! `generalize` at each top-level `Constraint::Let`'s boundary (plan §7,
//! TM5). Errors are collected across the whole tree rather than stopping at
//! the first, matching `knot-canonical`'s own stance.
//!
//! **Two-phase `HasInstance` checking.** A `HasInstance{ty, ..}` obligation
//! often can't be classified the moment it's seen — `ty` might still be an
//! unresolved flexible variable that a *later* `Equal` in the same list
//! ultimately pins down (e.g. a signed binding's body constrains its
//! parameter's type via a plain `Equal` generated *after* the `HasInstance`
//! obligation that parameter's own use produced — see `constrain::decl`).
//! So every `HasInstance` is just collected into `pending` as `solve_rec`
//! walks the tree, calling `unify` immediately for every `Equal` along the
//! way; only *after* the whole tree (or, for `Ref::Local`-shadowed rigid
//! checks specifically, since rigid identity is globally unique via
//! `Substitution::fresh_rigid`, effectively the whole program) has been
//! walked does `solve` go back and classify each pending obligation: rigid
//! (checked against `given` — no instance table needed, a rigid variable
//! can never have more permissions than its own signature granted) or
//! concrete (left in the returned list — checking those against a real
//! instance table is `interface/table.rs`'s job, TM6).
//!
//! **What a `Constraint::Let` actually does here**: push every member's
//! `header_ty` onto `ambient` (so no *inner* `generalize` call mistakes an
//! outer, still-in-progress binding's own type variable for one it owns)
//! and register any signature's `given` facts, solve `header_con`, then for
//! each member build a `Scheme` — a signed member's is just its signature
//! restated (`declared`'s `rigid_vars`/`given` directly); an unsigned one
//! needs the real `generalize` (`ftv(ty) \ ftv(ambient)`, gathering up any
//! of *its own* pending obligations into the new scheme's `constraints`
//! along the way) — then run the ambiguous-CAF check against it (this
//! session's replacement for the monomorphism restriction, plan §3). Both
//! `top_level: true` and `false` groups do all of this identically; only
//! the destination differs: `true` installs into `env` (the global
//! `SchemeEnv`, keyed by name, for `Constraint::Lookup` to find later);
//! `false` installs into `local_env` (keyed by the member's own
//! `header_ty`, for `Constraint::LookupLocal` to find instead) — see
//! `constrain::mod`'s own doc on why `Ref::Local` forces a `let`-bound
//! name through a different lookup constraint than a top-level one, even
//! though both now get real let-polymorphism.

use std::collections::{HashMap, HashSet};

use knot_canonical::ast::Ref;
use knot_syntax::span::Span;

use crate::constrain::Constraint;
use crate::error::{TypeError, TypeErrorKind};
use crate::interface::table::superclasses;
use crate::ty::{Scheme, Structure};
use crate::unify::{structure_children, unify};
use crate::var::{Substitution, TypeVarId};

/// Which namespace a `Constraint::Lookup` reference resolves against —
/// `Ref::Local`/`Ref::Unresolved` never need one (see `scheme_key`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SchemeKey {
    TopLevel(String),
    Builtin(String),
    Imported(Vec<String>, String),
}

pub fn scheme_key(reference: &Ref) -> Option<SchemeKey> {
    match reference {
        Ref::TopLevel(name) => Some(SchemeKey::TopLevel(name.clone())),
        Ref::Builtin(name) => Some(SchemeKey::Builtin(name.clone())),
        Ref::Imported { module, name } => Some(SchemeKey::Imported(module.clone(), name.clone())),
        Ref::Local(_) | Ref::Unresolved(_) => None,
    }
}

fn reference_display(reference: &Ref) -> String {
    match reference {
        Ref::TopLevel(n) | Ref::Builtin(n) | Ref::Local(n) | Ref::Unresolved(n) => n.clone(),
        Ref::Imported { module, name } => format!("{}.{}", module.join("."), name),
    }
}

/// Every scheme available to `Constraint::Lookup`, keyed by namespace.
/// Starts out populated only with whatever a caller seeds ahead of time
/// (prelude signatures, TM8) — module-level `let`/top-level bindings add to
/// it as `solve` walks their `Constraint::Let`s.
#[derive(Debug, Default)]
pub struct SchemeEnv {
    schemes: HashMap<SchemeKey, Scheme>,
}

impl SchemeEnv {
    pub fn new() -> Self {
        SchemeEnv::default()
    }

    pub fn insert(&mut self, key: SchemeKey, scheme: Scheme) {
        self.schemes.insert(key, scheme);
    }

    pub fn get(&self, key: &SchemeKey) -> Option<&Scheme> {
        self.schemes.get(key)
    }
}

/// A `HasInstance` obligation not yet classified as satisfied or not — see
/// module docs on the two-phase approach. Never actually returned once
/// `ty` resolves to a rigid variable (`solve` fully resolves those inline);
/// only concrete-typed obligations, needing the real instance table (TM6),
/// survive into the returned list.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingInstance {
    pub span: Span,
    pub interface: String,
    pub ty: TypeVarId,
}

/// `solve_with_obligations`'s side-table shape — see its own doc comment.
pub type ObligationMap = HashMap<TypeVarId, Vec<(String, TypeVarId)>>;

/// Entry point: solves `constraint` against `env` (mutated in place with
/// whatever new top-level schemes it discovers), returning every
/// still-unresolved (concrete-typed) instance obligation plus every error
/// found across the whole tree.
pub fn solve(
    sub: &mut Substitution,
    env: &mut SchemeEnv,
    constraint: &Constraint,
) -> (Vec<PendingInstance>, Vec<TypeError>) {
    let (pending, errors, _obligations, _given) = solve_with_obligations(sub, env, constraint);
    (pending, errors)
}

/// Same as `solve`, plus two extras: a side-table Fix #3's Stage B
/// (`elaborate::elaborate_module`) needs -- for every `Lookup`/`LookupLocal`
/// reference resolved along the way, its own `expected` `TypeVarId` (also
/// that reference's `TExpr::Var`/`TExpr::Ctor` node's own `ty` — see
/// `ast.rs`'s own doc comment on why that's already a unique key with
/// nothing extra needed) mapped to exactly the `(interface, TypeVarId)`
/// pairs its scheme's instantiation produced -- and the final `given` map
/// itself (every rigid variable's own directly-and-transitively-implied
/// interfaces, per `insert_given_with_superclasses`), which `interface::
/// instance::check_instance`'s own recursive per-argument checks need too
/// (Fix #13): a *concrete* pending obligation like `Semigroup (Max a)` still
/// has a bare rigid `a` buried inside it once `check_instance` recurses into
/// `Max`'s own `requires`, and that rigid `a` can only ever be resolved by
/// consulting `given`, never by `sub.resolve_structure` (a `Rigid` slot has
/// no structure to resolve, by design). Split out from `solve` itself
/// (rather than always building it) purely so `solve`'s own ~15 existing
/// callers, which have no use for either, don't have to change at all.
pub fn solve_with_obligations(
    sub: &mut Substitution,
    env: &mut SchemeEnv,
    constraint: &Constraint,
) -> (
    Vec<PendingInstance>,
    Vec<TypeError>,
    ObligationMap,
    HashMap<TypeVarId, HashSet<String>>,
) {
    let mut ambient = Vec::new();
    let mut given: HashMap<TypeVarId, HashSet<String>> = HashMap::new();
    let mut local_env: HashMap<TypeVarId, Scheme> = HashMap::new();
    let mut pending = Vec::new();
    let mut errors = Vec::new();
    let mut obligations = HashMap::new();
    solve_rec(
        sub,
        env,
        &mut local_env,
        &mut ambient,
        &mut given,
        &mut pending,
        &mut errors,
        &mut obligations,
        constraint,
    );

    let mut unresolved = Vec::new();
    for p in pending {
        let root = sub.find(p.ty);
        if sub.is_rigid(root) {
            let satisfied = given
                .get(&root)
                .is_some_and(|interfaces| interfaces.contains(&p.interface));
            if !satisfied {
                errors.push(TypeError {
                    span: p.span,
                    kind: TypeErrorKind::NoInstanceForRigid {
                        interface: p.interface,
                    },
                });
            }
        } else if p.interface == "Num" && sub.resolve_structure(root).is_none() {
            // Numeric-literal defaulting, second half -- see `generalize`'s
            // own doc comment for the first half (a `Num` obligation
            // captured into some binding's own generalized scheme, e.g.
            // `x = 5`). This half catches the opposite case: a `Num`
            // obligation that never became part of *any* scheme's own
            // quantified variables at all, e.g. `f x y = x == y; result =
            // f 1 2` -- both literals' own fresh variables get unified
            // together with `f`'s own rigid-at-declaration, flexible-at-
            // use-site `a`, but `result`'s own inferred type is just
            // `Bool`, so that shared variable is never part of anyone's
            // own free-type-variable set to quantify (and thus default)
            // over inside `generalize`. Left unhandled, it would just stay
            // "pending" forever, silently never resolving to anything
            // (surfacing as `StillAbstract` wherever `elaborate::
            // elaborate_module` looked at it, or simply vanishing here
            // otherwise) rather than the `Int` a real user would expect.
            sub.bind(
                root,
                Structure::App(Ref::Builtin("Int".to_string()), Vec::new()),
            );
        } else {
            unresolved.push(p);
        }
    }
    (unresolved, errors, obligations, given)
}

/// Records `var` as `given interface`, *and* every superclass `interface`
/// itself transitively implies (`Ord` -> `Eq`; `Integral` -> `Num`, `Ord`,
/// and (via `Ord`) `Eq`) -- not just the literal interface named in a
/// signature's own constraint list or an instance's own context. Without
/// this, `f :: Ord a => a -> a -> Bool; f x y = x == y` would wrongly fail
/// with `NoInstanceForRigid("Eq")`: every real `Ord` instance is
/// *guaranteed* to have a matching `Eq` one (superclass coherence enforces
/// this at declaration time, see `interface::instance::build_instance_
/// table`), so a rigid variable only ever given `Ord` should already be
/// able to use `Eq`-only operations too. Found via `corpus/programs`'s own
/// realistic-program probing (Fix #12) -- `binary-search-tree.knot`'s own
/// `contains` uses `==` under nothing but `Ord a =>`, and `combine-all.knot`
/// uses `<>` under nothing but `Monoid a =>`.
fn insert_given_with_superclasses(
    given: &mut HashMap<TypeVarId, HashSet<String>>,
    var: TypeVarId,
    interface: &str,
) {
    let mut closure = HashSet::new();
    closure.insert(interface.to_string());
    collect_transitive_superclasses(interface, &mut closure);
    given.entry(var).or_default().extend(closure);
}

fn collect_transitive_superclasses(interface: &str, out: &mut HashSet<String>) {
    for sup in superclasses(interface) {
        if out.insert(sup.to_string()) {
            collect_transitive_superclasses(sup, out);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn solve_rec(
    sub: &mut Substitution,
    env: &mut SchemeEnv,
    local_env: &mut HashMap<TypeVarId, Scheme>,
    ambient: &mut Vec<TypeVarId>,
    given: &mut HashMap<TypeVarId, HashSet<String>>,
    pending: &mut Vec<PendingInstance>,
    errors: &mut Vec<TypeError>,
    obligations: &mut ObligationMap,
    constraint: &Constraint,
) {
    match constraint {
        Constraint::True => {}
        Constraint::Equal {
            span,
            expected,
            actual,
        } => {
            if let Err(e) = unify(sub, *expected, *actual) {
                errors.push(TypeError {
                    span: *span,
                    kind: TypeErrorKind::Unify(e),
                });
            }
        }
        Constraint::HasInstance {
            span,
            interface,
            ty,
        } => {
            pending.push(PendingInstance {
                span: *span,
                interface: interface.clone(),
                ty: *ty,
            });
        }
        Constraint::Lookup {
            span,
            reference,
            expected,
        } => match scheme_key(reference).and_then(|k| env.get(&k).cloned()) {
            Some(scheme) => {
                let (fresh_ty, fresh_constraints) = instantiate(sub, &scheme);
                if let Err(e) = unify(sub, *expected, fresh_ty) {
                    errors.push(TypeError {
                        span: *span,
                        kind: TypeErrorKind::Unify(e),
                    });
                }
                obligations.insert(
                    *expected,
                    fresh_constraints
                        .iter()
                        .map(|(ty, interface)| (interface.clone(), *ty))
                        .collect(),
                );
                for (ty, interface) in fresh_constraints {
                    pending.push(PendingInstance {
                        span: *span,
                        interface,
                        ty,
                    });
                }
            }
            None => errors.push(TypeError {
                span: *span,
                kind: TypeErrorKind::UnboundValue(reference_display(reference)),
            }),
        },
        // The `let`-bound counterpart of `Lookup` above -- identical logic,
        // just keyed by `TypeVarId` against `local_env` instead of by
        // `SchemeKey` against `env` (see `constrain::mod`'s own doc on why
        // `Ref::Local` needs a separate constraint at all).
        Constraint::LookupLocal {
            span,
            key,
            expected,
        } => match local_env.get(key).cloned() {
            Some(scheme) => {
                let (fresh_ty, fresh_constraints) = instantiate(sub, &scheme);
                if let Err(e) = unify(sub, *expected, fresh_ty) {
                    errors.push(TypeError {
                        span: *span,
                        kind: TypeErrorKind::Unify(e),
                    });
                }
                obligations.insert(
                    *expected,
                    fresh_constraints
                        .iter()
                        .map(|(ty, interface)| (interface.clone(), *ty))
                        .collect(),
                );
                for (ty, interface) in fresh_constraints {
                    pending.push(PendingInstance {
                        span: *span,
                        interface,
                        ty,
                    });
                }
            }
            None => panic!(
                "LookupLocal({key:?}) has no scheme -- constrain::decl should have installed one \
                 into local_env before any reference to it could have been generated"
            ),
        },
        Constraint::And(cs) => {
            for c in cs {
                solve_rec(
                    sub,
                    env,
                    local_env,
                    ambient,
                    given,
                    pending,
                    errors,
                    obligations,
                    c,
                );
            }
        }
        Constraint::Let {
            top_level,
            members,
            header_con,
            body_con,
        } => {
            let start = ambient.len();
            for m in members {
                ambient.push(m.header_ty);
                if let Some(d) = &m.declared {
                    for (var, interface) in &d.given {
                        insert_given_with_superclasses(given, *var, interface);
                    }
                }
            }

            solve_rec(
                sub,
                env,
                local_env,
                ambient,
                given,
                pending,
                errors,
                obligations,
                header_con,
            );

            // Pop this group's own header_tys *before* generalizing them --
            // `ambient` must reflect only outer (ancestor, still-in-progress)
            // bindings during the ftv-difference below, never the very
            // variables a member is trying to quantify over. Leaving them
            // in here was a real bug: it made `ftv(ty) \ ftv(ambient)`
            // cancel to empty every time, silently defeating
            // generalization for every top-level binding.
            ambient.truncate(start);

            for m in members {
                let scheme = match &m.declared {
                    Some(d) => Scheme {
                        vars: d.rigid_vars.clone(),
                        constraints: d.given.clone(),
                        ty: m.header_ty,
                    },
                    None => generalize(sub, ambient, m.header_ty, pending),
                };
                check_ambiguous(sub, &scheme, m.span, errors);
                if *top_level {
                    env.insert(SchemeKey::TopLevel(m.name.clone()), scheme);
                } else {
                    local_env.insert(m.header_ty, scheme);
                }
            }

            solve_rec(
                sub,
                env,
                local_env,
                ambient,
                given,
                pending,
                errors,
                obligations,
                body_con,
            );
        }
        // An instance method's own `given` facts -- register and recurse,
        // no generalization/scheme-installation/ambiguity-checking at all
        // (see this variant's own doc comment on why none of that applies).
        Constraint::Given { given: facts, body } => {
            for (var, interface) in facts {
                insert_given_with_superclasses(given, *var, interface);
            }
            solve_rec(
                sub,
                env,
                local_env,
                ambient,
                given,
                pending,
                errors,
                obligations,
                body,
            );
        }
    }
}

/// This session's replacement for the monomorphism restriction (plan §3):
/// a zero-argument binding generalized over a variable that still carries
/// an interface obligation is ambiguous outright, rather than defaulted or
/// silently allowed. Arity >= 1 (the resolved type is a function) is never
/// restricted, no matter how the constrained variable is used.
fn check_ambiguous(
    sub: &mut Substitution,
    scheme: &Scheme,
    span: Span,
    errors: &mut Vec<TypeError>,
) {
    if scheme.constraints.is_empty() {
        return;
    }
    if matches!(sub.resolve_structure(scheme.ty), Some(Structure::Fn(_, _))) {
        return;
    }
    for (_, interface) in &scheme.constraints {
        errors.push(TypeError {
            span,
            kind: TypeErrorKind::AmbiguousConstraint {
                interface: interface.clone(),
            },
        });
    }
}

/// `ftv(ty) \ ftv(ambient)` (plan §4's simplified, non-rank-based
/// alternative to Elm's pool/level generalization) — the textbook algorithm,
/// not the performance-oriented one, per the project's standing
/// simplicity-over-performance instruction. Also gathers up any `pending`
/// obligation whose type is among the newly-quantified variables into the
/// resulting scheme's own `constraints`, removing it from `pending` (it's
/// now this scheme's responsibility to re-derive fresh at each
/// instantiation, not something to check once here and forget) -- *unless*
/// it's a still-dangling `"Num"` obligation, which defaults to `Int`
/// instead (see the loop's own doc comment).
fn generalize(
    sub: &mut Substitution,
    ambient: &[TypeVarId],
    ty: TypeVarId,
    pending: &mut Vec<PendingInstance>,
) -> Scheme {
    let mut ftv_ty = HashSet::new();
    free_vars(sub, ty, &mut ftv_ty);
    let mut ftv_ambient = HashSet::new();
    for &a in ambient {
        free_vars(sub, a, &mut ftv_ambient);
    }
    let mut quantified: HashSet<TypeVarId> = ftv_ty
        .difference(&ftv_ambient)
        .copied()
        // A rigid variable is never "ours" to generalize -- it belongs to
        // whatever signature introduced it. Only reachable here if it's
        // somehow absent from `ambient` too, which shouldn't happen given
        // how `solve_rec` populates `ambient`, but costs nothing to guard.
        .filter(|v| !sub.is_rigid(*v))
        .collect();

    let mut constraints = Vec::new();
    pending.retain(|p| {
        let root = sub.find(p.ty);
        if !quantified.contains(&root) {
            return true;
        }
        // Numeric-literal defaulting (spec's simplified stand-in for
        // Haskell's own defaulting rules): `constrain::expr`'s own
        // `CExpr::IntLit` handling is the *only* thing in this closed
        // language that can ever produce a bare, non-function,
        // `Num`-polymorphic value with nothing else to pin it down (every
        // other route to a `Num` obligation either applies a function --
        // resolving the type via unification with a concrete argument or
        // signature -- or is itself function-typed, already exempted by
        // `check_ambiguous`'s own arity check). Rather than leaving `x =
        // 5` (no signature) either needlessly quantified or tripping that
        // same zero-arg restriction, resolve it to `Int` right here,
        // before it ever becomes part of a scheme at all -- a real,
        // user-written constraint (`Ord a =>`) is never named `"Num"`
        // *and* still dangling like this, so this can't misfire on one.
        if p.interface == "Num" && sub.resolve_structure(root).is_none() {
            sub.bind(
                root,
                Structure::App(Ref::Builtin("Int".to_string()), Vec::new()),
            );
            quantified.remove(&root);
            return false;
        }
        constraints.push((root, p.interface.clone()));
        false
    });

    Scheme {
        vars: quantified.into_iter().collect(),
        constraints,
        ty,
    }
}

/// Every free (`Unbound` or `Rigid`) variable reachable from `ty`'s own
/// structure. Both count as "free" for `generalize`'s purposes: an
/// `Unbound` one might still need quantifying (or excluding, if it's also
/// in the ambient set); a `Rigid` one must never be implicitly adopted by
/// an unrelated inner binding as if it were its own.
fn free_vars(sub: &mut Substitution, ty: TypeVarId, out: &mut HashSet<TypeVarId>) {
    let root = sub.find(ty);
    match sub.resolve_structure(root) {
        Some(structure) => {
            for child in structure_children(&structure) {
                free_vars(sub, child, out);
            }
        }
        None => {
            out.insert(root);
        }
    }
}

/// Fresh copy of `scheme.ty`, substituting a fresh unbound variable for
/// each of `scheme.vars`, plus a fresh `HasInstance`-shaped obligation per
/// `scheme.constraints` entry pointing at whichever fresh variable its own
/// quantified var mapped to. Every other node gets copied too (rather than
/// shared with the original) for simplicity — safe, not just convenient:
/// two structurally-identical-but-differently-`TypeVarId`'d bound nodes are
/// fully interchangeable under `unify`, which only ever compares structure,
/// never identity, so failing to share a plain (non-quantified) subtree
/// changes nothing observable, just costs a few extra arena slots.
pub fn instantiate(
    sub: &mut Substitution,
    scheme: &Scheme,
) -> (TypeVarId, Vec<(TypeVarId, String)>) {
    if scheme.vars.is_empty() {
        return (scheme.ty, Vec::new());
    }
    let mut mapping = HashMap::new();
    for &v in &scheme.vars {
        mapping.insert(v, sub.fresh_unbound());
    }
    let fresh_ty = copy_structure(sub, scheme.ty, &mapping);
    let fresh_constraints = scheme
        .constraints
        .iter()
        .map(|(v, interface)| (*mapping.get(v).unwrap_or(v), interface.clone()))
        .collect();
    (fresh_ty, fresh_constraints)
}

fn copy_structure(
    sub: &mut Substitution,
    ty: TypeVarId,
    mapping: &HashMap<TypeVarId, TypeVarId>,
) -> TypeVarId {
    let root = sub.find(ty);
    if let Some(&fresh) = mapping.get(&root) {
        return fresh;
    }
    match sub.resolve_structure(root) {
        Some(Structure::App(r, args)) => {
            let new_args = args
                .iter()
                .map(|a| copy_structure(sub, *a, mapping))
                .collect();
            sub.fresh_bound(Structure::App(r, new_args))
        }
        Some(Structure::Fn(a, b)) => {
            let na = copy_structure(sub, a, mapping);
            let nb = copy_structure(sub, b, mapping);
            sub.fresh_bound(Structure::Fn(na, nb))
        }
        Some(Structure::Tuple(elems)) => {
            let new_elems = elems
                .iter()
                .map(|e| copy_structure(sub, *e, mapping))
                .collect();
            sub.fresh_bound(Structure::Tuple(new_elems))
        }
        Some(Structure::Record(fields, ext)) => {
            let new_fields = fields
                .iter()
                .map(|(k, v)| (k.clone(), copy_structure(sub, *v, mapping)))
                .collect();
            let new_ext = ext.map(|e| copy_structure(sub, e, mapping));
            sub.fresh_bound(Structure::Record(new_fields, new_ext))
        }
        Some(Structure::Unit) => sub.fresh_bound(Structure::Unit),
        // The partially-applied leading arguments (see that variant's own
        // doc comment) go through `copy_structure` too, same reasoning as
        // `VarApp`'s own `args` just below.
        Some(Structure::Ctor(r, args)) => {
            let new_args = args
                .iter()
                .map(|a| copy_structure(sub, *a, mapping))
                .collect();
            sub.fresh_bound(Structure::Ctor(r, new_args))
        }
        // The constructor-variable head goes through `copy_structure` too,
        // exactly like any other child -- if it's one of `scheme.vars`
        // (the common case: `map`'s own `f`), the `mapping` check at the
        // top of this function catches it and returns the fresh copy
        // already reserved for it; if it's some outer, non-generalized
        // constructor variable, it falls through to the `None` arm below
        // and is correctly shared as-is instead.
        Some(Structure::VarApp(f, args)) => {
            let nf = copy_structure(sub, f, mapping);
            let new_args = args
                .iter()
                .map(|a| copy_structure(sub, *a, mapping))
                .collect();
            sub.fresh_bound(Structure::VarApp(nf, new_args))
        }
        // Unbound (free in the ambient environment, not one of `scheme.vars`)
        // or Rigid (belongs to some enclosing signature) -- neither gets
        // copied; both are shared as-is with the original. An unresolved
        // constructor variable (`fresh_ctor_unbound`, never bound to a
        // `Ctor`) also lands here, for the same reason -- see `ty::
        // Structure::VarApp`'s own doc comment on why it needs no separate
        // case.
        None => root,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constrain::decl::constrain_module;
    use knot_canonical::ast::CDecl;
    use knot_syntax::span::Spanned;

    fn decls(src: &str) -> Vec<Spanned<CDecl>> {
        let mut state = knot_syntax::ParseState::new(src);
        let raw = state.parse_decls().unwrap();
        assert!(state.is_eof(), "leftover input: {src}");
        knot_canonical::canonicalize_decls(&raw).unwrap_or_else(|errs| panic!("{errs:?}"))
    }

    fn scheme_of<'a>(env: &'a SchemeEnv, name: &str) -> &'a Scheme {
        env.get(&SchemeKey::TopLevel(name.to_string()))
            .unwrap_or_else(|| panic!("no scheme installed for {name:?}"))
    }

    /// TM8 hasn't wired up the real prelude yet, so any test whose source
    /// mentions `True`/`False` needs to seed them itself.
    fn seed_bool_ctors(sub: &mut Substitution, env: &mut SchemeEnv) {
        let bool_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Bool".to_string()), vec![]));
        env.insert(
            SchemeKey::Builtin("True".to_string()),
            Scheme::monomorphic(bool_ty),
        );
        env.insert(
            SchemeKey::Builtin("False".to_string()),
            Scheme::monomorphic(bool_ty),
        );
    }

    #[test]
    fn unsigned_binding_is_generalized_over_its_own_free_variable() {
        let mut sub = Substitution::new();
        let cs = decls("id x = x\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let mut env = SchemeEnv::new();
        let (pending, errors) = solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(pending.is_empty());
        let scheme = scheme_of(&env, "id");
        assert_eq!(scheme.vars.len(), 1);
        assert!(scheme.constraints.is_empty());
        assert_eq!(
            sub.resolve_structure(scheme.ty),
            Some(Structure::Fn(scheme.vars[0], scheme.vars[0]))
        );
    }

    #[test]
    fn let_polymorphism_lets_a_top_level_binding_be_used_at_two_types() {
        let mut sub = Substitution::new();
        let cs = decls("identity x = x\nuseIdentity = (identity 1, identity True)\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let mut env = SchemeEnv::new();
        seed_bool_ctors(&mut sub, &mut env);
        let (pending, errors) = solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(pending.is_empty());

        let use_scheme = scheme_of(&env, "useIdentity");
        match sub.resolve_structure(use_scheme.ty) {
            Some(Structure::Tuple(elems)) => {
                assert_eq!(elems.len(), 2);
                let int_ty =
                    sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
                let bool_ty =
                    sub.fresh_bound(Structure::App(Ref::Builtin("Bool".to_string()), vec![]));
                assert_eq!(
                    sub.resolve_structure(elems[0]),
                    sub.resolve_structure(int_ty)
                );
                assert_eq!(
                    sub.resolve_structure(elems[1]),
                    sub.resolve_structure(bool_ty)
                );
            }
            other => panic!("expected a Tuple(Int, Bool), got {other:?}"),
        }
    }

    #[test]
    fn let_polymorphism_lets_a_let_bound_binding_be_used_at_two_types() {
        // Fix #1's own flagship: same shape as the top-level version above,
        // but `id` is `let`-bound inside `useIdentity`'s body instead of
        // being a sibling top-level function -- exercises
        // `LocalBinding::Generalizable`/`Constraint::LookupLocal`/
        // `local_env` end to end rather than `Lookup`/the global `SchemeEnv`.
        let mut sub = Substitution::new();
        let cs = decls("useIdentity = let id = \\x -> x in (id 1, id True)\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let mut env = SchemeEnv::new();
        seed_bool_ctors(&mut sub, &mut env);
        let (pending, errors) = solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(pending.is_empty());

        let use_scheme = scheme_of(&env, "useIdentity");
        match sub.resolve_structure(use_scheme.ty) {
            Some(Structure::Tuple(elems)) => {
                assert_eq!(elems.len(), 2);
                let int_ty =
                    sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
                let bool_ty =
                    sub.fresh_bound(Structure::App(Ref::Builtin("Bool".to_string()), vec![]));
                assert_eq!(
                    sub.resolve_structure(elems[0]),
                    sub.resolve_structure(int_ty)
                );
                assert_eq!(
                    sub.resolve_structure(elems[1]),
                    sub.resolve_structure(bool_ty)
                );
            }
            other => panic!("expected a Tuple(Int, Bool), got {other:?}"),
        }
    }

    #[test]
    fn mutually_recursive_functions_solve_to_concrete_non_generic_schemes() {
        let mut sub = Substitution::new();
        let cs = decls(
            "isOdd n = if n == 0 then False else isEven (n - 1)\n\
             isEven n = if n == 0 then True else isOdd (n - 1)\n",
        );
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let mut env = SchemeEnv::new();
        seed_bool_ctors(&mut sub, &mut env);
        let (_pending, errors) = solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");

        for name in ["isOdd", "isEven"] {
            let scheme = scheme_of(&env, name);
            assert!(scheme.vars.is_empty(), "{name} should be fully concrete");
            match sub.resolve_structure(scheme.ty) {
                Some(Structure::Fn(arg, ret)) => {
                    let int_ty =
                        sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
                    let bool_ty =
                        sub.fresh_bound(Structure::App(Ref::Builtin("Bool".to_string()), vec![]));
                    assert_eq!(sub.resolve_structure(arg), sub.resolve_structure(int_ty));
                    assert_eq!(sub.resolve_structure(ret), sub.resolve_structure(bool_ty));
                }
                other => panic!("expected {name} :: Int -> Bool, got {other:?}"),
            }
        }
    }

    #[test]
    fn unsigned_binding_gathers_its_own_dangling_pending_instance_into_its_scheme() {
        let mut sub = Substitution::new();
        let cs = decls("useEq x y = x == y\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let mut env = SchemeEnv::new();
        let (pending, errors) = solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        // The Eq obligation belongs to useEq's own quantified variable, so
        // it should have been absorbed into the scheme, not left dangling.
        assert!(pending.is_empty(), "{pending:?}");

        let scheme = scheme_of(&env, "useEq");
        assert_eq!(scheme.vars.len(), 1);
        assert_eq!(scheme.constraints, vec![(scheme.vars[0], "Eq".to_string())]);
    }

    #[test]
    fn signed_binding_uses_its_signature_as_its_scheme_verbatim() {
        let mut sub = Substitution::new();
        let cs = decls("myMax :: Ord a => a -> a -> a\nmyMax x y = x\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let mut env = SchemeEnv::new();
        let (pending, errors) = solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(pending.is_empty());

        let scheme = scheme_of(&env, "myMax");
        assert_eq!(scheme.vars.len(), 1);
        assert_eq!(
            scheme.constraints,
            vec![(scheme.vars[0], "Ord".to_string())]
        );
    }

    #[test]
    fn body_using_more_than_its_signature_grants_is_a_rigid_instance_error() {
        // myMax's signature only grants Ord, but its body needs Num too.
        let mut sub = Substitution::new();
        let cs = decls("myMax :: Ord a => a -> a -> a\nmyMax x y = x + y\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let mut env = SchemeEnv::new();
        let (_pending, errors) = solve(&mut sub, &mut env, &tree);
        assert!(
            errors.iter().any(|e| matches!(
                &e.kind,
                TypeErrorKind::NoInstanceForRigid { interface } if interface == "Num"
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn zero_argument_binding_with_a_dangling_constraint_is_ambiguous() {
        // `empty :: Monoid a => a` (spec §2.4) -- a real prelude name
        // (`knot-canonical` already resolves it as `Ref::Builtin`), just not
        // yet seeded with its real scheme (TM8). A hand-seeded stand-in
        // scheme is enough to exercise the check: a CAF whose type floats
        // over an unresolved interface with no argument to ever pin it down.
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        let mut env = SchemeEnv::new();
        env.insert(
            SchemeKey::Builtin("empty".to_string()),
            Scheme {
                vars: vec![a],
                constraints: vec![(a, "Monoid".to_string())],
                ty: a,
            },
        );

        let cs = decls("bad = empty\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (_pending, errors) = solve(&mut sub, &mut env, &tree);
        assert!(
            errors.iter().any(|e| matches!(
                &e.kind,
                TypeErrorKind::AmbiguousConstraint { interface } if interface == "Monoid"
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn an_unseeded_builtin_reference_is_an_unbound_value_error() {
        // No TM8 seeding yet -- referencing a real builtin (per
        // knot-canonical's own prelude) that this crate hasn't installed a
        // scheme for is exactly today's gap, not a bug in this milestone.
        let mut sub = Substitution::new();
        let cs = decls("f = compare\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let mut env = SchemeEnv::new();
        let (_pending, errors) = solve(&mut sub, &mut env, &tree);
        assert!(
            errors
                .iter()
                .any(|e| matches!(&e.kind, TypeErrorKind::UnboundValue(name) if name == "compare")),
            "{errors:?}"
        );
    }

    #[test]
    fn instantiate_gives_independent_fresh_copies_each_time() {
        let mut sub = Substitution::new();
        let v = sub.fresh_unbound();
        let scheme = Scheme {
            vars: vec![v],
            constraints: vec![(v, "Num".to_string())],
            ty: v,
        };
        let (ty1, cs1) = instantiate(&mut sub, &scheme);
        let (ty2, cs2) = instantiate(&mut sub, &scheme);
        assert_ne!(ty1, ty2);
        assert_eq!(cs1, vec![(ty1, "Num".to_string())]);
        assert_eq!(cs2, vec![(ty2, "Num".to_string())]);
        // Unifying one instantiation must not affect the other.
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        unify(&mut sub, ty1, int_ty).unwrap();
        assert!(sub.resolve_structure(ty2).is_none());
    }

    #[test]
    fn monomorphic_scheme_instantiates_to_itself_with_no_constraints() {
        let mut sub = Substitution::new();
        let int_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Int".to_string()), vec![]));
        let scheme = Scheme::monomorphic(int_ty);
        let (ty, cs) = instantiate(&mut sub, &scheme);
        assert_eq!(ty, int_ty);
        assert!(cs.is_empty());
    }

    #[test]
    fn ord_given_transitively_implies_eq_given_on_the_same_rigid_var() {
        // `Ord a =>` alone must let the body use `==`, not just `<`/`>` --
        // every real Ord instance is guaranteed an Eq one by coherence, so
        // a rigid `a` only given Ord should already have Eq too (Fix #12).
        let cs = decls("useBoth :: Ord a => a -> a -> Bool\nuseBoth x y = x == y || x < y\n");
        let errors = crate::check::check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn monoid_given_transitively_implies_semigroup_given_on_the_same_rigid_var() {
        let cs = decls("useAppend :: Monoid a => a -> a -> a\nuseAppend x y = x <> y\n");
        let errors = crate::check::check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn integral_given_transitively_implies_both_num_and_eq_given() {
        // Integral's own superclasses are [Num, Ord] -- Eq must come along
        // transitively through Ord, not just Num directly.
        let cs = decls(
            "useAll :: Integral a => a -> a -> a\nuseAll x y = if x == y then x + y else x\n",
        );
        let errors = crate::check::check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn eq_given_still_does_not_imply_ord_given() {
        // Sanity check the fix isn't overly permissive -- Eq has no
        // superclasses of its own, so it must not grant Ord-only operations.
        let cs = decls("useLess :: Eq a => a -> a -> Bool\nuseLess x y = x < y\n");
        let errors = crate::check::check_module(&cs);
        assert!(errors
            .iter()
            .any(|e| matches!(&e.kind, TypeErrorKind::NoInstanceForRigid { interface } if interface == "Ord")));
    }
}
