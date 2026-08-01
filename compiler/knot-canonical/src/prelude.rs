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
///
/// `Sensitivity` is here as a stub, arity-1 opaque head only — per
/// `knot-type-checker-plan.md` §3.5/§4 (2026-08-01), it's eventually a
/// recursive type-level function (spec §9.6) that expands into a matching
/// record/tuple shape rather than a normal nominal type. None of that
/// expansion is implemented yet: for now `knot-checker` treats it exactly
/// like `Option` or any other one-argument built-in — two `Sensitivity a`
/// unify iff their `a`s do, with no introspection into `a`'s own shape.
///
/// `UnravelInput` is here for the same reason `knot-checker`'s
/// `annotation/table.rs` (TM6) now needs it: deriving `unravel`'s expected
/// type (plan §3.5/spec §9.1) builds `UnravelInput A -> UnravelInput B ->
/// ...` from the annotated binding's own signature, so the name needs to
/// resolve. Like `Sensitivity`, it's an opaque arity-1 head only — its
/// actual shape (`type alias UnravelInput a = { orig : a, hints : List a }`)
/// is never unified against here, only referenced by name. Neither type has
/// data constructors of its own (nothing added to `BUILTIN_CONSTRUCTORS`
/// below) — the eventual leaf constraint vocabulary
/// (`Exact`/`Range`/`Tolerance`/`Free`, spec §13) is still TBD and
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
    "Option",
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
