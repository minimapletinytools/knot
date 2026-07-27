//! Parser entry points. `parse_module` (M8) is the only one meant to be public;
//! everything else here is an internal building block assembled milestone by
//! milestone — see `knot-ast-parser-plan.md` §5 for the build order.

pub mod annotation;
pub mod decl;
pub mod expr;
pub mod module;
pub mod pattern;
pub mod ty;
