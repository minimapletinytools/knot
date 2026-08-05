//! Whole-module (and, via `constrain_let_bindings`, `let`-expression)
//! binding-group constraint generation: SCC dependency splitting (plan §5)
//! plus building the resulting nested `Constraint::Let` tree. This is what
//! finally makes a `Ref::TopLevel` reference resolvable at all — see
//! `constrain::expr::constrain_name_ref` for the generation-time half of the
//! story (a self/mutual reference to a binding in the group currently being
//! checked resolves immediately; everything else still defers via `Lookup`).
//!
//! **Rigid variables, for real.** A binding with an explicit signature gets
//! its own declared type variables turned into `Substitution::fresh_rigid`
//! slots (built in TM0/TM1, unused until now) — its params/body are
//! constrained against the signature's own shape directly, so `unify`'s
//! existing rigid-vs-concrete rejection (plan §5) actually gets exercised.
//! A signed binding's resulting scheme is just its signature restated (see
//! `LetMember::declared`), not something `generalize` needs to compute —
//! only unsigned bindings need the general `ftv`-difference algorithm
//! (`solve.rs`, TM5).
//!
//! **`constrain_group_chain`/`constrain_bindings` are generic over `R`**
//! (Fix #3): `finish`'s own return value is threaded straight through,
//! completely uninspected by this module, alongside the `Constraint` tree
//! and — new for Fix #3 — a flat `Vec<LetMember>` covering *every* group in
//! the chain (each member's own `elaborated_body` already filled in). This
//! is what lets `constrain_module` (whose `finish` has no real body to
//! report) and `constrain_let_bindings` (whose `finish` needs to hand back
//! a whole `TypedExpr`) share one implementation: `R` is `TypeVarId` for
//! the former, `TypedExpr` for the latter, and the shared machinery neither
//! knows nor cares which.
//!
//! **`CDecl::Instance` methods are checked too** (Fix #5,
//! `knot-checker-gaps-plan.md`) — `constrain_instance` treats each method
//! as a binding whose "signature" is synthesized from `interface::table::
//! METHODS` instead of user-written, reusing `instantiate_rigid` for the
//! instance's own target (exactly like a signed top-level binding's `ty`)
//! and turning the instance's own `constraints` into `given` facts
//! (`Constraint::Given`) the method bodies get to assume. `CDecl::
//! TypeAlias`/`CDecl::TypeDecl` still need no constraints at all (purely
//! type-level; their own name references were already validated during
//! canonicalization).

use std::collections::{HashMap, HashSet};

use knot_canonical::ast::{
    CDecl, CExpr, CFnDef, CInstanceDecl, CPattern, CType, CTypeSignature, Ref,
};
use knot_syntax::span::{Span, Spanned};

use crate::ast::{TExpr, Typed, TypedExpr};
use crate::constrain::expr::constrain_expr;
use crate::constrain::pattern::constrain_pattern;
use crate::constrain::{Constraint, DeclaredScheme, LetMember, LocalScope};
use crate::interface::table::{ctor_method_shape, method_shape, CtorMethodShape, MethodShape};
use crate::ty::Structure;
use crate::var::{Substitution, TypeVarId};

fn app0(sub: &mut Substitution, name: &str) -> TypeVarId {
    sub.fresh_bound(Structure::App(Ref::Builtin(name.to_string()), vec![]))
}

/// A binding abstracted over its two possible sources: a top-level `CFnDef`
/// (always a plain name, with its own optional signature and param list) or
/// a `let`-binding (a `CPattern` LHS, no separate params or signature at
/// all — Knot's `let` grammar has neither; a local function is written
/// `let f = \x -> ...`, not `let f x = ...`). Only the `Var`-pattern case
/// can meaningfully self/mutually-reference — see `constrain_let_bindings`.
struct RawBinding<'a> {
    name: String,
    span: Span,
    signature: Option<&'a Spanned<CTypeSignature>>,
    params: &'a [Spanned<CPattern>],
    body: &'a Spanned<CExpr>,
}

/// Every `Ref::TopLevel` name reachable anywhere within `expr` — the
/// dependency-graph edges for SCC splitting. Annotation *values* aren't
/// walked: `constrain::expr` doesn't constrain them yet either (TM6), so a
/// dependency only they would introduce isn't real yet — see its own
/// `Annotated` doc comment.
fn collect_top_level_refs(expr: &CExpr, out: &mut HashSet<String>) {
    match expr {
        CExpr::Var(Ref::TopLevel(n)) | CExpr::Ctor(Ref::TopLevel(n)) => {
            out.insert(n.clone());
        }
        CExpr::Var(_)
        | CExpr::Ctor(_)
        | CExpr::IntLit(_)
        | CExpr::FloatLit(_)
        | CExpr::StringLit(_)
        | CExpr::Unit
        | CExpr::Hole => {}
        CExpr::Lambda(params, body) => {
            for p in params {
                collect_pattern_refs(&p.node, out);
            }
            collect_top_level_refs(&body.node, out);
        }
        CExpr::App(f, a) => {
            collect_top_level_refs(&f.node, out);
            collect_top_level_refs(&a.node, out);
        }
        CExpr::BinOp(_, l, r) => {
            collect_top_level_refs(&l.node, out);
            collect_top_level_refs(&r.node, out);
        }
        CExpr::OpRef(_) => {}
        CExpr::Negate(e) => collect_top_level_refs(&e.node, out),
        CExpr::If(c, t, e) => {
            collect_top_level_refs(&c.node, out);
            collect_top_level_refs(&t.node, out);
            collect_top_level_refs(&e.node, out);
        }
        CExpr::Let(bindings, body) => {
            for (p, e) in bindings {
                collect_pattern_refs(&p.node, out);
                collect_top_level_refs(&e.node, out);
            }
            collect_top_level_refs(&body.node, out);
        }
        CExpr::Case(scrutinee, arms) => {
            collect_top_level_refs(&scrutinee.node, out);
            for (p, e) in arms {
                collect_pattern_refs(&p.node, out);
                collect_top_level_refs(&e.node, out);
            }
        }
        CExpr::Do(stmts, final_expr) => {
            for stmt in stmts {
                match stmt {
                    knot_canonical::ast::CDoStmt::Bind(p, e) => {
                        collect_pattern_refs(&p.node, out);
                        collect_top_level_refs(&e.node, out);
                    }
                    knot_canonical::ast::CDoStmt::Expr(e) => collect_top_level_refs(&e.node, out),
                }
            }
            collect_top_level_refs(&final_expr.node, out);
        }
        CExpr::List(es) | CExpr::Tuple(es) => {
            for e in es {
                collect_top_level_refs(&e.node, out);
            }
        }
        CExpr::Record(fields) => {
            for (_, e) in fields {
                collect_top_level_refs(&e.node, out);
            }
        }
        CExpr::RecordUpdate(base, fields) => {
            collect_top_level_refs(&base.node, out);
            for (_, e) in fields {
                collect_top_level_refs(&e.node, out);
            }
        }
        CExpr::FieldAccess(base, _) => collect_top_level_refs(&base.node, out),
        CExpr::Annotated(_annotations, target) => collect_top_level_refs(&target.node, out),
    }
}

fn collect_pattern_refs(pattern: &CPattern, out: &mut HashSet<String>) {
    match pattern {
        CPattern::Ctor(Ref::TopLevel(n), subs) => {
            out.insert(n.clone());
            for s in subs {
                collect_pattern_refs(&s.node, out);
            }
        }
        CPattern::Ctor(_, subs) | CPattern::Tuple(subs) => {
            for s in subs {
                collect_pattern_refs(&s.node, out);
            }
        }
        CPattern::Cons(head, tail) => {
            collect_pattern_refs(&head.node, out);
            collect_pattern_refs(&tail.node, out);
        }
        CPattern::As(inner, _) => collect_pattern_refs(&inner.node, out),
        CPattern::Wildcard(_)
        | CPattern::Var(_)
        | CPattern::Literal(_)
        | CPattern::Nil
        | CPattern::Record(_)
        | CPattern::Unit => {}
    }
}

/// Tarjan's strongly-connected-components, over the subgraph restricted to
/// `names` (an edge to anything outside this set — an unrelated top-level
/// function, a builtin — isn't part of *this* group's own recursion and is
/// left for the ordinary deferred `Lookup` path to handle). Returned in
/// dependency order: a component's own dependencies, if any within `names`,
/// always come *before* it in the result — exactly the order
/// `constrain_bindings` needs to nest `Constraint::Let`s in, since each
/// dependency's `Let` must be the *outer* one (see call site).
fn scc_order(names: &[String], edges: &HashMap<String, HashSet<String>>) -> Vec<Vec<String>> {
    struct State {
        counter: usize,
        indices: HashMap<String, usize>,
        lowlink: HashMap<String, usize>,
        on_stack: HashSet<String>,
        stack: Vec<String>,
        sccs: Vec<Vec<String>>,
    }

    fn strongconnect(name: &str, edges: &HashMap<String, HashSet<String>>, st: &mut State) {
        st.indices.insert(name.to_string(), st.counter);
        st.lowlink.insert(name.to_string(), st.counter);
        st.counter += 1;
        st.stack.push(name.to_string());
        st.on_stack.insert(name.to_string());

        if let Some(deps) = edges.get(name) {
            for dep in deps {
                if !st.indices.contains_key(dep) {
                    strongconnect(dep, edges, st);
                    let merged = st.lowlink[name].min(st.lowlink[dep]);
                    st.lowlink.insert(name.to_string(), merged);
                } else if st.on_stack.contains(dep) {
                    let merged = st.lowlink[name].min(st.indices[dep]);
                    st.lowlink.insert(name.to_string(), merged);
                }
            }
        }

        if st.lowlink[name] == st.indices[name] {
            let mut component = Vec::new();
            loop {
                let w = st.stack.pop().expect("Tarjan stack unexpectedly empty");
                st.on_stack.remove(&w);
                let done = w == name;
                component.push(w);
                if done {
                    break;
                }
            }
            st.sccs.push(component);
        }
    }

    let mut st = State {
        counter: 0,
        indices: HashMap::new(),
        lowlink: HashMap::new(),
        on_stack: HashSet::new(),
        stack: Vec::new(),
        sccs: Vec::new(),
    };
    for name in names {
        if !st.indices.contains_key(name) {
            strongconnect(name, edges, &mut st);
        }
    }
    st.sccs
}

/// Builds the `Structure` a `CType` describes, turning each distinct
/// type-variable (or record row-extension) name into its own rigid
/// `Substitution` slot, shared across every occurrence of that name — see
/// module docs on why this matters for enforcing a signature honestly.
fn instantiate_rigid(
    sub: &mut Substitution,
    ty: &CType,
    rigids: &mut HashMap<String, TypeVarId>,
) -> TypeVarId {
    match ty {
        CType::Named(r, args) => {
            let arg_tys = args
                .iter()
                .map(|a| instantiate_rigid(sub, a, rigids))
                .collect();
            sub.fresh_bound(Structure::App(r.clone(), arg_tys))
        }
        CType::Var(name) => rigid_var(sub, rigids, name),
        // `f a` (spec §10.6) -- `f` itself becomes a rigid var exactly like
        // an ordinary type variable (`rigid_var`'s own shared `rigids` map
        // guarantees the same TypeVarId across every mention of `f` within
        // this one signature, so `Collection f => f a -> f b`'s two `f`s
        // agree), then wrapped around its own recursively-instantiated
        // arguments into a `Structure::VarApp` -- the same shape `unify.rs`
        // and `check_instance`'s rigid-defers-to-`given` case already know
        // how to handle, since it's the same one the hand-written
        // `Collection`/`Context` prelude schemes have always produced.
        CType::VarApp(name, args) => {
            let f_var = rigid_var(sub, rigids, name);
            let arg_tys = args
                .iter()
                .map(|a| instantiate_rigid(sub, a, rigids))
                .collect();
            sub.fresh_bound(Structure::VarApp(f_var, arg_tys))
        }
        CType::Fn(a, b) => {
            let a_ty = instantiate_rigid(sub, a, rigids);
            let b_ty = instantiate_rigid(sub, b, rigids);
            sub.fresh_bound(Structure::Fn(a_ty, b_ty))
        }
        CType::Tuple(ts) => {
            let tys = ts
                .iter()
                .map(|t| instantiate_rigid(sub, t, rigids))
                .collect();
            sub.fresh_bound(Structure::Tuple(tys))
        }
        CType::Record(fields, spreads, ext) => {
            debug_assert!(
                spreads.is_empty(),
                "canonicalization always resolves every record spread away \
                 before knot-checker ever sees a CType"
            );
            let field_tys = fields
                .iter()
                .map(|(name, t)| (name.clone(), instantiate_rigid(sub, t, rigids)))
                .collect();
            let ext_ty = ext.as_ref().map(|name| rigid_var(sub, rigids, name));
            sub.fresh_bound(Structure::Record(field_tys, ext_ty))
        }
        CType::Unit => sub.fresh_bound(Structure::Unit),
    }
}

fn rigid_var(
    sub: &mut Substitution,
    rigids: &mut HashMap<String, TypeVarId>,
    name: &str,
) -> TypeVarId {
    *rigids
        .entry(name.to_string())
        .or_insert_with(|| sub.fresh_rigid(name.to_string()))
}

/// Generation-time counterpart of a signature: the expected type to check a
/// signed binding's body against, its rigid variables, and its `given`
/// interface obligations (see `DeclaredScheme`).
fn build_declared(sub: &mut Substitution, sig: &CTypeSignature) -> (TypeVarId, DeclaredScheme) {
    let mut rigids = HashMap::new();
    let expected_ty = instantiate_rigid(sub, &sig.ty, &mut rigids);
    let given = sig
        .constraints
        .iter()
        .map(|c| {
            (
                rigid_var(sub, &mut rigids, &c.type_var),
                c.interface.clone(),
            )
        })
        .collect();
    (
        expected_ty,
        DeclaredScheme {
            rigid_vars: rigids.into_values().collect(),
            given,
        },
    )
}

/// Same walk as `instantiate_rigid`, but for a context that needs genuinely
/// quantifiable, ordinary flexible variables instead of rigid ones — a
/// constructor's own type parameters aren't being checked against a body
/// the way a signature's are, they're just being *given* a type, so there's
/// nothing for rigidity to protect here. Kept as its own small function
/// rather than parameterizing `instantiate_rigid` over "how to make a
/// fresh var": the two are similar, but different enough in *why* they
/// exist that sharing one implementation would obscure which is which.
fn instantiate_flexible(
    sub: &mut Substitution,
    ty: &CType,
    vars: &mut HashMap<String, TypeVarId>,
) -> TypeVarId {
    match ty {
        CType::Named(r, args) => {
            let arg_tys = args
                .iter()
                .map(|a| instantiate_flexible(sub, a, vars))
                .collect();
            sub.fresh_bound(Structure::App(r.clone(), arg_tys))
        }
        CType::Var(name) => *vars
            .entry(name.clone())
            .or_insert_with(|| sub.fresh_unbound()),
        // Mirrors `instantiate_rigid`'s own `VarApp` case, just flexible
        // (`fresh_ctor_unbound`) instead of rigid -- e.g. a data
        // constructor field typed `f a` for a type declared `type Wrapper f
        // a = Wrap (f a)`.
        CType::VarApp(name, args) => {
            let f_var = *vars
                .entry(name.clone())
                .or_insert_with(|| sub.fresh_ctor_unbound());
            let arg_tys = args
                .iter()
                .map(|a| instantiate_flexible(sub, a, vars))
                .collect();
            sub.fresh_bound(Structure::VarApp(f_var, arg_tys))
        }
        CType::Fn(a, b) => {
            let a_ty = instantiate_flexible(sub, a, vars);
            let b_ty = instantiate_flexible(sub, b, vars);
            sub.fresh_bound(Structure::Fn(a_ty, b_ty))
        }
        CType::Tuple(ts) => {
            let tys = ts
                .iter()
                .map(|t| instantiate_flexible(sub, t, vars))
                .collect();
            sub.fresh_bound(Structure::Tuple(tys))
        }
        CType::Record(fields, spreads, ext) => {
            debug_assert!(
                spreads.is_empty(),
                "canonicalization always resolves every record spread away \
                 before knot-checker ever sees a CType"
            );
            let field_tys = fields
                .iter()
                .map(|(name, t)| (name.clone(), instantiate_flexible(sub, t, vars)))
                .collect();
            let ext_ty = ext.as_ref().map(|name| {
                *vars
                    .entry(name.clone())
                    .or_insert_with(|| sub.fresh_unbound())
            });
            sub.fresh_bound(Structure::Record(field_tys, ext_ty))
        }
        CType::Unit => sub.fresh_bound(Structure::Unit),
    }
}

/// Seeds `env` with a `Scheme` for every constructor of every `CDecl::
/// TypeDecl` in `decls` — e.g. `type Shape = Circle Float` installs
/// `Circle :: Float -> Shape` under `SchemeKey::TopLevel("Circle")`,
/// exactly the shape `prelude.rs`'s own `seed_constructors` hand-writes for
/// built-ins like `Just`/`Ok`, just derived from the real declaration
/// instead of hardcoded — nothing else in this crate did this at all before
/// (a real, previously-undocumented gap: without it, *any* use of a
/// user-defined constructor as a value or in a pattern is an `UnboundValue`
/// error, since `Ref::TopLevel`-headed `Lookup`/pattern-`Lookup`
/// constraints have nowhere to resolve against).
///
/// Needs no unification or generalization at all: a constructor's own type
/// is completely determined by its declaration (this language has no
/// syntax for constraining an ADT's own type parameters, unlike a
/// function's signature), so this installs a finished `Scheme` directly
/// rather than routing through `Constraint`/`solve.rs` the way `constrain_
/// module`'s ordinary bindings do. Every one of the type's own declared
/// parameters gets its own quantified variable, even one a particular
/// variant's fields never mention (`type Result e a = Ok a | Err e`'s `Ok`
/// still needs to be quantified over `e`, or its constructed value's own
/// type wouldn't actually be `Result e a` in full).
///
/// Only *local* declarations are covered, the same limitation `resolve::
/// alias::expand_aliases` documents for imported type aliases: a real gap
/// once cross-module linking exists, not something fixable here.
pub fn seed_user_constructors(
    sub: &mut Substitution,
    env: &mut crate::solve::SchemeEnv,
    decls: &[Spanned<CDecl>],
) {
    for d in decls {
        let CDecl::TypeDecl(type_name, params, variants, _deriving) = &d.node else {
            continue;
        };
        for (ctor_name, field_types) in variants {
            let mut vars = HashMap::new();
            let field_tys: Vec<TypeVarId> = field_types
                .iter()
                .map(|t| instantiate_flexible(sub, t, &mut vars))
                .collect();
            for p in params {
                vars.entry(p.clone()).or_insert_with(|| sub.fresh_unbound());
            }
            let param_vars: Vec<TypeVarId> = params.iter().map(|p| vars[p]).collect();
            let result_ty =
                sub.fresh_bound(Structure::App(Ref::TopLevel(type_name.clone()), param_vars));
            let ctor_ty = field_tys
                .iter()
                .rev()
                .fold(result_ty, |acc, &f| sub.fresh_bound(Structure::Fn(f, acc)));
            env.insert(
                crate::solve::SchemeKey::TopLevel(ctor_name.clone()),
                crate::ty::Scheme {
                    vars: vars.into_values().collect(),
                    constraints: vec![],
                    ty: ctor_ty,
                },
            );
        }
    }
}

/// A group's own header info, before its members' bodies are constrained —
/// split out from `LetMember` itself only because `LetMember.elaborated_body`
/// doesn't exist yet at this point (every member's `header_ty` must be bound
/// into `scope` *before* any of their bodies are constrained, so self/mutual
/// references resolve — see `constrain_group_chain`).
struct Header {
    name: String,
    span: Span,
    header_ty: TypeVarId,
    declared: Option<DeclaredScheme>,
}

/// Splits `bindings` into SCCs and builds one nested `Constraint::Let` per
/// component, in dependency order. `finish` produces whatever comes after
/// every group — the rest of the module (just `Constraint::True`), or a
/// `let`'s own `in` expression — and is invoked, along with every later
/// group in the chain, *while all earlier groups' names are still in
/// `scope`* (see `constrain_group_chain`): that's what lets a later
/// binding, or the final body, reference an earlier one, and it's the bug
/// an earlier version of this function had (`scope` was pushed and popped
/// entirely *within* one group's own processing, so nothing after it could
/// ever see those names — building `body_con` from an already-finished
/// `Constraint` value, rather than a continuation still able to run inside
/// the right scope, made that impossible to get right).
fn constrain_bindings<R>(
    sub: &mut Substitution,
    scope: &mut LocalScope,
    bindings: Vec<RawBinding>,
    top_level: bool,
    finish: impl FnOnce(&mut Substitution, &mut LocalScope) -> (R, Constraint),
) -> (R, Constraint, Vec<LetMember>) {
    let names: Vec<String> = bindings.iter().map(|b| b.name.clone()).collect();
    let name_set: HashSet<String> = names.iter().cloned().collect();

    let mut edges = HashMap::new();
    for b in &bindings {
        let mut refs = HashSet::new();
        for p in b.params {
            collect_pattern_refs(&p.node, &mut refs);
        }
        collect_top_level_refs(&b.body.node, &mut refs);
        refs.retain(|n| name_set.contains(n));
        edges.insert(b.name.clone(), refs);
    }
    let mut by_name = HashMap::new();
    for b in &bindings {
        by_name.insert(b.name.clone(), b);
    }

    let sccs = scc_order(&names, &edges);
    constrain_group_chain(sub, scope, &by_name, &sccs, 0, top_level, finish)
}

/// Processes `sccs[index..]` recursively: pushes the current group's names,
/// recurses for the rest of the chain (and eventually `finish`) *before*
/// popping them back off, then wraps the recursive result in this group's
/// own `Constraint::Let` on the way back out. Dependency order (`scc_order`)
/// plus this push-recurse-pop shape together guarantee a group's names stay
/// visible for exactly as long as they should: every later group, and the
/// final body, but nothing before or outside this whole chain.
///
/// Returns this chain's own `R` (whatever `finish` produced, passed straight
/// through untouched), the nested `Constraint::Let` tree, and *every* group's
/// `LetMember`s flattened into one `Vec` (Fix #3) — collected directly here,
/// rather than by re-walking the returned `Constraint` tree afterward, since
/// a body's own constraints can contain unrelated, more-deeply-nested
/// `Constraint::Let`s (from an inner `let`-expression) that a tree-walk could
/// not safely tell apart from this chain's own groups.
fn constrain_group_chain<R>(
    sub: &mut Substitution,
    scope: &mut LocalScope,
    by_name: &HashMap<String, &RawBinding>,
    sccs: &[Vec<String>],
    index: usize,
    top_level: bool,
    finish: impl FnOnce(&mut Substitution, &mut LocalScope) -> (R, Constraint),
) -> (R, Constraint, Vec<LetMember>) {
    let Some(group_names) = sccs.get(index) else {
        let (r, c) = finish(sub, scope);
        return (r, c, Vec::new());
    };
    let group: Vec<&RawBinding> = group_names.iter().map(|n| by_name[n]).collect();

    scope.push();
    let mut headers = Vec::new();
    for b in &group {
        let (header_ty, declared) = match b.signature {
            Some(sig) => {
                let (ty, declared) = build_declared(sub, &sig.node);
                (ty, Some(declared))
            }
            None => (sub.fresh_unbound(), None),
        };
        scope.bind(&b.name, header_ty);
        headers.push(Header {
            name: b.name.clone(),
            span: b.span,
            header_ty,
            declared,
        });
    }

    let mut header_constraints = Vec::new();
    let mut members = Vec::new();
    for (b, header) in group.iter().zip(headers) {
        scope.push();
        let mut body_constraints = Vec::new();
        let typed_params: Vec<crate::ast::TypedPattern> = b
            .params
            .iter()
            .map(|p| constrain_pattern(sub, scope, p, &mut body_constraints))
            .collect();
        let typed_body = constrain_expr(sub, scope, b.body, &mut body_constraints);
        scope.pop();
        let inferred = typed_params.iter().rev().fold(typed_body.ty, |acc, p| {
            sub.fresh_bound(Structure::Fn(p.ty, acc))
        });
        // Solve the header/signature <-> inferred-type link *before*
        // `body_constraints`, not after (Fix #13): with a signature present,
        // `header.header_ty` is already the rigid type built by
        // `build_declared`, but each param pattern's own type (`p.ty`, folded
        // into `inferred` above) is still a fresh, unconnected unbound
        // variable at this point -- `constrain_pattern` never consults the
        // signature. If `body_constraints` (which may contain nested `let`s,
        // per `Expr::Let`'s own recursive use of this same function) were
        // solved first, as they were before this fix, any such nested `let`
        // generalizing over a param-derived variable would see it as a
        // fresh var absent from `ambient` -- since it hasn't been unified
        // into the signature's rigid structure yet -- and wrongly treat it
        // as newly quantifiable, dragging along whatever interface
        // obligation it carried (e.g. `Ord`) into the nested binding's own
        // scheme and tripping `check_ambiguous` on an ordinary zero-arg
        // `let` like a hand-rolled quicksort's `smaller`/`larger`. Solving
        // this `Equal` first unifies the param variable's root with the
        // rigid one immediately, so by the time `body_constraints` runs it's
        // already indistinguishable from `header.header_ty` itself: visible
        // in `ambient`, correctly excluded from generalization, and already
        // covered by this signature's own `given`.
        header_constraints.push(Constraint::Equal {
            span: b.span,
            expected: header.header_ty,
            actual: inferred,
        });
        header_constraints.extend(body_constraints);
        // A zero-param binding's own value *is* its body; otherwise it's
        // the params folded into one `Lambda` wrapping that body -- the
        // curried `Fn` chain above, mirrored one layer up in `TExpr` shape.
        let elaborated_body = if typed_params.is_empty() {
            typed_body
        } else {
            Typed {
                span: b.span,
                ty: header.header_ty,
                node: TExpr::Lambda(typed_params, Box::new(typed_body)),
            }
        };
        members.push(LetMember {
            name: header.name,
            span: header.span,
            header_ty: header.header_ty,
            declared: header.declared,
            elaborated_body,
        });
    }

    // Top-level names must be visible to *everything after this group* only
    // through the deferred `Lookup` + generalized-scheme path (installed by
    // `solve.rs`), never as a raw `LocalScope` hit -- that's the whole point
    // of let-polymorphism (each use gets its own fresh instantiation). So
    // this frame closes *now*, before recursing into later groups/the final
    // body, not after.
    //
    // A `let`-expression's names (`top_level: false`) need the *same*
    // polymorphism, but can't go through `Lookup` -- `Ref::Local` is what
    // `knot-canonical` gives a reference to a `let`-bound name, same as a
    // lambda param, so it has to keep resolving via `LocalScope` (see
    // `constrain::expr::constrain_name_ref`). The frame therefore stays
    // open through the rest of the chain (a later sibling/the final `in`
    // body must still be able to see these names at all), but each member
    // is promoted from `Monomorphic` to `Generalizable` first: every
    // reference from here on emits its own `Constraint::LookupLocal`
    // instead of reusing this group's own header type variable directly.
    if top_level {
        scope.pop();
    } else {
        for member in &members {
            scope.promote_to_generalizable(&member.name);
        }
    }
    let (result, body_con, mut rest_members) =
        constrain_group_chain(sub, scope, by_name, sccs, index + 1, top_level, finish);
    if !top_level {
        scope.pop();
    }

    let mut all_members = members.clone();
    all_members.append(&mut rest_members);

    let whole = Constraint::Let {
        top_level,
        members,
        header_con: Box::new(Constraint::And(header_constraints)),
        body_con: Box::new(body_con),
    };
    (result, whole, all_members)
}

/// Builds the `Structure` a `MethodShape` describes, substituting `self_ty`
/// (the instance's own target, already built via `instantiate_rigid`) for
/// every `MethodShape::SelfTy` — the same "instantiate a type template
/// against something concrete" idea `instantiate_rigid` itself is, just
/// over `MethodShape` instead of `CType`.
fn instantiate_method_shape(
    sub: &mut Substitution,
    shape: &MethodShape,
    self_ty: TypeVarId,
) -> TypeVarId {
    match shape {
        MethodShape::SelfTy => self_ty,
        MethodShape::Fn(a, b) => {
            let a_ty = instantiate_method_shape(sub, a, self_ty);
            let b_ty = instantiate_method_shape(sub, b, self_ty);
            sub.fresh_bound(Structure::Fn(a_ty, b_ty))
        }
        MethodShape::Bool => app0(sub, "Bool"),
        MethodShape::Ordering => app0(sub, "Ordering"),
        MethodShape::StringTy => app0(sub, "String"),
    }
}

/// `instantiate_method_shape`'s `CtorMethodShape` counterpart — `self_ctor`
/// is the concrete constructor a `Collection`/`Context` instance targets
/// (e.g. `List`), substituted in wherever `SelfApp` applies it to
/// something; `vars` collects this *one method's* own extra type variables
/// (`map`'s `a`/`b`), fresh per call so two different methods never
/// accidentally share one just because they reuse the same letter.
fn instantiate_ctor_method_shape(
    sub: &mut Substitution,
    shape: &CtorMethodShape,
    self_ctor: &Ref,
    vars: &mut HashMap<String, TypeVarId>,
) -> TypeVarId {
    match shape {
        CtorMethodShape::Var(name) => *vars
            .entry((*name).to_string())
            .or_insert_with(|| sub.fresh_unbound()),
        CtorMethodShape::Fn(a, b) => {
            let a_ty = instantiate_ctor_method_shape(sub, a, self_ctor, vars);
            let b_ty = instantiate_ctor_method_shape(sub, b, self_ctor, vars);
            sub.fresh_bound(Structure::Fn(a_ty, b_ty))
        }
        CtorMethodShape::SelfApp(arg) => {
            let arg_ty = instantiate_ctor_method_shape(sub, arg, self_ctor, vars);
            sub.fresh_bound(Structure::App(self_ctor.clone(), vec![arg_ty]))
        }
        CtorMethodShape::Bool => app0(sub, "Bool"),
        CtorMethodShape::IntTy => app0(sub, "Int"),
    }
}

/// The bare constructor a `Collection`/`Context` instance's own `target`
/// names — `instance Collection List` targets just `List`, unapplied
/// (unlike `instance Eq Shape`, where the target is a fully-formed type),
/// so this is looking for a `CType::Named` with *no* arguments at all,
/// rather than building any `Structure` from it the way `instantiate_rigid`
/// would. `None` for anything else (a malformed target, e.g. `instance
/// Collection (List Int)`) — silently unchecked, matching `constrain_
/// instance`'s own stance on a method name it doesn't recognize.
fn ctor_target(target: &CType) -> Option<&Ref> {
    match target {
        CType::Named(r, args) if args.is_empty() => Some(r),
        _ => None,
    }
}

/// Checks every method of `inst` against its interface's own `MethodShape`
/// (Fix #5) — the target's own type variables become rigid (reusing
/// `instantiate_rigid`, exactly like an ordinary signed binding's own
/// `ty`), the instance's own `constraints` become `given` facts
/// (`Constraint::Given`) the bodies get to assume, and each method's body
/// is constrained against its shape instantiated with the target
/// substituted for `Self` — exactly like a signed top-level binding's body
/// is constrained against its own signature, just without needing
/// `RawBinding`'s borrowed-signature shape at all (there's no parsed
/// `CTypeSignature` here to borrow — the "signature" is synthesized).
/// `Collection`/`Context` instances go through `constrain_ctor_instance`
/// instead — a genuinely different shape of check, see its own doc comment.
///
/// A method whose name has no entry in the relevant shape table (an
/// interface neither table covers at all, or a genuinely bogus method
/// name) is silently skipped, not type-checked at all: validating that an
/// instance's method list exactly matches its interface is a separate,
/// narrower concern this doesn't attempt.
///
/// Deliberately doesn't thread an `elaborated_body` out anywhere (unlike
/// `constrain_group_chain`'s members): `constrain_pattern`/`constrain_expr`
/// still build one as a normal by-product, but nothing consumes it yet —
/// an instance method doesn't fit `LetMember`'s shape (it's never
/// generalized, never installed into a scheme environment, and dispatched
/// by `(interface, head)` rather than by name), so giving it a proper home
/// in the elaborated output is real, separate follow-up work, not
/// something to force into the existing shape as a shortcut.
fn constrain_instance(sub: &mut Substitution, inst: &CInstanceDecl) -> Constraint {
    if matches!(inst.interface.as_str(), "Collection" | "Context") {
        return constrain_ctor_instance(sub, inst);
    }

    let mut rigids = HashMap::new();
    let target_ty = instantiate_rigid(sub, &inst.target, &mut rigids);
    let given: Vec<(TypeVarId, String)> = inst
        .constraints
        .iter()
        .map(|c| {
            (
                rigid_var(sub, &mut rigids, &c.type_var),
                c.interface.clone(),
            )
        })
        .collect();

    let mut method_constraints = Vec::new();
    for method in &inst.methods {
        let Some(shape) = method_shape(&inst.interface, &method.name) else {
            continue;
        };
        let expected_ty = instantiate_method_shape(sub, shape, target_ty);
        method_constraints.push(constrain_method_body_against(sub, method, expected_ty));
    }

    Constraint::Given {
        given,
        body: Box::new(Constraint::And(method_constraints)),
    }
}

/// `constrain_instance`'s own shape, for `Collection`/`Context` (Fix #5's
/// remaining, now-closed gap): `Self` is the instance's own bare
/// constructor target (`ctor_target`), never a rigid ordinary type
/// variable, and each method gets its *own* fresh set of extra type
/// variables (`instantiate_ctor_method_shape`'s own `vars` map, started
/// fresh per method) rather than sharing one rigid-variable pool the way
/// an ordinary instance's methods all share its target's own rigids —
/// `map`'s `a`/`b` have nothing to do with `filter`'s own `a`. No `given`
/// facts are possible here at all: the target has no type-variable
/// argument (it's an unapplied constructor) for a `Foo a =>` clause to
/// even attach to, so unlike the ordinary path this is never wrapped in a
/// `Constraint::Given`.
fn constrain_ctor_instance(sub: &mut Substitution, inst: &CInstanceDecl) -> Constraint {
    let Some(self_ctor) = ctor_target(&inst.target) else {
        return Constraint::True;
    };
    let self_ctor = self_ctor.clone();

    let mut method_constraints = Vec::new();
    for method in &inst.methods {
        let Some(shape) = ctor_method_shape(&inst.interface, &method.name) else {
            continue;
        };
        let mut vars = HashMap::new();
        let expected_ty = instantiate_ctor_method_shape(sub, shape, &self_ctor, &mut vars);
        method_constraints.push(constrain_method_body_against(sub, method, expected_ty));
    }

    Constraint::And(method_constraints)
}

/// Constrains one method's params + body against `expected_ty`, exactly
/// like a signed top-level binding's body is checked against its
/// signature — shared between `constrain_instance`'s two shapes (ordinary
/// interfaces and `Collection`/`Context`), which only differ in *how*
/// `expected_ty` itself gets built.
fn constrain_method_body_against(
    sub: &mut Substitution,
    method: &CFnDef,
    expected_ty: TypeVarId,
) -> Constraint {
    let mut scope = LocalScope::new();
    scope.push();
    let mut body_constraints = Vec::new();
    let typed_params: Vec<crate::ast::TypedPattern> = method
        .params
        .iter()
        .map(|p| constrain_pattern(sub, &mut scope, p, &mut body_constraints))
        .collect();
    let typed_body = constrain_expr(sub, &mut scope, &method.body, &mut body_constraints);
    scope.pop();

    let inferred = typed_params.iter().rev().fold(typed_body.ty, |acc, p| {
        sub.fresh_bound(Structure::Fn(p.ty, acc))
    });
    // Solve the header/signature <-> inferred-type link *before*
    // `body_constraints`, not after (same fix as Fix #13's own
    // `constrain_group_chain`, applied here too since instance methods
    // build this same shape independently): `expected_ty` is built from
    // the instance's own rigid variables (`instantiate_method_shape`/
    // `instantiate_ctor_method_shape`, sharing `constrain_instance`'s own
    // `rigids` map), but each param pattern's own type is still a fresh,
    // unconnected unbound variable at this point. If `body_constraints`
    // were solved first, a nested `let` inside the method body
    // generalizing over a param-derived variable would see it as
    // ambient-free rather than the (soon-to-be) rigid one this instance
    // declares, wrongly making it look quantifiable -- misfiring
    // `AmbiguousConstraint` on an ordinary local `let` inside a method
    // body, e.g. a `Show` instance computing intermediate values via
    // `div`/`mod` before formatting them.
    let mut constraints = vec![Constraint::Equal {
        span: method.body.span,
        expected: expected_ty,
        actual: inferred,
    }];
    constraints.extend(body_constraints);
    Constraint::And(constraints)
}

/// The whole-module entry point: every `CDecl::Fn` becomes a `RawBinding`;
/// SCC-split and wrapped in nested `Constraint::Let`s. There's no module
/// "body" the way a `let...in` has one — solving a module is entirely about
/// populating the scheme environment these `Let`s build, so the innermost
/// constraint is trivially `Constraint::True`. Every `CDecl::Instance`'s own
/// methods (Fix #5) are checked too, each producing its own `Constraint::
/// Given`, combined with the `Fn` chain via `Constraint::And`. The returned
/// `Vec<LetMember>` covers every top-level `Fn` binding (not instance
/// methods — see `constrain_instance`'s own doc comment), each with its own
/// `elaborated_body` (Fix #3) already filled in.
pub fn constrain_module(
    sub: &mut Substitution,
    decls: &[Spanned<CDecl>],
) -> (Constraint, Vec<LetMember>) {
    let bindings: Vec<RawBinding> = decls
        .iter()
        .filter_map(|d| match &d.node {
            CDecl::Fn(fndef) => Some(RawBinding {
                name: fndef.name.clone(),
                span: d.span,
                signature: fndef.signature.as_ref(),
                params: &fndef.params,
                body: &fndef.body,
            }),
            CDecl::TypeAlias(..) | CDecl::TypeDecl(..) | CDecl::Instance(_) => None,
        })
        .collect();
    let mut scope = LocalScope::new();
    let (_unused, tree, members) = constrain_bindings(sub, &mut scope, bindings, true, |sub, _| {
        (sub.fresh_unbound(), Constraint::True)
    });

    let instance_constraints: Vec<Constraint> = decls
        .iter()
        .filter_map(|d| match &d.node {
            CDecl::Instance(inst) => Some(constrain_instance(sub, inst)),
            _ => None,
        })
        .collect();

    let combined = if instance_constraints.is_empty() {
        tree
    } else {
        let mut all = vec![tree];
        all.extend(instance_constraints);
        Constraint::And(all)
    };
    (combined, members)
}

/// `CExpr::Let`'s own binding list. Only a plain `Var` pattern can
/// meaningfully self/mutually-reference (see module docs), so those go
/// through the same SCC machinery as top-level bindings, unsigned always
/// (Knot's `let` grammar has no signature syntax). Any other pattern
/// (`Tuple`, `Ctor`, ...) can't recurse into its own right-hand side at all
/// — `let (x, y) = (x, y) in ...` needs `x`/`y` to already exist to define
/// them, which is nonsensical — so those are constrained directly against
/// the pattern, outside the SCC grouping entirely, each its own trivial
/// non-recursive step.
///
/// Returns the whole `let`-expression, fully elaborated (Fix #3): a flat
/// `TExpr::Let` combining every `Var`-pattern member's own `elaborated_body`
/// (recovered from the `Vec<LetMember>` `constrain_bindings` hands back)
/// with every non-`Var`-pattern binding's typed form, built directly inside
/// `finish` below. Member order in the result isn't guaranteed to match
/// source order (`Var`-pattern members, in whatever order their own SCC
/// chain produced them, come first) -- harmless, since Knot's `let`
/// bindings are simultaneous/mutually-visible within their own scope, not
/// sequential statements, and canonicalization already rejects duplicate
/// names within one `let` block.
pub fn constrain_let_bindings(
    sub: &mut Substitution,
    scope: &mut LocalScope,
    bindings: &[(Spanned<CPattern>, Spanned<CExpr>)],
    constrain_body: impl FnOnce(&mut Substitution, &mut LocalScope, &mut Vec<Constraint>) -> TypedExpr,
) -> (TypedExpr, Constraint) {
    let (var_bindings, other_bindings): (Vec<_>, Vec<_>) = bindings
        .iter()
        .partition(|(p, _)| matches!(p.node, CPattern::Var(_)));

    let raw: Vec<RawBinding> = var_bindings
        .iter()
        .map(|(p, e)| {
            let CPattern::Var(name) = &p.node else {
                unreachable!("filtered to Var patterns above")
            };
            RawBinding {
                name: name.clone(),
                span: p.span,
                signature: None,
                params: &[],
                body: e,
            }
        })
        .collect();

    // `constrain_bindings` handles `raw` being empty correctly on its own
    // (the SCC split is trivially empty, so `finish` runs immediately in
    // the unmodified scope) — no separate empty-case branch needed here.
    //
    // Non-`Var` patterns are deliberately handled *inside* `finish`, not
    // before this call: they run after every `Var`-pattern (function)
    // sibling is already in scope, so e.g. `let (a, b) = (1, 2); f = \x ->
    // a + x in f b` resolves `a` correctly. The reverse isn't true — a
    // `Var`-pattern binding's own body can't see a `Tuple`/`Ctor`-pattern
    // sibling's names, since those aren't bound until `finish` runs, which
    // is after every `Var`-pattern group has already been constrained. A
    // narrow, documented gap rather than full Haskell-style
    // every-binding-sees-every-binding visibility, which would need
    // predeclaring placeholder types for destructured names too — not worth
    // the complexity for how rarely a let block actually needs it.
    let (other_result, tree, var_members) =
        constrain_bindings(sub, scope, raw, false, |sub, scope| {
            let mut constraints = Vec::new();
            let mut other_typed = Vec::new();
            for (p, e) in &other_bindings {
                let typed_rhs = constrain_expr(sub, scope, e, &mut constraints);
                let typed_pat = constrain_pattern(sub, scope, p, &mut constraints);
                constraints.push(Constraint::Equal {
                    span: p.span,
                    expected: typed_pat.ty,
                    actual: typed_rhs.ty,
                });
                other_typed.push((typed_pat, typed_rhs));
            }
            let typed_body = constrain_body(sub, scope, &mut constraints);
            ((other_typed, typed_body), Constraint::And(constraints))
        });
    let (mut other_typed, typed_body) = other_result;

    let mut all_bindings: Vec<(crate::ast::TypedPattern, TypedExpr)> = var_members
        .into_iter()
        .map(|m| {
            (
                Typed {
                    span: m.span,
                    ty: m.header_ty,
                    node: crate::ast::TPattern::Var(m.name),
                },
                m.elaborated_body,
            )
        })
        .collect();
    all_bindings.append(&mut other_typed);

    let span = typed_body.span;
    let ty = typed_body.ty;
    let elaborated = Typed {
        span,
        ty,
        node: TExpr::Let(all_bindings, Box::new(typed_body)),
    };
    (elaborated, tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use knot_canonical::ast::CDecl;

    fn decls(src: &str) -> Vec<Spanned<CDecl>> {
        let mut state = knot_syntax::ParseState::new(src);
        let raw = state.parse_decls().unwrap();
        assert!(state.is_eof(), "leftover input: {src}");
        knot_canonical::canonicalize_decls(&raw).unwrap_or_else(|errs| panic!("{errs:?}"))
    }

    fn as_let(c: &Constraint) -> (&[LetMember], &Constraint, &Constraint) {
        match c {
            Constraint::Let {
                top_level,
                members,
                header_con,
                body_con,
            } => {
                assert!(
                    *top_level,
                    "constrain_module should always produce top_level: true"
                );
                (members, header_con, body_con)
            }
            other => panic!("expected a Let, got {other:?}"),
        }
    }

    #[test]
    fn single_unsigned_binding_is_its_own_group_with_a_trivial_body() {
        let mut sub = Substitution::new();
        let cs = decls("id x = x\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (members, _header, body) = as_let(&tree);
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].name, "id");
        assert!(members[0].declared.is_none());
        assert_eq!(*body, Constraint::True);
        // The elaborated body should be a one-param Lambda wrapping `x`.
        assert!(matches!(
            members[0].elaborated_body.node,
            TExpr::Lambda(ref params, _) if params.len() == 1
        ));
    }

    #[test]
    fn mutually_recursive_bindings_share_one_scc_group() {
        let mut sub = Substitution::new();
        let cs = decls(
            "isOdd n = if n == 0 then False else isEven (n - 1)\n\
             isEven n = if n == 0 then True else isOdd (n - 1)\n",
        );
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (members, _header, body) = as_let(&tree);
        let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"isOdd"));
        assert!(names.contains(&"isEven"));
        assert_eq!(*body, Constraint::True);
    }

    #[test]
    fn a_dependency_becomes_the_outer_group_and_its_user_the_inner_one() {
        let mut sub = Substitution::new();
        let cs = decls("helper x = x\nuseHelper y = helper y\n");
        let (tree, all_members) = constrain_module(&mut sub, &cs);
        let (outer_members, _outer_header, inner) = as_let(&tree);
        assert_eq!(outer_members.len(), 1);
        assert_eq!(outer_members[0].name, "helper");
        let (inner_members, _inner_header, innermost) = as_let(inner);
        assert_eq!(inner_members.len(), 1);
        assert_eq!(inner_members[0].name, "useHelper");
        assert_eq!(*innermost, Constraint::True);
        // The flattened member list (Fix #3) should carry both.
        let all_names: Vec<&str> = all_members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(all_names, vec!["helper", "useHelper"]);
    }

    #[test]
    fn signed_binding_gets_rigid_vars_and_given_constraints() {
        let mut sub = Substitution::new();
        let cs = decls("myMax :: Ord a => a -> a -> a\nmyMax x y = x\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (members, _header, _body) = as_let(&tree);
        assert_eq!(members.len(), 1);
        let declared = members[0].declared.as_ref().expect("myMax has a signature");
        assert_eq!(declared.rigid_vars.len(), 1);
        assert_eq!(
            declared.given,
            vec![(declared.rigid_vars[0], "Ord".to_string())]
        );

        // header_ty should be exactly `a -> a -> a` with every `a` the same rigid var.
        let rigid_a = declared.rigid_vars[0];
        match sub.resolve_structure(members[0].header_ty) {
            Some(Structure::Fn(arg1, rest)) => {
                assert_eq!(arg1, rigid_a);
                match sub.resolve_structure(rest) {
                    Some(Structure::Fn(arg2, ret)) => {
                        assert_eq!(arg2, rigid_a);
                        assert_eq!(ret, rigid_a);
                    }
                    other => panic!("expected a second Fn arrow, got {other:?}"),
                }
            }
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    #[test]
    fn self_recursive_binding_is_a_singleton_group_via_scope_not_lookup() {
        let mut sub = Substitution::new();
        let cs = decls("loop x = loop x\n");
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (members, header, _body) = as_let(&tree);
        assert_eq!(members.len(), 1);
        // The self-call should resolve monomorphically (no deferred Lookup
        // for `loop` itself) -- only ordinary Equal constraints expected.
        fn all_equal(c: &Constraint) -> bool {
            match c {
                Constraint::Equal { .. } => true,
                Constraint::And(cs) => cs.iter().all(all_equal),
                _ => false,
            }
        }
        assert!(
            all_equal(header),
            "expected only Equal constraints, got {header:?}"
        );
    }

    #[test]
    fn var_pattern_let_binding_produces_a_non_top_level_let() {
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let span = knot_syntax::span::Span::new(0, 0);
        let bindings = vec![(
            Spanned::new(span, CPattern::Var("x".to_string())),
            Spanned::new(span, CExpr::IntLit(1)),
        )];
        let (_typed, tree) =
            constrain_let_bindings(&mut sub, &mut scope, &bindings, |sub, _, _| Typed {
                span,
                ty: sub.fresh_unbound(),
                node: TExpr::Unit,
            });
        match tree {
            Constraint::Let { top_level, .. } => assert!(!top_level),
            other => panic!("expected a Let, got {other:?}"),
        }
    }

    #[test]
    fn tuple_pattern_let_binding_is_not_treated_as_a_recursive_group() {
        // Exercises constrain_let_bindings's non-Var-pattern path directly:
        // a tuple-pattern let binding can't self-reference, so it should
        // just constrain the RHS and match the pattern against it.
        let mut sub = Substitution::new();
        let mut scope = LocalScope::new();
        scope.push();
        let bindings = vec![(
            Spanned::new(
                knot_syntax::span::Span::new(0, 0),
                CPattern::Tuple(vec![
                    Spanned::new(
                        knot_syntax::span::Span::new(0, 0),
                        CPattern::Var("a".to_string()),
                    ),
                    Spanned::new(
                        knot_syntax::span::Span::new(0, 0),
                        CPattern::Var("b".to_string()),
                    ),
                ]),
            ),
            Spanned::new(
                knot_syntax::span::Span::new(0, 0),
                CExpr::Tuple(vec![
                    Spanned::new(knot_syntax::span::Span::new(0, 0), CExpr::IntLit(1)),
                    Spanned::new(knot_syntax::span::Span::new(0, 0), CExpr::Unit),
                ]),
            ),
        )];
        let (typed, tree) =
            constrain_let_bindings(&mut sub, &mut scope, &bindings, |sub, scope, cs| {
                constrain_expr(
                    sub,
                    scope,
                    &Spanned::new(
                        knot_syntax::span::Span::new(0, 0),
                        CExpr::Var(Ref::Local("a".to_string())),
                    ),
                    cs,
                )
            });
        // No SCC group at all -- just the accumulated Equal constraints.
        assert!(!matches!(tree, Constraint::Let { .. }));
        if let Constraint::And(cs) = &tree {
            for eq in cs {
                if let Constraint::Equal {
                    expected, actual, ..
                } = eq
                {
                    crate::unify::unify(&mut sub, *expected, *actual).unwrap();
                }
            }
        }
        assert_eq!(
            sub.resolve_structure(typed.ty),
            sub.resolve_structure(scope.lookup("a").header_ty())
        );
        assert!(matches!(typed.node, TExpr::Let(ref bs, _) if bs.len() == 1));
    }

    // -- seed_user_constructors --

    #[test]
    fn a_user_defined_constructor_is_usable_as_a_value_and_in_a_pattern() {
        let cs = decls(
            "type Shape = Circle Float\n\
             area s = case s of\n  Circle r -> r\n\
             main = area (Circle 5.0)\n",
        );
        let mut sub = Substitution::new();
        let mut env = crate::solve::SchemeEnv::new();
        seed_user_constructors(&mut sub, &mut env, &cs);
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_polymorphic_constructor_is_generalized_over_every_declared_param() {
        // Result e a's own Err doesn't mention `a` at all -- it must still
        // be quantified over it, or `Err "boom"` couldn't be used at two
        // different `a`s in two different places.
        let cs = decls(
            "type MyResult e a = MyOk a | MyErr e\n\
             useAtInt = case MyErr \"boom\" of\n  MyOk x -> x\n  MyErr _ -> 0\n\
             useAtBool = case MyErr \"boom\" of\n  MyOk x -> x\n  MyErr _ -> True\n",
        );
        let mut sub = Substitution::new();
        let mut env = crate::solve::SchemeEnv::new();
        seed_user_constructors(&mut sub, &mut env, &cs);
        let bool_ty = sub.fresh_bound(Structure::App(Ref::Builtin("Bool".to_string()), vec![]));
        env.insert(
            crate::solve::SchemeKey::Builtin("True".to_string()),
            crate::ty::Scheme::monomorphic(bool_ty),
        );
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_constructors_scheme_has_the_expected_curried_shape() {
        let cs = decls("type Box a = Box a\n");
        let mut sub = Substitution::new();
        let mut env = crate::solve::SchemeEnv::new();
        seed_user_constructors(&mut sub, &mut env, &cs);
        let scheme = env
            .get(&crate::solve::SchemeKey::TopLevel("Box".to_string()))
            .expect("Box should have a scheme")
            .clone();
        assert_eq!(scheme.vars.len(), 1);
        match sub.resolve_structure(scheme.ty) {
            Some(Structure::Fn(field, result)) => {
                assert_eq!(sub.find(field), sub.find(scheme.vars[0]));
                match sub.resolve_structure(result) {
                    Some(Structure::App(r, args)) => {
                        assert_eq!(r, Ref::TopLevel("Box".to_string()));
                        assert_eq!(args.len(), 1);
                        assert_eq!(sub.find(args[0]), sub.find(scheme.vars[0]));
                    }
                    other => panic!("expected App(Box, [a]), got {other:?}"),
                }
            }
            other => panic!("expected a Fn shape, got {other:?}"),
        }
    }

    // -- extensible record aliases applied to a concrete type --
    // (`type alias Selectable a = { a | isSelected : Bool }`, then
    // `Selectable Foo` -- see `knot_canonical::resolve::alias`'s own
    // `substitute_record_ext` for where the merge actually happens.)

    #[test]
    fn an_extensible_record_aliases_own_definition_type_checks() {
        let cs = decls(
            "type alias Foo = { name : String }\n\
             type alias Selectable a = { a | isSelected : Bool }\n\
             makeSelectableFoo :: Selectable Foo\n\
             makeSelectableFoo =\n  { name = \"Bar\", isSelected = True }\n",
        );
        let mut sub = Substitution::new();
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (mut env, _table) = crate::prelude::seed(&mut sub);
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn passing_a_selectable_foo_to_a_plain_foo_signature_is_still_rejected() {
        // `Selectable Foo` has an extra `isSelected` field beyond what the
        // closed `Foo -> String` signature accepts -- matches real Elm's
        // own behavior for this exact program (a closed record alias
        // demands an *exact* field match, not "at least these fields").
        let cs = decls(
            "type alias Foo = { name : String }\n\
             type alias Selectable a = { a | isSelected : Bool }\n\
             getFooName :: Foo -> String\n\
             getFooName foo = foo.name\n\
             makeSelectableFoo :: Selectable Foo\n\
             makeSelectableFoo =\n  { name = \"Bar\", isSelected = True }\n\
             getMyFooName = getFooName makeSelectableFoo\n",
        );
        let mut sub = Substitution::new();
        let (tree, _members) = constrain_module(&mut sub, &cs);
        let (mut env, _table) = crate::prelude::seed(&mut sub);
        let (_pending, errors) = crate::solve::solve(&mut sub, &mut env, &tree);
        assert_eq!(errors.len(), 1, "{errors:?}");
    }
}
