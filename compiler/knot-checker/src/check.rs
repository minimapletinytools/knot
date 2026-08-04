//! `check_module` — the entry point `lib.rs` has documented as missing
//! since this crate's own TM0-TM9 milestones: `constrain_module`, `solve`,
//! `build_instance_table`, and `check_pending` wired together into one
//! call, against a real, fully-seeded prelude, instead of every caller
//! (previously: every single test) re-deriving the wiring by hand.
//!
//! **Why this needed to exist before a semantic corpus could.** Nothing
//! anywhere merged `prelude::seed`'s builtin `InstanceTable` with a
//! module's own `build_instance_table` result — every existing test either
//! used a bare `InstanceTable::new()` plus hand-picked `insert_builtin`
//! calls for exactly what that one test needed, or never called
//! `check_pending` at all. That's an easy mistake to make silently: a
//! scratch test for the utterly mundane `addX a b = a + b` on `Float`s
//! spuriously reported `NoInstance("Num")` purely from forgetting this
//! merge, not from any real bug. A semantic corpus that hand-wires this
//! per-fixture would hit the same trap on every single fixture.
//!
//! **What this deliberately leaves out.** No `elaborate::elaborate_module`
//! call here — see `check_module`'s own doc comment for why: it isn't
//! just extra scope, it would actively double-count `NoInstance` errors for
//! ordinary top-level bindings while *also* silently missing every
//! obligation from inside an instance method's own body, since
//! `elaborate_module` only walks `LetMember`s (see `lib.rs`'s own Fix #5
//! note that instance methods aren't threaded through elaboration at all
//! yet). `check_pending` is the complete, already-correct source of truth
//! for "does every instance obligation in this module actually resolve";
//! elaboration's own richer `ObligationResolution` classification is a
//! separate, narrower concern layered on top for later, once elaboration
//! itself covers the whole module.
//!
//! **No canonicalization here either** — `check_module` takes an already
//! canonicalized `&[Spanned<CDecl>]`, matching every other function in this
//! crate (`constrain_module`, `build_instance_table`, ...). A caller (a
//! corpus runner, or eventually a real driver) runs
//! `knot_canonical::canonicalize_decls`/`canonicalize_module` first and
//! decides how to report a canonicalization failure separately from a type
//! error — the two error types (`CanonError`/`TypeError`) aren't unified
//! into one here, since nothing else in this crate does that either.

use knot_canonical::ast::CDecl;
use knot_syntax::span::Spanned;

use crate::constrain::decl::{constrain_module, seed_user_constructors};
use crate::error::TypeError;
use crate::interface::instance::{build_instance_table, check_pending};
use crate::solve::solve_with_obligations;

/// Type-checks one already-canonicalized module in full: seeds the real
/// prelude (built-in values *and* instances), seeds every user-defined
/// ADT's own constructor schemes, builds the module's own declared instance
/// table (merged with the prelude's builtin one — the prerequisite this
/// module's own doc comment explains), generates and solves constraints
/// over every top-level binding and instance method body, and checks every
/// resulting instance obligation. Returns every error found; an empty
/// `Vec` means `decls` type-checks cleanly. Also runs pattern-match
/// exhaustiveness checking (`exhaustiveness::check_module_exhaustiveness`)
/// over the same `decls` — see `check_module_with_warnings` for how to
/// actually get at what that finds; this entry point discards it, matching
/// every one of its own existing callers not caring (a non-exhaustive
/// `case` is a warning, never a `TypeError`, so it can't show up here).
///
/// **Fixed. A user re-declaring an instance a *builtin* type already has**
/// (`instance Eq Int where ...`) **is now flagged as a `DuplicateInstance`**,
/// the same as re-declaring a *user* instance twice — `build_instance_
/// table` now takes the seeded `prelude_table` as its own `builtins`
/// parameter, so its coherence pass sees both the module's own declared
/// instances and the builtin table `merge_from` would otherwise have
/// merged in silently afterward.
pub fn check_module(decls: &[Spanned<CDecl>]) -> Vec<TypeError> {
    let (errors, _warnings) = check_module_with_warnings(decls);
    errors
}

/// Same as `check_module`, plus every `exhaustiveness::Warning` found along
/// the way — a wholly separate return value, not folded into the
/// `Vec<TypeError>` above (see `exhaustiveness`'s own doc comment on why a
/// non-exhaustive `case` is never a reason to reject an otherwise-valid
/// program). Split out from `check_module` itself (rather than always
/// returning both) purely so `check_module`'s own ~30 existing callers,
/// which have no use for warnings, don't have to change at all — the same
/// reasoning `solve`/`solve_with_obligations` already established.
pub fn check_module_with_warnings(
    decls: &[Spanned<CDecl>],
) -> (Vec<TypeError>, Vec<crate::exhaustiveness::Warning>) {
    let mut sub = crate::var::Substitution::new();
    let (mut env, prelude_table) = crate::prelude::seed(&mut sub);
    seed_user_constructors(&mut sub, &mut env, decls);

    let (mut table, mut errors) = build_instance_table(decls, &prelude_table);
    table.merge_from(prelude_table);

    let (tree, _members) = constrain_module(&mut sub, decls);
    let (pending, solve_errors, _obligations, given) =
        solve_with_obligations(&mut sub, &mut env, &tree);
    errors.extend(solve_errors);

    check_pending(&mut sub, &table, &given, pending, &mut errors);

    let warnings = crate::exhaustiveness::check_module_exhaustiveness(decls);

    (errors, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decls(src: &str) -> Vec<Spanned<CDecl>> {
        let mut state = knot_syntax::ParseState::new(src);
        let raw = state.parse_decls().unwrap();
        assert!(state.is_eof(), "leftover input: {src}");
        knot_canonical::canonicalize_decls(&raw).unwrap_or_else(|errs| panic!("{errs:?}"))
    }

    #[test]
    fn a_plain_builtin_arithmetic_function_type_checks_with_no_hand_wiring() {
        // The exact case that spuriously failed while drafting the
        // corpus-semantic plan, purely from a scratch test forgetting to
        // merge the builtin instance table in by hand.
        let cs = decls("addX :: Float -> Float -> Float\naddX a b = a + b\n");
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn redeclaring_a_builtin_instance_through_the_real_entry_point_is_a_duplicate_error() {
        // Task #40, end to end through check_module itself (not just
        // build_instance_table directly) -- confirms the fix reaches all
        // the way through the real seeded-prelude-table wiring.
        let cs = decls("instance Eq Int where\n  (==) a b = False\n");
        let errors = check_module(&cs);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            crate::error::TypeErrorKind::DuplicateInstance { interface } if interface == "Eq"
        )));
    }

    #[test]
    fn a_user_defined_adt_with_a_declared_instance_type_checks_end_to_end() {
        let cs = decls(
            "type Shape = Circle Float\n\
             instance Eq Shape where\n  (==) a b = True\n\
             compareShapes :: Shape -> Shape -> Bool\n\
             compareShapes a b = a == b\n",
        );
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_record_spread_type_checks_end_to_end_through_the_real_pipeline() {
        // The knotty-drawings.knot motivating case, in miniature: a shared
        // fields alias spread into two otherwise-unrelated shape aliases,
        // plus the still-open constraint version spreading the same thing.
        let cs = decls(
            "type alias GraphicsElement = { id : Int, fill : String }\n\
             type alias IsGraphicsElement a = { a | ..GraphicsElement }\n\
             type alias Circle = { ..GraphicsElement, cx : Float, cy : Float, r : Float }\n\
             type alias Rect = { ..GraphicsElement, x : Float, y : Float }\n\
             describe :: IsGraphicsElement a -> String\n\
             describe shape = shape.fill\n\
             circleArea :: Circle -> Float\n\
             circleArea c = c.r\n\
             useDescribe :: Circle -> String\n\
             useDescribe c = describe c\n",
        );
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_genuinely_missing_instance_is_still_reported() {
        let cs = decls(
            "type Shape = Circle Float\n\
             compareShapes :: Shape -> Shape -> Bool\n\
             compareShapes a b = a == b\n",
        );
        let errors = check_module(&cs);
        assert!(errors
            .iter()
            .any(|e| matches!(&e.kind, crate::error::TypeErrorKind::NoInstance { interface } if interface == "Eq")));
    }

    #[test]
    fn a_user_instance_method_bodys_own_obligations_are_checked_too() {
        // Confirms check_pending (not elaborate_module) is really what's
        // wired in here -- an obligation that only ever appears inside an
        // instance method's own body, never in any top-level LetMember,
        // must still be caught.
        let cs = decls(concat!(
            "type alias Point = { x : Float }\n",
            "instance Semigroup Point where\n",
            "  (<>) a b = { x = a.x + b.x }\n",
        ));
        let errors = check_module(&cs);
        // Semigroup on a Record target is the InstanceTargetNotNominal
        // gap fixed alongside this entry point -- but the method body's
        // own `a.x + b.x` should still type-check against `Num Float`
        // cleanly regardless, proving check_pending reaches instance
        // method bodies at all.
        assert!(!errors
            .iter()
            .any(|e| matches!(&e.kind, crate::error::TypeErrorKind::NoInstance { interface } if interface == "Num")));
    }

    #[test]
    fn a_collection_instances_methods_are_checked_through_the_real_entry_point() {
        let cs = decls(concat!(
            "type Box a = Box a\n",
            "instance Collection Box where\n",
            "  map f b = case b of\n",
            "    Box x -> Box (f x)\n",
            "  foldl f z b = case b of\n",
            "    Box x -> f z x\n",
            "  foldr f z b = case b of\n",
            "    Box x -> f x z\n",
            "  filter p b = b\n",
            "  length b = 1\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_user_signature_can_be_generic_over_any_collection() {
        // The feature this session's own VarApp grammar/canonical/checker
        // work adds (spec §10.6): a user's own signed function, not just
        // the hand-built prelude schemes, can now be genuinely polymorphic
        // over "any Collection" -- one shared signature, called here
        // against both a user-declared custom instance and the builtin
        // List.
        let cs = decls(concat!(
            "type Box a = Box a\n",
            "instance Collection Box where\n",
            "  map f b = case b of\n",
            "    Box x -> Box (f x)\n",
            "  foldl f z b = case b of\n",
            "    Box x -> f z x\n",
            "  foldr f z b = case b of\n",
            "    Box x -> f x z\n",
            "  filter p b = b\n",
            "  length b = 1\n",
            "countIt :: Collection f => f Int -> Int\n",
            "countIt xs = length xs\n",
            "countedBox :: Int\n",
            "countedBox = countIt (Box 5)\n",
            "countedList :: Int\n",
            "countedList = countIt [1, 2, 3]\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_user_signature_generic_over_collection_rejects_a_type_with_no_collection_instance() {
        // Maybe is a Context, not a Collection (prelude.rs's own seeding) --
        // proves the constraint is genuinely enforced, not vacuously
        // accepted just because the signature parses.
        let cs = decls(concat!(
            "countIt :: Collection f => f Int -> Int\n",
            "countIt xs = length xs\n",
            "bad :: Int\n",
            "bad = countIt (Just 5)\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.iter().any(|e| matches!(
            &e.kind,
            crate::error::TypeErrorKind::NoInstance { interface } if interface == "Collection"
        )));
    }

    #[test]
    fn a_user_signature_can_be_generic_over_any_context() {
        // Same feature, the other constructor-shaped interface -- a signed
        // function using bind/pure through a rigid Context-constrained `f`,
        // called against a user-declared custom instance, a builtin Maybe,
        // and a builtin Result.
        let cs = decls(concat!(
            "type Box a = Box a\n",
            "instance Context Box where\n",
            "  pure x = Box x\n",
            "  bind b f = case b of\n",
            "    Box x -> f x\n",
            "passThrough :: Context f => f Int -> f Int\n",
            "passThrough fa = bind fa (\\x -> pure x)\n",
            "throughBox :: Box Int\n",
            "throughBox = passThrough (Box 5)\n",
            "throughMaybe :: Maybe Int\n",
            "throughMaybe = passThrough (Just 5)\n",
            "throughResult :: Result String Int\n",
            "throughResult = passThrough (Ok 5)\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_nested_let_over_a_rigid_ord_constrained_parameter_is_not_ambiguous() {
        // Fix #13: a hand-rolled quicksort's `smaller`/`larger`, each a
        // zero-arg `let`-binding built from a comparison against the
        // enclosing rigid `a`. Used to wrongly fire `AmbiguousConstraint`
        // because the header-vs-inferred-type `Equal` constraint solved
        // *after* the body, so `xs`'s pattern variable still looked like a
        // fresh, unconnected (thus generalizable) variable at the point
        // `smaller`/`larger` themselves were generalized.
        let cs = decls(concat!(
            "quicksort :: Ord a => List a -> List a\n",
            "quicksort xs = case xs of\n",
            "  [] -> []\n",
            "  pivot : rest ->\n",
            "    let\n",
            "        smaller = filter (\\x -> x < pivot) rest\n",
            "        larger = filter (\\x -> x >= pivot) rest\n",
            "    in\n",
            "    quicksort smaller <> (pivot : quicksort larger)\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_parametric_instances_own_requires_resolves_against_a_rigid_given_argument() {
        // Fix #14: `Max a`'s own `Semigroup` instance requires `Ord` on its
        // argument -- when `maximumOf` uses `<>` on `Max a`-typed values
        // under nothing but `Ord a =>`, `check_instance`'s recursive check
        // into that argument used to answer `false` unconditionally for the
        // bare rigid `a` (no `Structure` for a `Rigid` slot to resolve to),
        // regardless of `given`, misreporting `NoInstance("Semigroup")`.
        let cs = decls(concat!(
            "type Max a = Max a\n",
            "instance Ord a => Semigroup (Max a) where\n",
            "  (<>) x y = case (x, y) of\n",
            "    (Max a, Max b) -> if a > b then x else y\n",
            "unwrapMax :: Max a -> a\n",
            "unwrapMax m = case m of\n",
            "  Max a -> a\n",
            "maximumOf :: Ord a => a -> List a -> a\n",
            "maximumOf first rest = unwrapMax (foldl (\\acc x -> acc <> Max x) (Max first) rest)\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_recursive_parametric_instance_resolves_its_own_element_constraint() {
        // Same root cause as Fix #14 above, but self-referential: `Show
        // (Tree a)`'s own body recursively calls `show` on child `Tree a`
        // nodes, which needs the *same* instance's own table entry to
        // already resolve against the still-rigid, `given`-only element
        // type `a` it's defined in terms of.
        let cs = decls(concat!(
            "type Tree a = Leaf | Node (Tree a) a (Tree a)\n",
            "instance Show a => Show (Tree a) where\n",
            "  show t = case t of\n",
            "    Leaf -> \".\"\n",
            "    Node l x r -> \"(\" <> show l <> \" \" <> show x <> \" \" <> show r <> \")\"\n",
            "sample :: Tree Int\n",
            "sample = Node (Node Leaf 1 Leaf) 2 (Node Leaf 3 Leaf)\n",
            "sampleShown :: String\n",
            "sampleShown = show sample\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_custom_ord_instance_on_a_record_alias_overrides_the_structural_fallback() {
        // interfaces/point-ord.knot's own motivating case: Ord *by
        // magnitude*, a real pattern the automatic structural per-field
        // derivation could never produce. Used to be InstanceTargetNotNominal.
        let cs = decls(concat!(
            "type alias Point = { x : Float, y : Float }\n",
            "magnitude :: Point -> Float\n",
            "magnitude p = p.x * p.x + p.y * p.y\n",
            "instance Eq Point where\n",
            "  (==) a b = a.x == b.x && a.y == b.y\n",
            "instance Ord Point where\n",
            "  compare a b = compare (magnitude a) (magnitude b)\n",
            "closerToOrigin :: Point -> Point -> Point\n",
            "closerToOrigin a b = if a < b then a else b\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_custom_num_instance_on_a_record_alias_type_checks_end_to_end() {
        // interfaces/vector2-custom-num.knot's own motivating case: Num has
        // no structural fallback at all, so this is exactly the case a
        // custom record instance matters most for. Used to be
        // InstanceTargetNotNominal *and* NoInstance("Num") on every use.
        let cs = decls(concat!(
            "type alias Vector2 = { x : Float, y : Float }\n",
            "instance Num Vector2 where\n",
            "  (+) a b = { x = a.x + b.x, y = a.y + b.y }\n",
            "  (-) a b = { x = a.x - b.x, y = a.y - b.y }\n",
            "  (*) a b = { x = a.x * b.x, y = a.y * b.y }\n",
            "  negate v = { x = 0.0 - v.x, y = 0.0 - v.y }\n",
            "  abs v = v\n",
            "  signum v = v\n",
            "sampleSum :: Vector2\n",
            "sampleSum = { x = 1.0, y = 2.0 } + { x = 3.0, y = 4.0 }\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn an_open_row_polymorphic_record_still_cant_take_a_custom_instance() {
        // `HasX a = { a | x : Float }` is a genuinely open row -- there's
        // no fixed, exact shape to match a custom instance's own declared
        // target against (a use site could always have more fields via
        // `a`), so this must still be rejected, unlike a fully closed
        // record.
        let cs = decls(concat!(
            "type alias HasX a = { a | x : Float }\n",
            "instance Eq (HasX a) where\n",
            "  (==) p q = p.x == q.x\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.iter().any(
            |e| matches!(&e.kind, crate::error::TypeErrorKind::InstanceTargetNotNominal { interface } if interface == "Eq")
        ), "{errors:?}");
    }

    #[test]
    fn a_declared_float_signature_accepts_an_integer_literal_body() {
        // The exact motivating case: `x :: Float; x = 5` used to be a hard
        // Unify::Mismatch(Float, Int) -- an int literal is now `Num a =>
        // a`, so it unifies with the declared `Float` directly.
        let cs = decls("x :: Float\nx = 5\n");
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn an_unannotated_integer_literal_still_defaults_to_int() {
        // No signature at all -- must still behave exactly as before this
        // fix (ordinary Int arithmetic), not become ambiguous or silently
        // stay polymorphic.
        let cs = decls("x = 5\ny = x + 1\n");
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn an_integer_literal_used_as_a_generic_num_argument_still_defaults() {
        // The `f x y = x == y; result = f 1 2` shape -- the literals' own
        // shared Num-obligated variable never becomes part of *any*
        // binding's own generalized scheme (`result`'s own type is just
        // `Bool`), so it has to default via solve_with_obligations's own
        // final sweep, not generalize's.
        let cs = decls("f :: Eq a => a -> a -> Bool\nf x y = x == y\nresult = f 1 2\n");
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn an_integer_literal_defaults_to_int_even_inside_a_custom_num_instance() {
        // A custom Num instance elsewhere in the module must not change
        // plain-Int-literal defaulting for code that never touches it.
        let cs = decls(concat!(
            "type alias Vector2 = { x : Float, y : Float }\n",
            "instance Num Vector2 where\n",
            "  (+) a b = { x = a.x + b.x, y = a.y + b.y }\n",
            "  (-) a b = { x = a.x - b.x, y = a.y - b.y }\n",
            "  (*) a b = { x = a.x * b.x, y = a.y * b.y }\n",
            "  negate v = { x = 0.0 - v.x, y = 0.0 - v.y }\n",
            "  abs v = v\n",
            "  signum v = v\n",
            "plainSum = 1 + 2\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn an_integer_literal_with_no_num_instance_for_its_pinned_type_is_a_real_error() {
        // `f :: Bool; f = 5` -- Bool has no Num instance, so this must
        // still be a genuine error (NoInstance("Num"), not a silently
        // accepted Unify success now that literals are polymorphic).
        let cs = decls("f :: Bool\nf = 5\n");
        let errors = check_module(&cs);
        assert!(
            errors.iter().any(
                |e| matches!(&e.kind, crate::error::TypeErrorKind::NoInstance { interface } if interface == "Num")
            ),
            "{errors:?}"
        );
    }

    #[test]
    fn map_fromlist_builds_a_map_end_to_end() {
        // collections/build-map.knot's own motivating case: Map.fromList
        // didn't exist at all (UnboundValue).
        let cs = decls(concat!(
            "populations :: Map String Int\n",
            "populations = Map.fromList [(\"Tokyo\", 37400000), (\"Delhi\", 30290000)]\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn map_get_insert_and_empty_type_check_end_to_end() {
        // collections/word-count-attempt.knot's own motivating case:
        // Map.get/Map.insert/Map.empty didn't exist at all.
        let cs = decls(concat!(
            "countWord :: String -> Map String Int -> Map String Int\n",
            "countWord word counts =\n",
            "  case Map.get word counts of\n",
            "    Just n -> Map.insert word (n + 1) counts\n",
            "    Nothing -> Map.insert word 1 counts\n",
            "countAll :: List String -> Map String Int\n",
            "countAll words = foldl (\\counts w -> countWord w counts) Map.empty words\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn map_key_type_still_needs_eq() {
        // A key type with no Eq instance must still be rejected -- Map's
        // own key-comparing functions all declare `Eq k =>`, not silently
        // accept anything. `NoEqType` is a concrete nominal type here (not
        // a bare lowercase type variable), so it resolves to an ordinary
        // `Structure::App`, not a rigid variable -- the obligation is
        // checked against the real instance table, reporting a plain
        // `NoInstance`, not `NoInstanceForRigid`.
        let cs = decls(concat!(
            "type NoEqType = NoEqType Float\n",
            "bad :: NoEqType -> Map NoEqType Int -> Maybe Int\n",
            "bad key m = Map.get key m\n",
        ));
        let errors = check_module(&cs);
        assert!(
            errors.iter().any(
                |e| matches!(&e.kind, crate::error::TypeErrorKind::NoInstance { interface } if interface == "Eq")
            ),
            "{errors:?}"
        );
    }

    #[test]
    fn an_instance_methods_local_let_over_an_integral_rigid_is_not_ambiguous() {
        // Round 4's own corpus/programs finding: `constrain::decl::
        // constrain_method_body_against` had the identical header-vs-body
        // solve-order bug Fix #13 fixed for ordinary function bindings
        // (`constrain_group_chain`), just in instance methods' own,
        // separate code path -- a `let` inside a `Show` instance's own
        // method body, computing intermediate values via `div`/`mod`
        // before formatting them, used to misfire `AmbiguousConstraint`.
        let cs = decls(concat!(
            "type alias Money = { cents : Int }\n",
            "instance Show Money where\n",
            "  show m =\n",
            "    let\n",
            "        dollars = div m.cents 100\n",
            "        remainder = mod m.cents 100\n",
            "    in\n",
            "    \"$\" <> show dollars <> \".\" <> show remainder\n",
        ));
        let errors = check_module(&cs);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn check_module_with_warnings_reports_a_non_exhaustive_case_but_no_type_error() {
        // known_gaps/non-exhaustive-case-not-flagged.knot's own motivating
        // case: a case missing an entire constructor arm used to produce
        // zero diagnostics of any kind. Now a Warning -- but still not a
        // TypeError, so a caller that only looks at check_module's own
        // Vec<TypeError> (all ~30 of check_module's own existing callers)
        // correctly keeps seeing this as a clean type-check.
        let cs = decls(concat!(
            "type Shape = Circle Float | Square Float | Triangle Float Float Float\n",
            "area :: Shape -> Float\n",
            "area shape = case shape of\n",
            "  Circle r -> 3.14159 * r * r\n",
            "  Square s -> s * s\n",
        ));
        let (errors, warnings) = check_module_with_warnings(&cs);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(
            warnings[0].kind,
            crate::exhaustiveness::WarningKind::NonExhaustiveMatch
        );
    }
}
