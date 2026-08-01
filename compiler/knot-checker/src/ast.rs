//! The Elaborated AST (plan §6's "`TExpr`/`TPattern`/..."): `CExpr`'s shape,
//! with a resolved type on every node and every interface-constrained call
//! site carrying its dictionary explicitly. This is the crate's actual
//! deliverable (plan §7, TM7) — everything before this milestone exists to
//! produce the information this AST records.
//!
//! **What's actually implemented here vs. still open.** `Dictionary`
//! *resolution* — given a concrete `(interface, TypeVarId)` obligation and
//! a solved `Substitution` + `InstanceTable`, determine exactly which
//! instance answers it — is fully implemented and tested
//! (`elaborate::resolve_dictionary`/`resolve_pending`). Wiring that into a
//! complete tree walk that turns an arbitrary `CExpr` into a whole `TExpr`
//! needs `constrain::expr`/`constrain::pattern` to *also* build a parallel
//! annotated tree as they generate constraints — today they only return a
//! bare `TypeVarId` per node, with no way to recover, after the fact,
//! which `TypeVarId` belonged to which `CExpr` node (the `Constraint` list
//! they populate is a flat `Vec`, not shaped like the original tree, so
//! there's nothing to walk "in lockstep" with `CExpr` post hoc). That's
//! real, concrete, well-understood follow-up work — extend TM3's functions
//! to return `(TypeVarId, TExpr)` instead of just `TypeVarId` — not a
//! design gap being glossed over. Deliberately not attempted as a
//! shortcut here: a version that *looked* like full elaboration but
//! silently correlated the wrong node to the wrong type would be worse
//! than not having one.

use knot_canonical::ast::Ref;

use crate::var::TypeVarId;

/// Which instance answers one `HasInstance` obligation, resolved. Doesn't
/// inline the instance's own method bodies (those still live in whichever
/// `CInstanceDecl`, or the prelude, they came from) — this only identifies
/// *which* instance a given call site should use. A runtime (Twine) is what
/// resolves `(interface, head)` into the actual method implementations to
/// call through; that's an execution concern, not a type-checking one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Dictionary {
    pub interface: String,
    pub head: Ref,
}

/// A constrained reference, elaborated: the original scheme's quantified
/// variables are gone (already instantiated, plan §5's `instantiate`),
/// replaced by the concrete dictionaries each one resolved to, in the same
/// order as the scheme's own `constraints` list.
#[derive(Debug, Clone, PartialEq)]
pub struct ElaboratedRef {
    pub reference: Ref,
    pub ty: TypeVarId,
    pub dictionaries: Vec<Dictionary>,
}
