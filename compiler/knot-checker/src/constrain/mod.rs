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
//! lookup to solving. A lambda/case/do param is the one `Ref::Local` case
//! that truly needs none of this: never generalized, so it resolves
//! immediately against `LocalScope` below, no constraint needed at all. A
//! `let`-bound name is also `Ref::Local` (`knot-canonical` doesn't
//! distinguish the two — see `LocalBinding`), but *does* get generalized,
//! so it needs its own deferred-lookup constraint too: `LookupLocal`, kept
//! separate from `Lookup` because it has no `Ref` to key on, only the
//! binding's own `TypeVarId`.

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
    /// The `let`-bound counterpart of `Lookup`, keyed by `TypeVarId` instead
    /// of `Ref` — a `let`-bound name isn't unique across different `let`
    /// blocks the way a top-level name is, but its own header `TypeVarId`
    /// (an arena index) always is. Resolved against `solve.rs`'s own
    /// `local_env`, populated the same way `Lookup`'s `SchemeEnv` is, just
    /// for `Constraint::Let { top_level: false, .. }` groups instead of
    /// `true` ones — see `LocalScope::promote_to_generalizable` for where
    /// generation decides a reference needs this instead of an immediate
    /// `LocalScope` hit.
    LookupLocal {
        span: Span,
        key: TypeVarId,
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
    /// **`top_level` — which scheme environment a member's generalized
    /// scheme goes into, not whether it gets generalized at all.**
    /// `knot-canonical`'s `Ref` has no separate case for "let-bound name" —
    /// its own doc comment groups `let` in with lambda/case/do under
    /// `Ref::Local` (all just "found in an enclosing local scope," since
    /// name *resolution* has no reason to care about polymorphism). A
    /// reference to a `let`-bound name therefore arrives as `Ref::Local`
    /// exactly like a lambda parameter, indistinguishable at the `Ref`
    /// level — so the distinction has to live in `LocalScope` instead (see
    /// `LocalBinding`): monomorphic while a group's own `header_con` is
    /// being generated (self/mutual reference, same as always), then
    /// promoted to `Generalizable` for everything generated afterward, for
    /// a `let`-expression group specifically (`constrain::decl`'s
    /// `promote_to_generalizable` call — a top-level group's frame is
    /// simply popped at that point instead, since it's referenced via
    /// `Ref::TopLevel`/`Lookup` from then on, never `LocalScope` again).
    /// Both kinds of group still get the *same* generalize-and-
    /// ambiguous-CAF treatment once `header_con` solves (`solve.rs`) — only
    /// the destination differs: `top_level: true` installs into the global
    /// `SchemeEnv` keyed by name; `top_level: false` installs into
    /// `solve.rs`'s `local_env`, keyed by each member's own `header_ty`
    /// (matching `LookupLocal`'s own key).
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

/// What a name found in `LocalScope` actually means for constraint
/// generation — see `Constraint::LookupLocal`'s own doc comment for the
/// full story of why this distinction exists at all (in short:
/// `knot-canonical`'s `Ref::Local` can't tell a lambda parameter from a
/// `let`-bound name, but they need different treatment here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalBinding {
    /// A lambda/case/do param, or a group member's own name while that
    /// group's `header_con` is still being generated (self/mutual
    /// reference) — always resolves to this exact `TypeVarId`, every time,
    /// no constraint needed at all.
    Monomorphic(TypeVarId),
    /// A `let`-bound name, once its own group is done being defined —
    /// keyed by its own header `TypeVarId` rather than by name (a bare
    /// name isn't unique across different `let` blocks; the arena index
    /// always is). Each reference gets its own `Constraint::LookupLocal`,
    /// resolved to a fresh instantiation later — real let-polymorphism,
    /// the same mechanism `Ref::TopLevel`'s `Lookup` already gets.
    Generalizable(TypeVarId),
}

impl LocalBinding {
    /// The `TypeVarId` this binding is *about*, regardless of which case —
    /// used where the distinction doesn't matter yet (binding it in the
    /// first place, or a self/mutual reference during a group's own
    /// `header_con`, which is always monomorphic no matter what the name
    /// will be promoted to afterward).
    pub fn header_ty(self) -> TypeVarId {
        match self {
            LocalBinding::Monomorphic(ty) | LocalBinding::Generalizable(ty) => ty,
        }
    }
}

/// Tracks simple local bindings (lambda params, case/as patterns, do binds,
/// and `let`-bound names before *and* after they're promoted) during
/// constraint generation. Mirrors `knot_canonical::env::Env`'s scope-stack
/// shape, mapping to a `LocalBinding` instead of just tracking presence.
#[derive(Debug, Default)]
pub struct LocalScope {
    frames: Vec<HashMap<String, LocalBinding>>,
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

    /// Binds `name` monomorphically — what every lambda/case/do param gets,
    /// and what a `let`/top-level group's own members get *while* that
    /// group's `header_con` is being generated (see
    /// `promote_to_generalizable` for the `let`-specific next step).
    pub fn bind(&mut self, name: &str, ty: TypeVarId) {
        self.frames
            .last_mut()
            .expect("LocalScope::bind called with no open scope")
            .insert(name.to_string(), LocalBinding::Monomorphic(ty));
    }

    /// Switches an already-bound name, in the current (innermost) frame, to
    /// `Generalizable` — called by `constrain::decl` right after a
    /// `let`-expression group's own `header_con` has been generated, so
    /// every reference from that point on (a later sibling, or the `in`
    /// body) defers through `Constraint::LookupLocal` instead of resolving
    /// immediately. Never called for a top-level group — that frame is
    /// popped entirely at the same point instead, since top-level names are
    /// referenced via `Ref::TopLevel`/`Lookup`, not `LocalScope`, from then
    /// on.
    pub fn promote_to_generalizable(&mut self, name: &str) {
        let ty = self.lookup(name).header_ty();
        self.frames
            .last_mut()
            .expect("promote_to_generalizable called with no open scope")
            .insert(name.to_string(), LocalBinding::Generalizable(ty));
    }

    /// Panics if `name` isn't bound — `knot-canonical` already guarantees
    /// every `Ref::Local` it produces refers to a real enclosing binding, so
    /// reaching this with an unknown name means a bug in this crate, not a
    /// user error to report gracefully.
    pub fn lookup(&self, name: &str) -> LocalBinding {
        self.try_lookup(name).unwrap_or_else(|| {
            panic!("Ref::Local({name:?}) not found in scope -- knot-canonical should have already guaranteed this")
        })
    }

    /// Non-panicking lookup. Also used for `Ref::TopLevel` names (see
    /// `constrain::expr::constrain_name_ref`) — while a `let`/top-level
    /// binding group is being checked (`constrain::decl`), its own members
    /// are pushed here just like any local binding, so a self/mutually-
    /// recursive reference resolves monomorphically instead of going
    /// through the deferred `Lookup` constraint. A `Ref::TopLevel` name
    /// belonging to some *other*, already-processed group correctly falls
    /// through (`None`) to that deferred path instead.
    pub fn try_lookup(&self, name: &str) -> Option<LocalBinding> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.get(name))
            .copied()
    }
}
