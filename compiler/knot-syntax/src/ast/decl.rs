use crate::ast::expr::{Annotation, Expr};
use crate::ast::pattern::Pattern;
use crate::ast::ty::{Constraint, Type, TypeSignature};
use crate::span::Spanned;

/// A type signature (`name :: Type`) and its definition (`name arg = expr`) are two
/// source lines but *one* node: the declaration parser looks ahead after an optional
/// signature line and merges it with the following definition for the same name.
/// Stacked/block annotations sit above the signature line but conceptually annotate
/// the merged binding, so they attach here — there's no separate `TypeSig` variant.
#[derive(Debug, Clone, PartialEq)]
pub struct FnDef {
    pub name: String,
    pub signature: Option<Spanned<TypeSignature>>,
    pub params: Vec<Spanned<Pattern>>,
    pub body: Spanned<Expr>,
    pub annotations: Vec<Annotation>,
}

/// Haskell-style: `instance Eq Shape where (==) a b = ...`. Methods reuse the same
/// one-equation-per-name rule as any other function (spec §2.3) — no special-casing.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceDecl {
    pub interface: String,
    /// e.g. `Eq a =>` for `instance Eq a => Eq (Maybe a)`.
    pub constraints: Vec<Constraint>,
    /// `Shape`, or `Maybe a`, etc.
    pub target: Type,
    /// Each method's `signature` is always `None` — inherited from the interface.
    pub methods: Vec<FnDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    Fn(FnDef),
    TypeAlias(String, Vec<String>, Type),
    /// ADT: name, type params, variants (each a constructor name + its
    /// argument types), and derived interfaces (`deriving (Eq, Ord, Show)`,
    /// empty if no `deriving` clause at all) — `type alias` has no
    /// equivalent, since a closed/open record or tuple alias already gets
    /// automatic structural `Eq`/`Ord`/`Show` a different way (spec §10.4).
    TypeDecl(String, Vec<String>, Vec<(String, Vec<Type>)>, Vec<String>),
    Instance(InstanceDecl),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Exposing {
    All,
    Some(Vec<ExposedItem>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExposedItem {
    Value(String),
    /// `Foo` — the type but not its constructors.
    TypeOnly(String),
    /// `Foo(..)` — the type and all its constructors.
    TypeWithVariants(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub module: Vec<String>,
    pub alias: Option<String>,
    pub exposing: Option<Exposing>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub name: Vec<String>,
    pub exposing: Exposing,
    pub imports: Vec<Import>,
    pub decls: Vec<Spanned<Decl>>,
}
