//! The closed interface set itself: fixed at `Eq`, `Ord`, `Show`,
//! `Semigroup`, `Monoid`, `Num`, `Fractional`, `Integral` (spec §2.3/§7),
//! plus `Collection`/`Context` (spec §6.3/§6.4) — no user-defined interface
//! can ever add to this list, so this is a small hardcoded table, not
//! something built from a module's own declarations (contrast
//! `instance.rs`'s `InstanceTable`, which is). Each entry's superclasses are
//! what `instance::build_instance_table`'s coherence pass checks already
//! exist before accepting e.g. `instance Ord Shape`.
//!
//! **`Collection`/`Context` are the odd ones out**: every other interface's
//! instances range over ordinary *types* (`Num Int`), but these two range
//! over type *constructors* (`Collection List`) — see `ty::Structure::
//! VarApp`'s own doc comment. Nothing in this table needs to know that
//! distinction, though: `is_known_interface`/`superclasses` only ever care
//! about the *name*, not what sort of thing its instances are keyed by. That
//! distinction only matters where an obligation actually gets resolved
//! against a concrete type/constructor (`interface::instance::check_pending`).
//!
//! **Method *shapes*** (Fix #5, `knot-checker-gaps-plan.md`) — `METHODS`
//! restates each of the eight ordinary (non-`Collection`/`Context`)
//! interfaces' own methods symbolically, relative to the interface's own
//! `Self` type, straight from spec §6/§7. `constrain::decl::
//! constrain_instance` instantiates a method's shape against whatever
//! concrete type a specific `instance` targets to get the real expected
//! type to check that method's body against — the same idea as an ordinary
//! signed top-level binding's signature, just synthesized instead of
//! user-written.
//!
//! **`Collection`/`Context` need a genuinely different shape, `CTOR_
//! METHODS`/`CtorMethodShape`, not just more `MethodShape` entries.** Their
//! own `Self` is a type *constructor* (`instance Collection List` names the
//! bare constructor, never applied to anything, unlike `instance Eq Shape`)
//! — so it can only ever appear *applied* (`SelfApp`), never bare the way
//! `MethodShape::SelfTy` is, and a method can introduce its own extra type
//! variables with nothing to do with `Self` at all (`map`'s `a`/`b`), which
//! `MethodShape` has no way to express since every ordinary interface's
//! methods only ever mention `Self` and fixed concrete types. Reusing one
//! enum for both would mean either shape admitting nonsensical
//! combinations (a bare `SelfTy` in a `Collection` method, an unbound `Var`
//! in an `Eq` one) that would only ever be caught by a runtime panic
//! instead of not type-checking in the first place.

/// One interface method's shape, relative to the interface's own `Self`
/// type — instantiated against a real target by `constrain::decl::
/// instantiate_method_shape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodShape {
    SelfTy,
    Fn(&'static MethodShape, &'static MethodShape),
    Bool,
    Ordering,
    StringTy,
}

use MethodShape::{Bool, Fn, Ordering, SelfTy, StringTy};

/// `(interface, &[(method name, shape)])` — spec §6.1/§6.2's own tables,
/// restated symbolically. Operator methods are keyed by their bare symbol
/// (`"=="`, not `"(==)"`) — `knot-syntax::parse::decl::decl_name` already
/// strips the parens when parsing `(==) a b = ...`.
pub const METHODS: &[(&str, &[(&str, MethodShape)])] = &[
    ("Eq", &[("==", Fn(&SelfTy, &Fn(&SelfTy, &Bool)))]),
    ("Ord", &[("compare", Fn(&SelfTy, &Fn(&SelfTy, &Ordering)))]),
    ("Show", &[("show", Fn(&SelfTy, &StringTy))]),
    ("Semigroup", &[("<>", Fn(&SelfTy, &Fn(&SelfTy, &SelfTy)))]),
    ("Monoid", &[("empty", SelfTy)]),
    (
        "Num",
        &[
            ("+", Fn(&SelfTy, &Fn(&SelfTy, &SelfTy))),
            ("-", Fn(&SelfTy, &Fn(&SelfTy, &SelfTy))),
            ("*", Fn(&SelfTy, &Fn(&SelfTy, &SelfTy))),
            ("negate", Fn(&SelfTy, &SelfTy)),
            ("abs", Fn(&SelfTy, &SelfTy)),
            ("signum", Fn(&SelfTy, &SelfTy)),
        ],
    ),
    (
        "Fractional",
        &[
            ("/", Fn(&SelfTy, &Fn(&SelfTy, &SelfTy))),
            ("recip", Fn(&SelfTy, &SelfTy)),
        ],
    ),
    (
        "Integral",
        &[
            ("div", Fn(&SelfTy, &Fn(&SelfTy, &SelfTy))),
            ("mod", Fn(&SelfTy, &Fn(&SelfTy, &SelfTy))),
        ],
    ),
];

/// The shape of `interface`'s own `method`, if both are known — `None` for
/// an unknown interface, or a method name that doesn't match any of the
/// interface's own known methods (see `constrain_instance`'s own doc
/// comment on what happens to those).
pub fn method_shape(interface: &str, method: &str) -> Option<&'static MethodShape> {
    METHODS
        .iter()
        .find(|(name, _)| *name == interface)
        .and_then(|(_, methods)| methods.iter().find(|(name, _)| *name == method))
        .map(|(_, shape)| shape)
}

/// A `Collection`/`Context` method's own shape (spec §6.3/§6.4) — see
/// module docs on why this needs to be a separate type from `MethodShape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtorMethodShape {
    /// One of this method's own extra type variables (e.g. `map`'s `a`/
    /// `b`) — the same name within one method's shape always means the
    /// same variable; two different methods never share one, even if they
    /// happen to reuse the same letter.
    Var(&'static str),
    Fn(&'static CtorMethodShape, &'static CtorMethodShape),
    /// `Self` applied to one argument — `f a` in `map :: (a -> b) -> f a ->
    /// f b`. `Self` itself is never meaningful unapplied here, unlike
    /// `MethodShape::SelfTy`, so there's no bare-`Self` variant to have.
    SelfApp(&'static CtorMethodShape),
    Bool,
    IntTy,
}

use CtorMethodShape::{Fn as CFn, SelfApp, Var};

/// `(interface, &[(method name, shape)])` for `Collection`/`Context`
/// (spec §6.3/§6.4), the same shape `METHODS` is for the other eight.
pub const CTOR_METHODS: &[(&str, &[(&str, CtorMethodShape)])] = &[
    (
        "Collection",
        &[
            (
                "map",
                CFn(
                    &CFn(&Var("a"), &Var("b")),
                    &CFn(&SelfApp(&Var("a")), &SelfApp(&Var("b"))),
                ),
            ),
            (
                "foldl",
                CFn(
                    &CFn(&Var("b"), &CFn(&Var("a"), &Var("b"))),
                    &CFn(&Var("b"), &CFn(&SelfApp(&Var("a")), &Var("b"))),
                ),
            ),
            (
                "foldr",
                CFn(
                    &CFn(&Var("a"), &CFn(&Var("b"), &Var("b"))),
                    &CFn(&Var("b"), &CFn(&SelfApp(&Var("a")), &Var("b"))),
                ),
            ),
            (
                "filter",
                CFn(
                    &CFn(&Var("a"), &CtorMethodShape::Bool),
                    &CFn(&SelfApp(&Var("a")), &SelfApp(&Var("a"))),
                ),
            ),
            ("length", CFn(&SelfApp(&Var("a")), &CtorMethodShape::IntTy)),
        ],
    ),
    (
        "Context",
        &[
            ("pure", CFn(&Var("a"), &SelfApp(&Var("a")))),
            (
                "bind",
                CFn(
                    &SelfApp(&Var("a")),
                    &CFn(&CFn(&Var("a"), &SelfApp(&Var("b"))), &SelfApp(&Var("b"))),
                ),
            ),
        ],
    ),
];

/// The shape of `interface`'s own `method`, for `Collection`/`Context`
/// specifically — the `CtorMethodShape` counterpart of `method_shape`.
pub fn ctor_method_shape(interface: &str, method: &str) -> Option<&'static CtorMethodShape> {
    CTOR_METHODS
        .iter()
        .find(|(name, _)| *name == interface)
        .and_then(|(_, methods)| methods.iter().find(|(name, _)| *name == method))
        .map(|(_, shape)| shape)
}

pub const INTERFACES: &[(&str, &[&str])] = &[
    ("Eq", &[]),
    ("Ord", &["Eq"]),
    ("Show", &[]),
    ("Semigroup", &[]),
    ("Monoid", &["Semigroup"]),
    ("Num", &[]),
    ("Fractional", &["Num"]),
    // spec §6.2: `interface (Num a, Ord a) => Integral a where ...`
    ("Integral", &["Num", "Ord"]),
    ("Collection", &[]),
    ("Context", &[]),
];

pub fn is_known_interface(name: &str) -> bool {
    INTERFACES.iter().any(|(n, _)| *n == name)
}

pub fn superclasses(name: &str) -> &'static [&'static str] {
    INTERFACES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, supers)| *supers)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_interface_is_known() {
        for name in [
            "Eq",
            "Ord",
            "Show",
            "Semigroup",
            "Monoid",
            "Num",
            "Fractional",
            "Integral",
            "Collection",
            "Context",
        ] {
            assert!(
                is_known_interface(name),
                "{name} should be a known interface"
            );
        }
    }

    #[test]
    fn unknown_interface_is_rejected() {
        assert!(!is_known_interface("Frobnicable"));
    }

    #[test]
    fn superclasses_match_spec_6_1_and_6_2() {
        assert_eq!(superclasses("Ord"), &["Eq"]);
        assert_eq!(superclasses("Monoid"), &["Semigroup"]);
        assert_eq!(superclasses("Fractional"), &["Num"]);
        assert_eq!(superclasses("Integral"), &["Num", "Ord"]);
        assert!(superclasses("Eq").is_empty());
    }

    #[test]
    fn every_core_interface_method_has_a_shape() {
        for (interface, methods) in [
            ("Eq", &["=="] as &[&str]),
            ("Ord", &["compare"]),
            ("Show", &["show"]),
            ("Semigroup", &["<>"]),
            ("Monoid", &["empty"]),
            ("Num", &["+", "-", "*", "negate", "abs", "signum"]),
            ("Fractional", &["/", "recip"]),
            ("Integral", &["div", "mod"]),
        ] {
            for method in methods {
                assert!(
                    method_shape(interface, method).is_some(),
                    "{interface}'s {method} should have a shape"
                );
            }
        }
    }

    #[test]
    fn monoids_empty_is_a_bare_self_ty_with_no_arguments() {
        assert_eq!(method_shape("Monoid", "empty"), Some(&SelfTy));
    }

    #[test]
    fn unknown_interface_or_method_has_no_shape() {
        assert!(method_shape("Frobnicable", "==").is_none());
        assert!(method_shape("Eq", "bogus").is_none());
    }

    #[test]
    fn collection_and_context_have_no_ordinary_method_shape() {
        // They're covered by ctor_method_shape instead -- see its own tests.
        assert!(method_shape("Collection", "map").is_none());
        assert!(method_shape("Context", "bind").is_none());
    }

    #[test]
    fn every_collection_and_context_method_has_a_ctor_shape() {
        for (interface, methods) in [
            (
                "Collection",
                &["map", "foldl", "foldr", "filter", "length"] as &[&str],
            ),
            ("Context", &["pure", "bind"]),
        ] {
            for method in methods {
                assert!(
                    ctor_method_shape(interface, method).is_some(),
                    "{interface}'s {method} should have a ctor shape"
                );
            }
        }
    }

    #[test]
    fn unknown_ctor_interface_or_method_has_no_shape() {
        assert!(ctor_method_shape("Frobnicable", "map").is_none());
        assert!(ctor_method_shape("Collection", "bogus").is_none());
    }
}
