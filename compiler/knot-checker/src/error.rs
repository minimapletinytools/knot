//! `unify()` (see `unify.rs`) has no notion of *why* two types were being
//! compared — it only ever sees `TypeVarId`s, never a span. `solve.rs` (a
//! later milestone) is what has a `Constraint`'s span on hand when it calls
//! `unify`, so it's the one that will wrap a bare `UnifyError` into a
//! spanned, module-level `TypeError` — kept as two separate types now rather
//! than threading a span through `unify` prematurely. `TypeError` itself
//! (mismatch/occurs-check plus the later no-instance/AmbiguousConstraint
//! kinds — plan §6) lands once `solve.rs` exists to produce it.

use crate::var::TypeVarId;

#[derive(Debug, Clone, PartialEq)]
pub enum UnifyError {
    Mismatch {
        expected: TypeVarId,
        actual: TypeVarId,
    },
    /// `var` occurs within `in_ty`'s own structure — binding would build an
    /// infinite type.
    Occurs { var: TypeVarId, in_ty: TypeVarId },
}
