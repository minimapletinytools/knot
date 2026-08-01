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
//! **Not yet handled**: `CDecl::Instance` — its methods are real function
//! bodies needing type-checking too, but checking one against its
//! interface's own method signature needs `interface/table.rs` (TM6),
//! which doesn't exist yet. `CDecl::TypeAlias`/`CDecl::TypeDecl` need no
//! constraints at all (purely type-level; their own name references were
//! already validated during canonicalization).

use std::collections::{HashMap, HashSet};

use knot_canonical::ast::{CDecl, CExpr, CPattern, CType, CTypeSignature, Ref};
use knot_syntax::span::{Span, Spanned};

use crate::constrain::expr::constrain_expr;
use crate::constrain::pattern::constrain_pattern;
use crate::constrain::{Constraint, DeclaredScheme, LetMember, LocalScope};
use crate::ty::Structure;
use crate::var::{Substitution, TypeVarId};

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
        CType::Record(fields, ext) => {
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
fn constrain_bindings(
    sub: &mut Substitution,
    scope: &mut LocalScope,
    bindings: Vec<RawBinding>,
    top_level: bool,
    finish: impl FnOnce(&mut Substitution, &mut LocalScope) -> (TypeVarId, Constraint),
) -> (TypeVarId, Constraint) {
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
fn constrain_group_chain(
    sub: &mut Substitution,
    scope: &mut LocalScope,
    by_name: &HashMap<String, &RawBinding>,
    sccs: &[Vec<String>],
    index: usize,
    top_level: bool,
    finish: impl FnOnce(&mut Substitution, &mut LocalScope) -> (TypeVarId, Constraint),
) -> (TypeVarId, Constraint) {
    let Some(group_names) = sccs.get(index) else {
        return finish(sub, scope);
    };
    let group: Vec<&RawBinding> = group_names.iter().map(|n| by_name[n]).collect();

    scope.push();
    let mut members = Vec::new();
    for b in &group {
        let (header_ty, declared) = match b.signature {
            Some(sig) => {
                let (ty, declared) = build_declared(sub, &sig.node);
                (ty, Some(declared))
            }
            None => (sub.fresh_unbound(), None),
        };
        scope.bind(&b.name, header_ty);
        members.push(LetMember {
            name: b.name.clone(),
            span: b.span,
            header_ty,
            declared,
        });
    }

    let mut header_constraints = Vec::new();
    for (b, member) in group.iter().zip(&members) {
        scope.push();
        let param_tys: Vec<TypeVarId> = b
            .params
            .iter()
            .map(|p| constrain_pattern(sub, scope, p, &mut header_constraints))
            .collect();
        let body_ty = constrain_expr(sub, scope, b.body, &mut header_constraints);
        scope.pop();
        let inferred = param_tys.into_iter().rev().fold(body_ty, |acc, param_ty| {
            sub.fresh_bound(Structure::Fn(param_ty, acc))
        });
        header_constraints.push(Constraint::Equal {
            span: b.span,
            expected: member.header_ty,
            actual: inferred,
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
    let (result_ty, body_con) =
        constrain_group_chain(sub, scope, by_name, sccs, index + 1, top_level, finish);
    if !top_level {
        scope.pop();
    }

    let whole = Constraint::Let {
        top_level,
        members,
        header_con: Box::new(Constraint::And(header_constraints)),
        body_con: Box::new(body_con),
    };
    (result_ty, whole)
}

/// The whole-module entry point: every `CDecl::Fn` becomes a `RawBinding`;
/// SCC-split and wrapped in nested `Constraint::Let`s. There's no module
/// "body" the way a `let...in` has one — solving a module is entirely about
/// populating the scheme environment these `Let`s build, so the innermost
/// constraint is trivially `Constraint::True`.
pub fn constrain_module(sub: &mut Substitution, decls: &[Spanned<CDecl>]) -> Constraint {
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
    let (_unused, tree) = constrain_bindings(sub, &mut scope, bindings, true, |sub, _| {
        (sub.fresh_unbound(), Constraint::True)
    });
    tree
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
pub fn constrain_let_bindings(
    sub: &mut Substitution,
    scope: &mut LocalScope,
    bindings: &[(Spanned<CPattern>, Spanned<CExpr>)],
    constrain_body: impl FnOnce(&mut Substitution, &mut LocalScope, &mut Vec<Constraint>) -> TypeVarId,
) -> (TypeVarId, Constraint) {
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
    constrain_bindings(sub, scope, raw, false, |sub, scope| {
        let mut constraints = Vec::new();
        for (p, e) in &other_bindings {
            let rhs_ty = constrain_expr(sub, scope, e, &mut constraints);
            let pat_ty = constrain_pattern(sub, scope, p, &mut constraints);
            constraints.push(Constraint::Equal {
                span: p.span,
                expected: pat_ty,
                actual: rhs_ty,
            });
        }
        let result_ty = constrain_body(sub, scope, &mut constraints);
        (result_ty, Constraint::And(constraints))
    })
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
        let tree = constrain_module(&mut sub, &cs);
        let (members, _header, body) = as_let(&tree);
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].name, "id");
        assert!(members[0].declared.is_none());
        assert_eq!(*body, Constraint::True);
    }

    #[test]
    fn mutually_recursive_bindings_share_one_scc_group() {
        let mut sub = Substitution::new();
        let cs = decls(
            "isOdd n = if n == 0 then False else isEven (n - 1)\n\
             isEven n = if n == 0 then True else isOdd (n - 1)\n",
        );
        let tree = constrain_module(&mut sub, &cs);
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
        let tree = constrain_module(&mut sub, &cs);
        let (outer_members, _outer_header, inner) = as_let(&tree);
        assert_eq!(outer_members.len(), 1);
        assert_eq!(outer_members[0].name, "helper");
        let (inner_members, _inner_header, innermost) = as_let(inner);
        assert_eq!(inner_members.len(), 1);
        assert_eq!(inner_members[0].name, "useHelper");
        assert_eq!(*innermost, Constraint::True);
    }

    #[test]
    fn signed_binding_gets_rigid_vars_and_given_constraints() {
        let mut sub = Substitution::new();
        let cs = decls("myMax :: Ord a => a -> a -> a\nmyMax x y = x\n");
        let tree = constrain_module(&mut sub, &cs);
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
        let tree = constrain_module(&mut sub, &cs);
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
        let (_ty, tree) = constrain_let_bindings(&mut sub, &mut scope, &bindings, |sub, _, _| {
            sub.fresh_unbound()
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
        let (result_ty, tree) =
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
        for c in std::iter::once(&tree) {
            if let Constraint::And(cs) = c {
                for eq in cs {
                    if let Constraint::Equal {
                        expected, actual, ..
                    } = eq
                    {
                        crate::unify::unify(&mut sub, *expected, *actual).unwrap();
                    }
                }
            }
        }
        assert_eq!(
            sub.resolve_structure(result_ty),
            sub.resolve_structure(scope.lookup("a").header_ty())
        );
    }
}
