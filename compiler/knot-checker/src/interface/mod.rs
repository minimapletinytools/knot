//! The closed interface set (spec §2.3/§7) and, once a module's own
//! `instance` declarations are known, the table of which `(interface, head
//! type)` pairs actually have one — what a `HasInstance` obligation
//! (`solve::PendingInstance`, once concrete) gets checked against.

pub mod instance;
pub mod table;
