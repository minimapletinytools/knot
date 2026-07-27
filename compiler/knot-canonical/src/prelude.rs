//! The always-in-scope environment every module starts with, before its own
//! imports or declarations are considered — built-in types, their constructors,
//! the closed interface set, and closed-interface method names. Nothing here
//! needs an `import`, matching how `List a`/`Option a`/etc. are usable as types
//! without importing anything (spec §3.6), and how Haskell's Prelude auto-exports
//! typeclass methods.
//!
//! `List.map`/`Map.fromList`-style *qualified* access to a type's own module
//! (spec §9's `import List` example) is a separate, currently under-specified
//! question — is `List.map` the same polymorphic `map` collection-interface
//! method spelled with an (unnecessary?) qualifier, or a distinct concrete
//! function belonging to a real `List` stdlib module? This crate doesn't need
//! to answer that yet: qualified references are always resolved via the
//! current module's own `import` list (see `env.rs`), never through this
//! prelude, so the ambiguity simply doesn't arise here.

/// Built-in types usable with no import, per spec §3.1/§3.6/§6.1/§8.
pub const BUILTIN_TYPES: &[&str] = &[
    "Bool", "Int", "Float", "String", "Unit", "List", "Map", "Option", "Result", "IO", "Ordering",
];

/// `(constructor name, arity, owning type name)`.
pub const BUILTIN_CONSTRUCTORS: &[(&str, usize, &str)] = &[
    ("True", 0, "Bool"),
    ("False", 0, "Bool"),
    ("Some", 1, "Option"),
    ("None", 0, "Option"),
    ("Ok", 1, "Result"),
    ("Err", 1, "Result"),
    ("LT", 0, "Ordering"),
    ("EQ", 0, "Ordering"),
    ("GT", 0, "Ordering"),
];

/// The closed interface set — fixed at `Eq`, `Ord`, `Show`, `Semigroup`,
/// `Monoid`, `Num`, `Fractional`, `Integral` (spec §2.3/§7); no user-defined
/// interface can ever add to this list.
pub const BUILTIN_INTERFACES: &[&str] = &[
    "Eq",
    "Ord",
    "Show",
    "Semigroup",
    "Monoid",
    "Num",
    "Fractional",
    "Integral",
];

/// Closed-interface methods and other prelude functions/values usable
/// unqualified with no import (spec §6, §6.3, §6.4). Symbolic operators
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
    "empty", // Collection interface (§6.3)
    "map",
    "foldl",
    "foldr",
    "filter",
    "length",
    // Context interface (§6.4)
    "pure",
    "bind", // Booleans (§4.8's "Boolean Operators" note)
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
