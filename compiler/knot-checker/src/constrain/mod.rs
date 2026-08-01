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

pub mod decl;
pub mod expr;
pub mod pattern;

use std::collections::HashMap;

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
    /// A signature's `Ord a =>`-style obligation (or `BinOp`/`Negate`'s own
    /// implicit one, e.g. `Add` needing `Num`) on a concrete, now-inferred
    /// type. Checking whether a resolvable instance actually exists is
    /// `interface/table.rs`'s job (TM6) — generation only ever records
    /// *what* must hold.
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
    /// `let`/top-level binding-group scoping and generalization (plan §2/§4).
    /// Built by `constrain::decl` (TM4) from an SCC-ordered dependency
    /// split — one `Let` per strongly-connected component, nested so a
    /// component's dependencies are always the *outer* `Let`s (their own
    /// `body_con` is the next component's `Let`, and so on inward).
    /// `header_con` constrains every member's own params/body, generated
    /// with each member already bound in `LocalScope` to its own
    /// `header_ty` — that's what lets self/mutual references inside the
    /// group resolve monomorphically instead of through `Lookup` (see
    /// `LocalScope::try_lookup`). Actually running `generalize` over each
    /// unsigned member once `header_con` solves — and just restating the
    /// signature directly for a signed one — is `solve.rs`'s job (TM5);
    /// this is still just a generation-time data shape, not the algorithm.
    ///
    /// **`top_level` — a real asymmetry, not a placeholder field.**
    /// `knot-canonical`'s `Ref` has no separate case for "let-bound name" —
    /// its own doc comment groups `let` in with lambda/case/do under
    /// `Ref::Local` (all just "found in an enclosing local scope," since
    /// name *resolution* has no reason to care about polymorphism). That
    /// means a reference to a `let`-bound name is `Ref::Local` exactly like
    /// a lambda parameter, and `constrain::expr::constrain_name_ref`
    /// resolves *every* `Ref::Local` the same way: an immediate, monomorphic
    /// `LocalScope` lookup, never a generalized/instantiated one. So a
    /// `let`-expression's own `Constraint::Let` (`top_level: false`) carries
    /// real member data, but nothing downstream ever looks it up by scheme —
    /// solving only needs to walk its `header_con`/`body_con` for ordinary
    /// unification, not install anything into the scheme environment or run
    /// the ambiguous-CAF check (§3 of the plan) against it. Only a
    /// module-level group (`top_level: true`, from `constrain_module`) is
    /// actually referenced via `Ref::TopLevel` elsewhere and needs the full
    /// generalize-and-install treatment. Net effect, documented as a real
    /// (sound, just occasionally over-conservative) limitation: `let`-bound
    /// names don't get let-polymorphism in this implementation — two uses of
    /// the same `let`-bound value must agree on one type, same as a lambda
    /// parameter would. They also aren't checked for the "no ambiguous
    /// zero-argument bindings" rule top-level bindings get. Only top-level
    /// definitions get the full treatment.
    Let {
        top_level: bool,
        members: Vec<LetMember>,
        header_con: Box<Constraint>,
        body_con: Box<Constraint>,
    },
}

/// One binding within a `Constraint::Let` group.
#[derive(Debug, Clone, PartialEq)]
pub struct LetMember {
    pub name: String,
    pub span: Span,
    /// The placeholder type `header_con` was generated against — self/
    /// mutual references inside the group point straight at this.
    pub header_ty: TypeVarId,
    /// `Some` for a binding with an explicit signature — see
    /// `DeclaredScheme`. `None` means its `Scheme` needs the general
    /// `ftv`-difference `generalize` algorithm instead (plan §4/§5).
    pub declared: Option<DeclaredScheme>,
}

/// A signature restates a binding's scheme directly rather than leaving
/// anything for `generalize` to infer: `rigid_vars` are the signature's own
/// type variables (`Substitution::fresh_rigid` slots, plan §5), and `given`
/// are its `Ord a =>`-style obligations, expressed as `(rigid var,
/// interface)` pairs. These are "given," not "wanted": the body may freely
/// rely on them (checked against this list, not the real instance table —
/// there's no concrete type to look up for a rigid variable), and they
/// become the resulting `Scheme.constraints` for callers to discharge fresh
/// at each instantiation instead.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredScheme {
    pub rigid_vars: Vec<TypeVarId>,
    pub given: Vec<(TypeVarId, String)>,
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
        self.try_lookup(name).unwrap_or_else(|| {
            panic!("Ref::Local({name:?}) not found in scope -- knot-canonical should have already guaranteed this")
        })
    }

    /// Non-panicking lookup. Used for `Ref::TopLevel` names too (see
    /// `constrain::expr::constrain_name_ref`) — while a `let`/top-level
    /// binding group is being checked (TM4/`constrain::decl`), its own
    /// members are pushed here just like any local binding, so a
    /// self/mutually-recursive reference resolves monomorphically instead of
    /// going through the deferred `Lookup` constraint. A `Ref::TopLevel` name
    /// belonging to some *other*, already-processed group correctly falls
    /// through (`None`) to that deferred path instead.
    pub fn try_lookup(&self, name: &str) -> Option<TypeVarId> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.get(name))
            .copied()
    }
}
