//! Pattern-match exhaustiveness/redundancy checking (plan §7, TM9 —
//! explicitly a stretch goal, lower priority: spec only ever wants a
//! *warning*, never a hard error, and this is a self-contained analysis
//! that doesn't block anything else in the pipeline). Maranget's
//! usefulness-checking algorithm (the same one Elm's `Nitpick.
//! PatternMatches` implements) — "is there a value this pattern matches
//! that no earlier row already covers."
//!
//! **Scope note**: this reports *whether* a `case` is exhaustive and *which*
//! arms are redundant, not a constructed counter-example ("missing:
//! `Circle _`") the way GHC/Elm's own diagnostics do — witness synthesis is
//! a further, well-defined extension of the same algorithm, deliberately
//! left out to keep this stretch milestone bounded. A boolean "does an
//! uncovered value exist" answer is already the useful, load-bearing part.
//!
//! **Now wired into `check::check_module`** (previously true, no longer:
//! this used to be entirely self-contained, with nothing else in the
//! pipeline calling into it at all). `check_module_exhaustiveness` is the
//! whole-module driver `is_exhaustive`/`redundant_arms` themselves don't
//! provide — it walks every `CDecl::Fn`/instance-method body's `CExpr`
//! tree, finds every `CExpr::Case`, and checks each one. Returns `Vec
//! <Warning>`, a wholly separate channel from `TypeError`: a non-exhaustive
//! `case` is never a reason to reject an otherwise-valid program (the spec
//! only ever wants a warning here, never a hard error — the very first
//! line of this doc comment), so this can never turn a program
//! `check_module` used to accept into one it now rejects.

use std::collections::{HashMap, HashSet};

use knot_canonical::ast::{CDecl, CDoStmt, CExpr, CPattern, Ref};
use knot_syntax::ast::pattern::PatternLiteral;
use knot_syntax::span::{Span, Spanned};

/// Every constructor's sibling set (including itself), keyed by any one of
/// them — `Ref::TopLevel("Circle")` and `Ref::TopLevel("Rectangle")` both
/// map to the same `[(Circle, 1), (Rectangle, 2)]` list for a `type Shape =
/// Circle Float | Rectangle Float Float`. Built-in enum-shaped types
/// (`Bool`, `Maybe`, `Result`, `Ordering`) are seeded up front; `List`
/// isn't — its patterns are `CPattern::Cons`/`CPattern::Nil`, a distinct
/// pattern shape entirely, not `CPattern::Ctor`, so it's handled directly
/// in `head_kind`/`complete_signature` instead of through this table.
pub struct CtorTable {
    by_ctor: HashMap<Ref, Vec<(Ref, usize)>>,
}

impl CtorTable {
    /// Derives every built-in sibling group straight from `knot_canonical::
    /// prelude::BUILTIN_CONSTRUCTORS` — that table already has exactly what
    /// a group needs (name, arity, owning type), grouped here by the third
    /// field instead of hardcoded a second time. This used to be its own
    /// hand-maintained literal list; found (and fixed) as a real, easy-to-
    /// miss duplicate while renaming `Option`/`Some`/`None` to `Maybe`/
    /// `Just`/`Nothing` — this file's own list had silently kept the old
    /// names for one extra edit before that was caught.
    pub fn new() -> Self {
        let mut table = CtorTable {
            by_ctor: HashMap::new(),
        };
        let mut groups: Vec<(&str, Vec<(&str, usize)>)> = Vec::new();
        for &(name, arity, owner) in knot_canonical::prelude::BUILTIN_CONSTRUCTORS {
            match groups.iter_mut().find(|(o, _)| *o == owner) {
                Some((_, members)) => members.push((name, arity)),
                None => groups.push((owner, vec![(name, arity)])),
            }
        }
        for (_, members) in &groups {
            table.add_group(members);
        }
        table
    }

    fn add_group(&mut self, ctors: &[(&str, usize)]) {
        let group: Vec<(Ref, usize)> = ctors
            .iter()
            .map(|(name, arity)| (Ref::Builtin(name.to_string()), *arity))
            .collect();
        for (r, _) in &group {
            self.by_ctor.insert(r.clone(), group.clone());
        }
    }

    /// Built-ins plus every user `type` declaration's own variants.
    pub fn from_decls(decls: &[Spanned<CDecl>]) -> Self {
        let mut table = CtorTable::new();
        for d in decls {
            if let CDecl::TypeDecl(_name, _params, variants, _deriving) = &d.node {
                let group: Vec<(Ref, usize)> = variants
                    .iter()
                    .map(|(ctor_name, args)| (Ref::TopLevel(ctor_name.clone()), args.len()))
                    .collect();
                for (r, _) in &group {
                    table.by_ctor.insert(r.clone(), group.clone());
                }
            }
        }
        table
    }

    fn siblings_of(&self, r: &Ref) -> Option<&[(Ref, usize)]> {
        self.by_ctor.get(r).map(|v| v.as_slice())
    }
}

impl Default for CtorTable {
    fn default() -> Self {
        CtorTable::new()
    }
}

/// What a pattern's own "shape" is, for specialization purposes —
/// `As`/`Var`/`Wildcard` all collapse into `Wildcard` (an alias's inner
/// pattern is unwrapped before this is ever computed; see `head_kind`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Head {
    Wildcard,
    Ctor(Ref),
    IntLit(i64),
    StrLit(String),
    Tuple,
    Cons,
    Nil,
    Unit,
}

fn unwrap_as(p: &CPattern) -> &CPattern {
    match p {
        CPattern::As(inner, _) => unwrap_as(&inner.node),
        other => other,
    }
}

fn head_kind(p: &CPattern) -> Head {
    match unwrap_as(p) {
        CPattern::Wildcard(_) | CPattern::Var(_) => Head::Wildcard,
        CPattern::Ctor(r, _) => Head::Ctor(r.clone()),
        CPattern::Literal(PatternLiteral::Int(n)) => Head::IntLit(*n),
        CPattern::Literal(PatternLiteral::Str(s)) => Head::StrLit(s.clone()),
        CPattern::Tuple(_) => Head::Tuple,
        CPattern::Cons(..) => Head::Cons,
        CPattern::Nil => Head::Nil,
        CPattern::Unit => Head::Unit,
        CPattern::As(..) => unreachable!("unwrap_as already peeled every As"),
    }
}

fn arity_of(p: &CPattern) -> usize {
    match unwrap_as(p) {
        CPattern::Ctor(_, subs) => subs.len(),
        CPattern::Tuple(subs) => subs.len(),
        CPattern::Cons(..) => 2,
        _ => 0,
    }
}

fn sub_patterns(p: &CPattern) -> Vec<CPattern> {
    match unwrap_as(p) {
        CPattern::Ctor(_, subs) => subs.iter().map(|s| s.node.clone()).collect(),
        CPattern::Tuple(subs) => subs.iter().map(|s| s.node.clone()).collect(),
        CPattern::Cons(head, tail) => vec![head.node.clone(), tail.node.clone()],
        _ => Vec::new(),
    }
}

type Row = Vec<CPattern>;

fn wildcards(n: usize) -> Vec<CPattern> {
    (0..n).map(|_| CPattern::Wildcard(None)).collect()
}

/// Keeps (and expands) rows compatible with matching `head`, drops the
/// rest — the core step of Maranget's algorithm: a wildcard row expands to
/// `arity` fresh wildcards (it matches *any* shape); a row already headed
/// by `head` contributes its own sub-patterns; anything else can't produce
/// a value shaped like `head` at all and is dropped.
fn specialize(matrix: &[Row], head: &Head, arity: usize) -> Vec<Row> {
    matrix
        .iter()
        .filter_map(|row| {
            let (first, rest) = row.split_first()?;
            match head_kind(first) {
                Head::Wildcard => {
                    let mut new_row = wildcards(arity);
                    new_row.extend_from_slice(rest);
                    Some(new_row)
                }
                h if h == *head => {
                    let mut new_row = sub_patterns(first);
                    new_row.extend_from_slice(rest);
                    Some(new_row)
                }
                _ => None,
            }
        })
        .collect()
}

/// Rows whose first column is a wildcard (any constructor-headed row can't
/// contribute to "what if none of the seen constructors apply"), that
/// column dropped — used when the constructors appearing in the matrix's
/// first column *don't* cover the whole type (or the type has no enumerable
/// complete set at all, like `Int`/`String`).
fn default_matrix(matrix: &[Row]) -> Vec<Row> {
    matrix
        .iter()
        .filter_map(|row| {
            let (first, rest) = row.split_first()?;
            matches!(head_kind(first), Head::Wildcard).then(|| rest.to_vec())
        })
        .collect()
}

/// `Some(complete_ctors)` if the constructors appearing in `first_column`
/// exhaust their whole type (so usefulness can be decided by checking each
/// one directly); `None` if incomplete, or if the type has no enumerable
/// complete set at all (`Int`/`String`, or no constructor seen yet). Takes
/// the actual patterns, not just their abstracted `Head`s — `Head` erases a
/// `Tuple`'s arity, which has to come from a real sample pattern instead.
fn complete_signature(ctors: &CtorTable, first_column: &[&CPattern]) -> Option<Vec<(Head, usize)>> {
    let heads: Vec<Head> = first_column.iter().map(|p| head_kind(p)).collect();
    let sample_idx = heads.iter().position(|h| *h != Head::Wildcard)?;
    match &heads[sample_idx] {
        Head::IntLit(_) | Head::StrLit(_) => None, // unbounded domain, never complete
        Head::Unit => Some(vec![(Head::Unit, 0)]),
        Head::Tuple => {
            let arity = arity_of(unwrap_as(first_column[sample_idx]));
            Some(vec![(Head::Tuple, arity)])
        }
        Head::Cons | Head::Nil => {
            let seen: HashSet<&Head> = heads.iter().collect();
            (seen.contains(&Head::Cons) && seen.contains(&Head::Nil))
                .then(|| vec![(Head::Cons, 2), (Head::Nil, 0)])
        }
        Head::Ctor(r) => {
            let siblings = ctors.siblings_of(r)?;
            let seen: HashSet<&Ref> = heads
                .iter()
                .filter_map(|h| match h {
                    Head::Ctor(r) => Some(r),
                    _ => None,
                })
                .collect();
            let complete = siblings.iter().all(|(sib, _)| seen.contains(sib));
            complete.then(|| {
                siblings
                    .iter()
                    .map(|(r, arity)| (Head::Ctor(r.clone()), *arity))
                    .collect()
            })
        }
        Head::Wildcard => unreachable!("position() above already skips Wildcard"),
    }
}

/// Is `query` useful against `matrix` — does some value match `query` that
/// no row of `matrix` already matches? Both must have the same row width.
fn is_useful(ctors: &CtorTable, matrix: &[Row], query: &[CPattern]) -> bool {
    let Some((first, rest)) = query.split_first() else {
        return matrix.is_empty();
    };
    match head_kind(first) {
        Head::Wildcard => {
            let first_column: Vec<&CPattern> =
                matrix.iter().filter_map(|row| row.first()).collect();
            match complete_signature(ctors, &first_column) {
                Some(complete) => complete.into_iter().any(|(head, arity)| {
                    let spec_matrix = specialize(matrix, &head, arity);
                    let mut spec_query = wildcards(arity);
                    spec_query.extend_from_slice(rest);
                    is_useful(ctors, &spec_matrix, &spec_query)
                }),
                None => is_useful(ctors, &default_matrix(matrix), rest),
            }
        }
        head => {
            let arity = arity_of(first);
            let spec_matrix = specialize(matrix, &head, arity);
            let mut spec_query = sub_patterns(first);
            spec_query.extend_from_slice(rest);
            is_useful(ctors, &spec_matrix, &spec_query)
        }
    }
}

/// `true` if every value of the scrutinee's type is matched by some arm.
pub fn is_exhaustive(ctors: &CtorTable, arm_patterns: &[CPattern]) -> bool {
    let matrix: Vec<Row> = arm_patterns.iter().map(|p| vec![p.clone()]).collect();
    !is_useful(ctors, &matrix, &[CPattern::Wildcard(None)])
}

/// Indices of arms that can never be reached — each one's own pattern
/// matches nothing that every *earlier* arm hasn't already matched.
pub fn redundant_arms(ctors: &CtorTable, arm_patterns: &[CPattern]) -> Vec<usize> {
    let mut redundant = Vec::new();
    for i in 0..arm_patterns.len() {
        let matrix: Vec<Row> = arm_patterns[..i].iter().map(|p| vec![p.clone()]).collect();
        let query = vec![arm_patterns[i].clone()];
        if !is_useful(ctors, &matrix, &query) {
            redundant.push(i);
        }
    }
    redundant
}

/// One diagnostic this pass produces — never a reason to reject a
/// program, so a wholly separate type from `crate::error::TypeError`
/// rather than another `TypeErrorKind` variant.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub span: Span,
    pub kind: WarningKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WarningKind {
    /// A `case`'s own arms don't cover every value of the scrutinee's type.
    NonExhaustiveMatch,
    /// This arm can never be reached — every earlier arm already matches
    /// everything it would.
    RedundantMatchArm,
}

/// The whole-module driver: walks every `CDecl::Fn`/`CDecl::Instance`
/// method body in `decls`, checking every `CExpr::Case` it finds. See this
/// module's own doc comment for why the result is `Vec<Warning>`, not
/// something folded into `check::check_module`'s own `Vec<TypeError>`.
pub fn check_module_exhaustiveness(decls: &[Spanned<CDecl>]) -> Vec<Warning> {
    let ctors = CtorTable::from_decls(decls);
    let mut warnings = Vec::new();
    for d in decls {
        match &d.node {
            CDecl::Fn(fndef) => walk_expr(&ctors, &fndef.body, &mut warnings),
            CDecl::Instance(inst) => {
                for method in &inst.methods {
                    walk_expr(&ctors, &method.body, &mut warnings);
                }
            }
            CDecl::TypeAlias(..) | CDecl::TypeDecl(..) => {}
        }
    }
    warnings
}

fn check_case(
    ctors: &CtorTable,
    arms: &[(Spanned<CPattern>, Spanned<CExpr>)],
    span: Span,
    warnings: &mut Vec<Warning>,
) {
    let arm_patterns: Vec<CPattern> = arms.iter().map(|(p, _)| p.node.clone()).collect();
    if !is_exhaustive(ctors, &arm_patterns) {
        warnings.push(Warning {
            span,
            kind: WarningKind::NonExhaustiveMatch,
        });
    }
    for i in redundant_arms(ctors, &arm_patterns) {
        warnings.push(Warning {
            span: arms[i].0.span,
            kind: WarningKind::RedundantMatchArm,
        });
    }
}

/// Every `CExpr` reachable from `expr`, looking for `Case` nodes to check.
/// `TPattern`/other binding forms (`Let`'s own destructuring, a `Lambda`'s
/// params, a `do`-bind) are never checked here — only `case` is meant to
/// be checked for exhaustiveness, matching this module's own existing
/// framing throughout (every doc comment above only ever talks about
/// `case` arms).
fn walk_expr(ctors: &CtorTable, expr: &Spanned<CExpr>, warnings: &mut Vec<Warning>) {
    match &expr.node {
        CExpr::IntLit(_)
        | CExpr::FloatLit(_)
        | CExpr::StringLit(_)
        | CExpr::Unit
        | CExpr::Var(_)
        | CExpr::Ctor(_)
        | CExpr::Hole => {}
        CExpr::Lambda(_, body) => walk_expr(ctors, body, warnings),
        CExpr::App(f, arg) => {
            walk_expr(ctors, f, warnings);
            walk_expr(ctors, arg, warnings);
        }
        CExpr::BinOp(_, l, r) => {
            walk_expr(ctors, l, warnings);
            walk_expr(ctors, r, warnings);
        }
        CExpr::Negate(inner) => walk_expr(ctors, inner, warnings),
        CExpr::If(c, t, e) => {
            walk_expr(ctors, c, warnings);
            walk_expr(ctors, t, warnings);
            walk_expr(ctors, e, warnings);
        }
        CExpr::Let(bindings, body) => {
            for (_, rhs) in bindings {
                walk_expr(ctors, rhs, warnings);
            }
            walk_expr(ctors, body, warnings);
        }
        CExpr::Case(scrutinee, arms) => {
            walk_expr(ctors, scrutinee, warnings);
            check_case(ctors, arms, expr.span, warnings);
            for (_, body) in arms {
                walk_expr(ctors, body, warnings);
            }
        }
        CExpr::Do(stmts, body) => {
            for stmt in stmts {
                match stmt {
                    CDoStmt::Bind(_, e) => walk_expr(ctors, e, warnings),
                    CDoStmt::Expr(e) => walk_expr(ctors, e, warnings),
                }
            }
            walk_expr(ctors, body, warnings);
        }
        CExpr::List(elems) | CExpr::Tuple(elems) => {
            for e in elems {
                walk_expr(ctors, e, warnings);
            }
        }
        CExpr::Record(fields) => {
            for (_, e) in fields {
                walk_expr(ctors, e, warnings);
            }
        }
        CExpr::RecordUpdate(base, updates) => {
            walk_expr(ctors, base, warnings);
            for (_, e) in updates {
                walk_expr(ctors, e, warnings);
            }
        }
        CExpr::FieldAccess(base, _) => walk_expr(ctors, base, warnings),
        CExpr::Annotated(_, target) => walk_expr(ctors, target, warnings),
    }
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

    /// Pulls the arm patterns out of `src`'s one `f x = case x of ...`
    /// binding's `CExpr::Case` -- `src` may have a preceding `type`
    /// declaration too (needed for any constructor the case arms use to
    /// actually resolve), so this scans for the `Fn` decl rather than
    /// assuming it's first.
    fn arm_patterns(src: &str) -> Vec<CPattern> {
        let cs = decls(src);
        let fndef = cs
            .iter()
            .find_map(|d| match &d.node {
                knot_canonical::ast::CDecl::Fn(f) => Some(f),
                _ => None,
            })
            .expect("expected a Fn decl");
        let knot_canonical::ast::CExpr::Case(_, arms) = &fndef.body.node else {
            panic!("expected the body to be a Case")
        };
        arms.iter().map(|(p, _)| p.node.clone()).collect()
    }

    #[test]
    fn wildcard_alone_is_exhaustive() {
        let ctors = CtorTable::new();
        let arms = arm_patterns("f x = case x of\n  _ -> 0\n");
        assert!(is_exhaustive(&ctors, &arms));
        assert!(redundant_arms(&ctors, &arms).is_empty());
    }

    #[test]
    fn bool_needs_both_true_and_false() {
        let ctors = CtorTable::new();
        let only_true = arm_patterns("f x = case x of\n  True -> 0\n");
        assert!(!is_exhaustive(&ctors, &only_true));

        let both = arm_patterns("f x = case x of\n  True -> 0\n  False -> 1\n");
        assert!(is_exhaustive(&ctors, &both));
        assert!(redundant_arms(&ctors, &both).is_empty());
    }

    #[test]
    fn wildcard_after_both_bool_ctors_is_redundant() {
        let ctors = CtorTable::new();
        let arms = arm_patterns("f x = case x of\n  True -> 0\n  False -> 1\n  _ -> 2\n");
        assert_eq!(redundant_arms(&ctors, &arms), vec![2]);
    }

    #[test]
    fn an_earlier_wildcard_makes_every_later_arm_redundant() {
        let ctors = CtorTable::new();
        let arms = arm_patterns("f x = case x of\n  _ -> 0\n  True -> 1\n  False -> 2\n");
        assert_eq!(redundant_arms(&ctors, &arms), vec![1, 2]);
    }

    #[test]
    fn user_defined_adt_exhaustiveness() {
        let shape_decl =
            "type Shape = Circle Float | Rectangle Float Float | Triangle Float Float Float\n";

        let missing_triangle = arm_patterns(&format!(
            "{shape_decl}f x = case x of\n  Circle r -> r\n  Rectangle w h -> w\n"
        ));
        let ctors = CtorTable::from_decls(&decls(shape_decl));
        assert!(!is_exhaustive(&ctors, &missing_triangle));

        let complete = arm_patterns(&format!(
            "{shape_decl}f x = case x of\n  Circle r -> r\n  Rectangle w h -> w\n  Triangle a b c -> a\n"
        ));
        assert!(is_exhaustive(&ctors, &complete));
    }

    #[test]
    fn nested_constructor_patterns_are_handled() {
        let ctors = CtorTable::new();
        let missing_just_false =
            arm_patterns("f x = case x of\n  Nothing -> 0\n  Just True -> 1\n");
        assert!(!is_exhaustive(&ctors, &missing_just_false));

        let complete =
            arm_patterns("f x = case x of\n  Nothing -> 0\n  Just True -> 1\n  Just False -> 2\n");
        assert!(is_exhaustive(&ctors, &complete));
    }

    #[test]
    fn list_cons_and_nil_must_both_be_covered() {
        let ctors = CtorTable::new();
        let missing_nil = arm_patterns("f x = case x of\n  h : t -> h\n");
        assert!(!is_exhaustive(&ctors, &missing_nil));

        let complete = arm_patterns("f x = case x of\n  h : t -> h\n  [] -> 0\n");
        assert!(is_exhaustive(&ctors, &complete));
    }

    #[test]
    fn int_and_string_literals_are_never_complete_without_a_wildcard() {
        let ctors = CtorTable::new();
        let just_literals = arm_patterns("f x = case x of\n  1 -> 0\n  2 -> 1\n");
        assert!(!is_exhaustive(&ctors, &just_literals));

        let with_wildcard = arm_patterns("f x = case x of\n  1 -> 0\n  2 -> 1\n  _ -> 2\n");
        assert!(is_exhaustive(&ctors, &with_wildcard));
    }

    #[test]
    fn tuple_pattern_is_exhaustive_via_its_own_element_wildcards() {
        let ctors = CtorTable::new();
        let arms = arm_patterns("f x = case x of\n  (a, b) -> a\n");
        assert!(is_exhaustive(&ctors, &arms));
    }

    #[test]
    fn as_pattern_defers_to_its_inner_pattern() {
        let ctors = CtorTable::new();
        let arms = arm_patterns("f x = case x of\n  True as t -> 0\n  False -> 1\n");
        assert!(is_exhaustive(&ctors, &arms));
    }

    // -- check_module_exhaustiveness: the whole-module driver --

    #[test]
    fn a_non_exhaustive_case_missing_a_whole_arm_is_a_warning() {
        let cs = decls(concat!(
            "type Shape = Circle Float | Square Float | Triangle Float Float Float\n",
            "area :: Shape -> Float\n",
            "area shape = case shape of\n",
            "  Circle r -> 3.14159 * r * r\n",
            "  Square s -> s * s\n",
        ));
        let warnings = check_module_exhaustiveness(&cs);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, WarningKind::NonExhaustiveMatch);
    }

    #[test]
    fn an_exhaustive_case_has_no_warnings() {
        let cs = decls(concat!(
            "type Shape = Circle Float | Square Float\n",
            "area :: Shape -> Float\n",
            "area shape = case shape of\n",
            "  Circle r -> 3.14159 * r * r\n",
            "  Square s -> s * s\n",
        ));
        assert!(check_module_exhaustiveness(&cs).is_empty());
    }

    #[test]
    fn a_redundant_arm_is_its_own_warning_kind() {
        let cs = decls(concat!(
            "type Shape = Circle Float | Square Float\n",
            "area :: Shape -> Float\n",
            "area shape = case shape of\n",
            "  _ -> 0.0\n",
            "  Circle r -> 3.14159 * r * r\n",
        ));
        let warnings = check_module_exhaustiveness(&cs);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, WarningKind::RedundantMatchArm);
    }

    #[test]
    fn a_nested_case_inside_a_case_arm_is_still_checked() {
        let cs = decls(concat!(
            "type Shape = Circle Float | Square Float\n",
            "type Color = Red | Blue\n",
            "describe :: Shape -> Color -> String\n",
            "describe shape color = case shape of\n",
            "  Circle r -> case color of\n",
            "    Red -> \"red circle\"\n",
            "  Square s -> \"square\"\n",
        ));
        let warnings = check_module_exhaustiveness(&cs);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, WarningKind::NonExhaustiveMatch);
    }

    #[test]
    fn an_instance_methods_own_case_is_checked_too() {
        let cs = decls(concat!(
            "type Shape = Circle Float | Square Float\n",
            "instance Show Shape where\n",
            "  show shape = case shape of\n",
            "    Circle r -> \"circle\"\n",
        ));
        let warnings = check_module_exhaustiveness(&cs);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, WarningKind::NonExhaustiveMatch);
    }
}
