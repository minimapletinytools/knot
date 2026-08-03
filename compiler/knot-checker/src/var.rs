//! Type variables as arena indices (`TypeVarId`) into a `Substitution`'s
//! `Vec<Slot>`, rather than Elm's `Rc<RefCell<_>>`-based mutable pointer graph
//! (`Type.UnionFind`) — see the type-checker plan §2/§4 for why. Same
//! algorithm (union-by-rank, path compression), just array-indexed.
//!
//! A `Slot` is one of four things: `Unbound` (a genuine flexible metavariable,
//! not yet solved for anything), `Link` (a union-find redirect to another
//! slot — never a root itself once path-compressed), `Bound` (resolved to a
//! concrete `Structure`), or `Rigid` (a signature's own type variable, opaque
//! during that signature's body-check — see `unify.rs` and plan §5 on why
//! this needs to behave differently from an ordinary flexible variable).

use crate::ty::Structure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeVarId(u32);

#[derive(Debug, Clone)]
enum Slot {
    Unbound { rank: u32 },
    Link(TypeVarId),
    Bound(Structure),
    Rigid { name: String },
}

#[derive(Debug, Default)]
pub struct Substitution {
    slots: Vec<Slot>,
}

impl Substitution {
    pub fn new() -> Self {
        Substitution { slots: Vec::new() }
    }

    pub fn fresh_unbound(&mut self) -> TypeVarId {
        let id = TypeVarId(self.slots.len() as u32);
        self.slots.push(Slot::Unbound { rank: 0 });
        id
    }

    /// A constructor-sorted variable — `f` in `map :: (a -> b) -> f a -> f
    /// b` (spec §10.6). Mechanically identical to `fresh_unbound`
    /// (same `Slot::Unbound` representation, same union-find plumbing): the
    /// "sort" isn't tracked on the slot at all, only by convention of where
    /// the resulting `TypeVarId` gets placed (a `Structure::VarApp` head,
    /// never an ordinary type position) — see `ty::Structure::VarApp`'s own
    /// doc comment for why a dedicated `Slot` variant isn't needed. This
    /// method exists purely so call sites read as "here's a constructor
    /// variable," not because the `Substitution` itself distinguishes one.
    pub fn fresh_ctor_unbound(&mut self) -> TypeVarId {
        self.fresh_unbound()
    }

    /// A signature's own type variable, opaque to unification against any
    /// concrete type or any *other* rigid variable — see `unify.rs`.
    pub fn fresh_rigid(&mut self, name: String) -> TypeVarId {
        let id = TypeVarId(self.slots.len() as u32);
        self.slots.push(Slot::Rigid { name });
        id
    }

    pub fn fresh_bound(&mut self, structure: Structure) -> TypeVarId {
        let id = TypeVarId(self.slots.len() as u32);
        self.slots.push(Slot::Bound(structure));
        id
    }

    /// Path-compressed find: follows `Link`s to the representative slot,
    /// rewriting every visited slot to point straight at it.
    pub fn find(&mut self, id: TypeVarId) -> TypeVarId {
        match self.slots[id.0 as usize] {
            Slot::Link(next) => {
                let root = self.find(next);
                self.slots[id.0 as usize] = Slot::Link(root);
                root
            }
            _ => id,
        }
    }

    /// The resolved shape at `id`'s root, after path compression — `None` for
    /// an `Unbound` or `Rigid` variable (nothing to recurse into).
    pub fn resolve_structure(&mut self, id: TypeVarId) -> Option<Structure> {
        let root = self.find(id);
        match &self.slots[root.0 as usize] {
            Slot::Bound(s) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn is_rigid(&mut self, id: TypeVarId) -> bool {
        let root = self.find(id);
        matches!(self.slots[root.0 as usize], Slot::Rigid { .. })
    }

    pub fn rigid_name(&mut self, id: TypeVarId) -> Option<String> {
        let root = self.find(id);
        match &self.slots[root.0 as usize] {
            Slot::Rigid { name } => Some(name.clone()),
            _ => None,
        }
    }

    fn rank(&self, id: TypeVarId) -> u32 {
        match self.slots[id.0 as usize] {
            Slot::Unbound { rank } => rank,
            _ => 0,
        }
    }

    /// Unions two *unbound flexible* variables by rank. Only ever called by
    /// `unify` once it's confirmed both sides are unbound (never rigid, never
    /// already-bound) — those cases route through `bind`/`link_to` instead.
    pub fn union_unbound(&mut self, a: TypeVarId, b: TypeVarId) {
        let (a, b) = (self.find(a), self.find(b));
        if a == b {
            return;
        }
        let (ra, rb) = (self.rank(a), self.rank(b));
        match ra.cmp(&rb) {
            std::cmp::Ordering::Less => self.slots[a.0 as usize] = Slot::Link(b),
            std::cmp::Ordering::Greater => self.slots[b.0 as usize] = Slot::Link(a),
            std::cmp::Ordering::Equal => {
                self.slots[b.0 as usize] = Slot::Link(a);
                self.slots[a.0 as usize] = Slot::Unbound { rank: ra + 1 };
            }
        }
    }

    /// Makes `from` an alias for `to`, unconditionally — no rank balancing.
    /// Used specifically when `to` must remain the representative no matter
    /// what (`to` being rigid — see `unify::bind_flexible_to_rigid`), where
    /// ordinary rank-balanced union could otherwise point the link the wrong
    /// way and let a rigid variable's identity quietly disappear into a
    /// flexible one.
    pub fn link_to(&mut self, from: TypeVarId, to: TypeVarId) {
        let (from, to) = (self.find(from), self.find(to));
        if from != to {
            self.slots[from.0 as usize] = Slot::Link(to);
        }
    }

    /// Binds an unbound flexible variable to a concrete structure. The
    /// occurs check is the caller's responsibility (see `unify::bind`) — this
    /// just performs the write.
    pub fn bind(&mut self, id: TypeVarId, structure: Structure) {
        let root = self.find(id);
        self.slots[root.0 as usize] = Slot::Bound(structure);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_vars_are_distinct_and_unbound() {
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        let b = sub.fresh_unbound();
        assert_ne!(a, b);
        assert_eq!(sub.find(a), a);
        assert!(sub.resolve_structure(a).is_none());
        assert!(!sub.is_rigid(a));
    }

    #[test]
    fn union_unbound_makes_find_agree() {
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        let b = sub.fresh_unbound();
        sub.union_unbound(a, b);
        assert_eq!(sub.find(a), sub.find(b));
    }

    #[test]
    fn union_is_idempotent_on_the_same_root() {
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        sub.union_unbound(a, a);
        assert_eq!(sub.find(a), a);
    }

    #[test]
    fn bind_resolves_to_the_given_structure() {
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        sub.bind(a, Structure::Unit);
        assert_eq!(sub.resolve_structure(a), Some(Structure::Unit));
    }

    #[test]
    fn binding_after_union_is_visible_through_either_var() {
        let mut sub = Substitution::new();
        let a = sub.fresh_unbound();
        let b = sub.fresh_unbound();
        sub.union_unbound(a, b);
        let root = sub.find(a);
        sub.bind(root, Structure::Unit);
        assert_eq!(sub.resolve_structure(a), Some(Structure::Unit));
        assert_eq!(sub.resolve_structure(b), Some(Structure::Unit));
    }

    #[test]
    fn rigid_var_reports_its_name_and_no_structure() {
        let mut sub = Substitution::new();
        let a = sub.fresh_rigid("a".to_string());
        assert!(sub.is_rigid(a));
        assert_eq!(sub.rigid_name(a), Some("a".to_string()));
        assert!(sub.resolve_structure(a).is_none());
    }

    #[test]
    fn link_to_makes_the_flexible_side_disappear_into_the_target() {
        let mut sub = Substitution::new();
        let rigid = sub.fresh_rigid("a".to_string());
        let flexible = sub.fresh_unbound();
        sub.link_to(flexible, rigid);
        assert_eq!(sub.find(flexible), sub.find(rigid));
        assert!(sub.is_rigid(flexible));
    }
}
