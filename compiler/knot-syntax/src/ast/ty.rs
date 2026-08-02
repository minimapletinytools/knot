use crate::ast::Name;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// `Int`, `List a`, `Map k v`, `Maybe a`, ...
    Named(Name, Vec<Type>),
    Var(String),
    Fn(Box<Type>, Box<Type>),
    /// Arity is checked post-parse (≤ 3) — see `validate.rs`.
    Tuple(Vec<Type>),
    /// Fields, spread record-alias targets (`{ ..Name, x : Float }` — see
    /// `knot-canonical::resolve::alias`'s own doc comment on why these are
    /// resolved there, not here), and an optional extension row variable,
    /// e.g. `{ r | x : Float }`.
    Record(Vec<(String, Type)>, Vec<String>, Option<String>),
    Unit,
}

/// e.g. `Ord a` in `Ord a => a -> a -> a`.
#[derive(Debug, Clone, PartialEq)]
pub struct Constraint {
    pub interface: String,
    pub type_var: String,
}

/// A full `::` signature: an optional constraint list plus the type itself.
/// Constraints only ever appear as this prefix list before `=>` — they're a property
/// of the signature, not of `Type` itself, so `Type` stays unconstrained-only.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeSignature {
    pub constraints: Vec<Constraint>,
    pub ty: Type,
}
