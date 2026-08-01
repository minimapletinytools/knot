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
//! Method *names* aren't modeled here yet — nothing in this crate needs to
//! know `Eq`'s method is called `(==)` until `elaborate.rs` (TM7) actually
//! builds a dictionary value to pass around.

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
}
