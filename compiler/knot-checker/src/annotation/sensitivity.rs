//! `Sensitivity`'s own type-level expansion (spec §9.6) — currently a stub,
//! per this session's explicit decision (2026-08-01): no recursion into
//! record/tuple shape yet, just an ordinary opaque one-argument type,
//! exactly like `Maybe`. `sensitivity_of` is the single seam
//! `annotation::table`'s `unravel` derivation calls through, so upgrading
//! this later — recursing into `Structure::Record`/`Structure::Tuple`, per
//! the design already written up in `knot-type-checker-plan.md` §3.5/§4 —
//! touches only this file, not any of its callers.

use knot_canonical::ast::Ref;

use crate::ty::Structure;
use crate::var::{Substitution, TypeVarId};

/// `Sensitivity T`, stubbed: always just wraps `ty` opaquely, with zero
/// introspection into its shape — two `Sensitivity a` unify iff their `a`s
/// do, exactly like `Maybe`. See module docs for what upgrading this to
/// the real (record/tuple-recursive) behavior would involve.
pub fn sensitivity_of(sub: &mut Substitution, ty: TypeVarId) -> TypeVarId {
    sub.fresh_bound(Structure::App(
        Ref::Builtin("Sensitivity".to_string()),
        vec![ty],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_opaquely_with_no_introspection_into_the_wrapped_shape() {
        let mut sub = Substitution::new();
        let out = sub.fresh_bound(Structure::Record(Default::default(), None));
        let sens = sensitivity_of(&mut sub, out);
        match sub.resolve_structure(sens) {
            Some(Structure::App(r, args)) => {
                assert_eq!(r, Ref::Builtin("Sensitivity".to_string()));
                // Wraps the *whole* record as one opaque argument -- not a
                // per-field expansion (that's the not-yet-built upgrade).
                assert_eq!(args, vec![out]);
            }
            other => panic!("expected an opaque Sensitivity wrapper, got {other:?}"),
        }
    }
}
