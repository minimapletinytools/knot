//! Type shapes (`Structure`) and generalized schemes (`Scheme`) — the data
//! this milestone defines. The *algorithms* that operate on a `Scheme`
//! (`generalize`, needing a `TypeEnv` to compute `ftv(ty) \ ftv(env)`, and
//! `instantiate`, needing a way to resolve a scheme's own `App` heads) wait
//! for later milestones (TM5 and TM3 respectively, per the plan) once there's
//! real constraint-generation/solving context to hang them on.

use std::collections::BTreeMap;

use knot_canonical::ast::Ref;

use crate::var::TypeVarId;

#[derive(Debug, Clone, PartialEq)]
pub enum Structure {
    /// `Int`, `List a`, `Map k v`, `Sensitivity a`, ... — every named type,
    /// built-in or user-defined, shares this one variant. `Sensitivity` gets
    /// no special case here: per the type-checker plan §3.5/§4's "stub" note,
    /// two `Sensitivity a` unify iff their `a`s do, exactly like `Maybe` —
    /// the eventual recursive expansion (spec §9.6) is a distinct, later
    /// piece of machinery that hasn't been built yet, not a variant of this
    /// enum.
    App(Ref, Vec<TypeVarId>),
    Fn(TypeVarId, TypeVarId),
    Tuple(Vec<TypeVarId>),
    /// Fields plus an optional row-extension variable (spec §3.4) — its
    /// unification (field-gathering) is TM2, not this milestone.
    Record(BTreeMap<String, TypeVarId>, Option<TypeVarId>),
    Unit,
    /// A *resolved* type constructor, standing for the constructor itself
    /// (`List`, `Map`, ...), *partially applied* to whichever of its own
    /// leading arguments are already fixed — empty for a 1-parameter
    /// constructor like `List`/`Maybe` (nothing precedes the one argument a
    /// `VarApp` itself ever varies over), one element for a 2-parameter one
    /// like `Map k v`/`Result e a` (`k`/`e`, with only `v`/`a` left for the
    /// `VarApp` to vary over) — see `unify.rs`'s own `VarApp`-vs-`App` case
    /// for where this actually gets built, by splitting a concrete `App`'s
    /// own argument list at `own_len - VarApp's own arg count`. What a
    /// constructor-sorted variable (see `VarApp`) gets bound to once
    /// unification pins it down. Never appears anywhere a `map`/`foldl`/...
    /// caller's own code can see directly; only ever the target of a
    /// `VarApp` head.
    Ctor(Ref, Vec<TypeVarId>),
    /// A type application headed by a constructor *variable* rather than a
    /// fixed `Ref` — `f a`, `f b`, ... in `map :: (a -> b) -> f a -> f b`
    /// (spec §6.3/§6.4's `Collection`/`Context` interfaces). The head is an
    /// ordinary `TypeVarId`, deliberately reusing the exact same
    /// `Unbound`/`Bound` machinery every other type variable already uses
    /// rather than introducing a separate "constructor slot" sort in
    /// `var.rs`: unbound until unification with a concrete `App` pins it to
    /// a `Ctor(Ref)`, from then on behaving exactly like any other resolved
    /// variable (see `unify.rs`'s `VarApp` cases and `crate::var::
    /// Substitution::fresh_ctor_unbound`'s own doc comment for why no
    /// dedicated `Slot` variant is needed). Deliberately *not* full kind
    /// polymorphism — only ever produced by this crate's own hand-built
    /// `Collection`/`Context` prelude schemes (`prelude.rs`), never by
    /// anything a user's own Knot source can write, matching Knot's closed-
    /// interface philosophy (no user-defined higher-kinded signatures to
    /// support).
    VarApp(TypeVarId, Vec<TypeVarId>),
}

/// A quantified type: `∀vars. constraints => ty`. `vars` are the
/// `Substitution` slots — `Rigid` while the scheme's own binding is being
/// body-checked (plan §5), instantiated to fresh flexible variables at
/// every other use site (TM5). `constraints` are that binding's own
/// `Ord a =>`-style obligations, carried on the *scheme* rather than
/// discharged once and forgotten — instantiating re-derives a fresh
/// `HasInstance` obligation per quantified var from these, so each call site
/// gets its own copy to satisfy against whatever concrete type it actually
/// uses, exactly like the rest of the scheme's shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    pub vars: Vec<TypeVarId>,
    pub constraints: Vec<(TypeVarId, String)>,
    pub ty: TypeVarId,
}

impl Scheme {
    /// A non-generalized type: no quantified variables, no constraints. What
    /// most lambda/case-bound names get directly — `generalize` (TM5) is
    /// only needed at `let`/top-level binding boundaries.
    pub fn monomorphic(ty: TypeVarId) -> Self {
        Scheme {
            vars: Vec::new(),
            constraints: Vec::new(),
            ty,
        }
    }
}
