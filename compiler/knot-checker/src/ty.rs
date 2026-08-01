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
    /// two `Sensitivity a` unify iff their `a`s do, exactly like `Option` —
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
}

/// A quantified type. `vars` are the `Substitution` slots — `Rigid` while the
/// scheme's own binding is being body-checked (plan §5), instantiated to
/// fresh flexible variables at every other use site (TM3).
#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    pub vars: Vec<TypeVarId>,
    pub ty: TypeVarId,
}

impl Scheme {
    /// A non-generalized type: no quantified variables at all. What most
    /// lambda/case-bound names get directly — `generalize` (TM5) is only
    /// needed at `let`/top-level binding boundaries.
    pub fn monomorphic(ty: TypeVarId) -> Self {
        Scheme {
            vars: Vec::new(),
            ty,
        }
    }
}
