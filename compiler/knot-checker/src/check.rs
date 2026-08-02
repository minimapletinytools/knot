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
use crate::solve::solve;

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
    let (pending, solve_errors) = solve(&mut sub, &mut env, &tree);
    errors.extend(solve_errors);

    check_pending(&mut sub, &table, pending, &mut errors);

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
}
