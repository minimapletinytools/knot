//! Constraint generation: walks a `CExpr`/`CPattern` tree and produces a
//! `Constraint` tree plus a type for the node, without ever calling `unify`
//! directly — solving (a later milestone) is a separate pass that walks
//! `Constraint` and does the actual unification, matching Elm's own
//! architecture (plan §2: "separate constraint generation from constraint
//! solving").
//!
//! **`Lookup` is the "`CLocal`-equivalent" TM3's plan entry names.** A
//! reference to anything other than a simple local binding (`Ref::TopLevel`,
//! `Ref::Imported`, `Ref::Builtin`) can't be resolved to a concrete type
//! during generation — a top-level binding's own principal type isn't known
//! until *its* `CLet` group is solved (TM4/TM5), and a builtin's signature
//! lives in a prelude table that's solving's concern, not generation's. So
//! generation just records "look this name up, instantiate whatever scheme
//! is found, and unify the result against `expected`" and defers the actual
//! lookup to solving. `Ref::Local` needs none of this — lambda/case/do
//! bindings are never generalized, so they're resolved immediately against
//! `LocalScope` below, with no constraint needed at all.

pub mod expr;
pub mod pattern;

use std::collections::{BTreeMap, HashMap};

use knot_canonical::ast::Ref;
use knot_syntax::span::Span;

use crate::var::TypeVarId;

#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    True,
    Equal {
        span: Span,
        expected: TypeVarId,
        actual: TypeVarId,
    },
    /// A signature's `Ord a =>`-style obligation on a concrete, now-inferred
    /// type. Nothing in this milestone emits one yet (that needs the
    /// operator/interface table BinOp and Negate depend on) — the variant
    /// exists now so `Equal`'s sibling shapes are all settled at once.
    HasInstance {
        span: Span,
        interface: String,
        ty: TypeVarId,
    },
    /// Deferred name lookup — see module docs.
    Lookup {
        span: Span,
        reference: Ref,
        expected: TypeVarId,
    },
    And(Vec<Constraint>),
    /// `let`/top-level binding-group scoping and generalization (plan §2/§4)
    /// — shape only, matching `ty.rs`'s Structure/Scheme precedent. Nothing
    /// constructs one of these until TM4's SCC splitting exists.
    Let {
        rigid_vars: Vec<TypeVarId>,
        flex_vars: Vec<TypeVarId>,
        header: BTreeMap<String, TypeVarId>,
        header_con: Box<Constraint>,
        body_con: Box<Constraint>,
    },
}

/// Tracks *only* simple local bindings (lambda params, case/as patterns, do
/// binds) during constraint generation — never generalized, so a name here
/// maps straight to the `TypeVarId` it was bound with, no `Scheme` involved.
/// Mirrors `knot_canonical::env::Env`'s scope-stack shape, mapping to a type
/// instead of just tracking presence.
#[derive(Debug, Default)]
pub struct LocalScope {
    frames: Vec<HashMap<String, TypeVarId>>,
}

impl LocalScope {
    pub fn new() -> Self {
        LocalScope { frames: Vec::new() }
    }

    pub fn push(&mut self) {
        self.frames.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        self.frames.pop();
    }

    pub fn bind(&mut self, name: &str, ty: TypeVarId) {
        self.frames
            .last_mut()
            .expect("LocalScope::bind called with no open scope")
            .insert(name.to_string(), ty);
    }

    /// Panics if `name` isn't bound — `knot-canonical` already guarantees
    /// every `Ref::Local` it produces refers to a real enclosing binding, so
    /// reaching this with an unknown name means a bug in this crate, not a
    /// user error to report gracefully.
    pub fn lookup(&self, name: &str) -> TypeVarId {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.get(name))
            .copied()
            .unwrap_or_else(|| {
                panic!("Ref::Local({name:?}) not found in scope -- knot-canonical should have already guaranteed this")
            })
    }
}
