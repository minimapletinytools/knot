//! The always-in-scope environment every module starts with, before its own
//! imports or declarations are considered — built-in types, their constructors,
//! the closed interface set, and closed-interface method names. Nothing here
//! needs an `import`, matching how `List a`/`Maybe a`/etc. are usable as types
//! without importing anything (spec §5.6), and how Haskell's Prelude auto-exports
//! typeclass methods.
//!
//! `List.map`/`Map.fromList`-style *qualified* access to a type's own module
//! (spec §12's `import List` example) is a separate, currently under-specified
//! question — is `List.map` the same polymorphic `map` collection-interface
//! method spelled with an (unnecessary?) qualifier, or a distinct concrete
//! function belonging to a real `List` stdlib module? This crate doesn't need
//! to answer that yet: qualified references are always resolved via the
//! current module's own `import` list (see `env.rs`), never through this
//! prelude, so the ambiguity simply doesn't arise here.

/// Built-in types usable with no import, per spec §5.1/§5.6/§2.4/§11.
///
/// `Sensitivity` is here as a stub, arity-1 opaque head only — per
/// `knot-type-checker-plan.md` §3.5/§4 (2026-08-01), it's eventually a
/// recursive type-level function (spec §14.6) that expands into a matching
/// record/tuple shape rather than a normal nominal type. None of that
/// expansion is implemented yet: for now `knot-checker` treats it exactly
/// like `Maybe` or any other one-argument built-in — two `Sensitivity a`
/// unify iff their `a`s do, with no introspection into `a`'s own shape.
///
/// `UnravelInput` is here for the same reason `knot-checker`'s
/// `annotation/table.rs` (TM6) now needs it: deriving `unravel`'s expected
/// type (plan §3.5/spec §14.1) builds `UnravelInput A -> UnravelInput B ->
/// ...` from the annotated binding's own signature, so the name needs to
/// resolve. Like `Sensitivity`, it's an opaque arity-1 head only — its
/// actual shape (`type alias UnravelInput a = { orig : a, hints : List a }`)
/// is never unified against here, only referenced by name. Neither type has
/// data constructors of its own (nothing added to `BUILTIN_CONSTRUCTORS`
/// below) — the eventual leaf constraint vocabulary
/// (`Exact`/`Range`/`Tolerance`/`Free`, spec §17) is still TBD and
/// deliberately not added yet, since nothing in `Sensitivity`'s current stub
/// treatment or `unravel`'s derived template needs to look inside it.
pub const BUILTIN_TYPES: &[&str] = &[
    "Bool",
    "Int",
    "Float",
    "String",
    "Unit",
    "List",
    "Map",
    "Maybe",
    "Result",
    "IO",
    "Ordering",
    "Sensitivity",
    "UnravelInput",
];

/// `(constructor name, arity, owning type name)`.
pub const BUILTIN_CONSTRUCTORS: &[(&str, usize, &str)] = &[
    ("True", 0, "Bool"),
    ("False", 0, "Bool"),
    ("Just", 1, "Maybe"),
    ("Nothing", 0, "Maybe"),
    ("Ok", 1, "Result"),
    ("Err", 1, "Result"),
    ("LT", 0, "Ordering"),
    ("EQ", 0, "Ordering"),
    ("GT", 0, "Ordering"),
];

/// The closed interface set — fixed at `Eq`, `Ord`, `Show`, `Semigroup`,
/// `Monoid`, `Num`, `Fractional`, `Integral` (spec §2.3/§10), plus
/// `Collection`/`Context` (spec §10.6); no user-defined interface can
/// ever add to this list. `Collection`/`Context` were missing here for a
/// while after `knot-checker`'s own `interface::table` gained them (Fix #2)
/// — found via live testing (`instance Collection MyType where ...` was
/// rejected at canonicalization with `UnknownInterface`, before ever
/// reaching the type checker) rather than by inspection, since no test
/// anywhere had actually written a real `instance Collection`/`Context`
/// declaration through this crate before.
pub const BUILTIN_INTERFACES: &[&str] = &[
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
];

/// Closed-interface methods and other prelude functions/values usable
/// unqualified with no import (spec §10, §10.6). Symbolic operators
/// (`(==)`, `(+)`, `(<>)`, ...) are deliberately absent: the grammar has no way
/// to reference one as a bare value (no `(op)`-as-expression production exists
/// today, unlike `decl_name`'s parenthesized-operator support for *declaring*
/// an instance method), so they can never appear as an `Expr::Var` needing
/// resolution in the first place.
pub const BUILTIN_VALUES: &[&str] = &[
    // Eq / Ord / Show
    "compare",
    "show", // Num / Fractional / Integral
    "negate",
    "abs",
    "signum",
    "recip",
    "div",
    "mod",
    "fromIntegral",
    // Semigroup / Monoid
    "empty", // Collection interface (§10.6)
    "map",
    "foldl",
    "foldr",
    "filter",
    "length",
    // Context interface (§10.6)
    "pure",
    "bind", // Booleans (§7.4's "Boolean Operators" note)
    "not",
];

pub fn is_builtin_type(name: &str) -> bool {
    BUILTIN_TYPES.contains(&name)
}

pub fn is_builtin_interface(name: &str) -> bool {
    BUILTIN_INTERFACES.contains(&name)
}

pub fn is_builtin_value(name: &str) -> bool {
    BUILTIN_VALUES.contains(&name)
}

/// `(arity, owning type name)` for a built-in constructor, if `name` is one.
pub fn builtin_constructor(name: &str) -> Option<(usize, &'static str)> {
    BUILTIN_CONSTRUCTORS
        .iter()
        .find(|(ctor, _, _)| *ctor == name)
        .map(|(_, arity, ty)| (*arity, *ty))
}
