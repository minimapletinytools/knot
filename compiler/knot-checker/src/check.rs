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
/// `Vec` means `decls` type-checks cleanly.
///
/// **One known, narrow gap, inherited rather than introduced here**: a
/// user re-declaring an instance a *builtin* type already has (`instance Eq
/// Int where ...`) isn't flagged as a `DuplicateInstance` the way
/// re-declaring a *user* instance twice is — `build_instance_table`'s own
/// coherence pass only ever sees the module's own declared instances, not
/// the builtin table it gets merged with only afterward. Worth a
/// `corpus/semantic` fixture pinning this down as a known limitation
/// rather than silently leaving it ambiguous, but out of scope for this
/// entry point itself.
pub fn check_module(decls: &[Spanned<CDecl>]) -> Vec<TypeError> {
    let mut sub = crate::var::Substitution::new();
    let (mut env, prelude_table) = crate::prelude::seed(&mut sub);
    seed_user_constructors(&mut sub, &mut env, decls);

    let (mut table, mut errors) = build_instance_table(decls);
    table.merge_from(prelude_table);

    let (tree, _members) = constrain_module(&mut sub, decls);
    let (pending, solve_errors, _obligations, given) =
        solve_with_obligations(&mut sub, &mut env, &tree);
    errors.extend(solve_errors);

    check_pending(&mut sub, &table, &given, pending, &mut errors);

    errors
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
}
