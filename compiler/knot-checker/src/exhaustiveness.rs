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

use std::collections::{HashMap, HashSet};

use knot_canonical::ast::{CDecl, CPattern, Ref};
use knot_syntax::ast::pattern::PatternLiteral;
use knot_syntax::span::Spanned;

/// Every constructor's sibling set (including itself), keyed by any one of
/// them — `Ref::TopLevel("Circle")` and `Ref::TopLevel("Rectangle")` both
/// map to the same `[(Circle, 1), (Rectangle, 2)]` list for a `type Shape =
/// Circle Float | Rectangle Float Float`. Built-in enum-shaped types
/// (`Bool`, `Option`, `Result`, `Ordering`) are seeded up front; `List`
/// isn't — its patterns are `CPattern::Cons`/`CPattern::Nil`, a distinct
/// pattern shape entirely, not `CPattern::Ctor`, so it's handled directly
/// in `head_kind`/`complete_signature` instead of through this table.
pub struct CtorTable {
    by_ctor: HashMap<Ref, Vec<(Ref, usize)>>,
}

impl CtorTable {
    pub fn new() -> Self {
        let mut table = CtorTable {
            by_ctor: HashMap::new(),
        };
        table.add_group(&[("True", 0), ("False", 0)]);
        table.add_group(&[("Some", 1), ("None", 0)]);
        table.add_group(&[("Ok", 1), ("Err", 1)]);
        table.add_group(&[("LT", 0), ("EQ", 0), ("GT", 0)]);
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
            if let CDecl::TypeDecl(_name, _params, variants) = &d.node {
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
        let missing_some_false = arm_patterns("f x = case x of\n  None -> 0\n  Some True -> 1\n");
        assert!(!is_exhaustive(&ctors, &missing_some_false));

        let complete =
            arm_patterns("f x = case x of\n  None -> 0\n  Some True -> 1\n  Some False -> 2\n");
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
}
