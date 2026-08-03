//! The Elaborated AST (plan §6's "`TExpr`/`TPattern`/..."): `CExpr`'s shape,
//! with a resolved type on every node and every interface-constrained call
//! site carrying enough information to resolve its dictionary. This is the
//! crate's actual deliverable (plan §7, TM7/Fix #3) — everything before this
//! milestone exists to produce the information this AST records.
//!
//! **`Typed<T>`** wraps a bare node (`TExpr`/`TPattern`) with the `Span`/
//! `TypeVarId` every node needs, so the enums themselves don't have to
//! repeat those two fields on every single variant the way `CExpr`/
//! `CPattern` would if they were retrofitted in place. `TExpr`/`TPattern`
//! otherwise mirror `CExpr`/`CPattern` variant-for-variant, with recursive
//! children as `Box<Typed<_>>` instead of `Box<Spanned<_>>`.
//!
//! **Two mechanisms for interface obligations, not one** — see
//! `constrain::expr`'s own doc comment for the generation-time half:
//! `BinOp`/`Negate` know their own `(interface, TypeVarId)` pairs already,
//! at generation time (the same values pushed as `Constraint::HasInstance`),
//! so those live directly on the node (`TExpr::BinOp`/`TExpr::Negate`'s own
//! `Vec<(String, TypeVarId)>` field). A `Var`/`Ctor` resolved via `Lookup`/
//! `LookupLocal` doesn't know its obligations until the referenced scheme is
//! *instantiated*, which only happens during solving — so those carry no
//! such field at all; `elaborate::elaborate_module` (Stage B) recovers them
//! from a side-table `solve::solve_with_obligations` builds during solving,
//! keyed by the reference's own `ty` (a fresh `TypeVarId` allocated once per
//! occurrence, so it's already a unique key with nothing extra needed).
//! `TPattern::Ctor` deliberately carries no such tracking either way: a data
//! constructor's own scheme never has `constraints` in this language (no
//! Knot syntax exists to write a "constrained constructor"), so there is
//! provably nothing to correlate.
//!
//! `Do` has no `TExpr` variant of its own — by the time `constrain_expr`
//! reaches it, `constrain::expr::desugar_do` has already rewritten it into
//! ordinary `App`/`Lambda` nodes calling `bind`/`pure` (spec §10.6/§11), which
//! *is* the correct target-language shape once dictionary-passing is real;
//! do-notation is pure syntax sugar with no runtime meaning of its own.
//! `Annotated` keeps its `CAnnotation`s completely unelaborated, matching
//! `constrain::expr`'s own current scope: annotation values aren't
//! constrained during generation at all (TM6/plan §3.5's job), so there is
//! nothing yet to attach beyond the original, un-typechecked value.
//!
//! **What "done" means for Stage B, honestly.** `elaborate::
//! elaborate_module` resolves every obligation whose type is *concretely*
//! known once solving finishes. An obligation that stayed on a genuinely
//! free type variable is not a `NoInstance` error — it means the enclosing
//! binding's own `generalize` (`solve.rs`) absorbed it into *that binding's*
//! polymorphic `Scheme.constraints` instead (`useEq x y = x == y`'s `Eq`
//! obligation, say), to be discharged by whichever concrete type each of
//! *its own* callers eventually uses. `ObligationResolution::StillAbstract`
//! names this case honestly rather than reporting a spurious error for
//! perfectly valid polymorphic code. Actually compiling a polymorphic
//! binding to *take* an extra dictionary parameter, and threading a
//! caller's own dictionary down through it (the full Wadler & Blott
//! dictionary-passing transform) is real, separate, future work this fix
//! doesn't attempt — telling the two cases apart correctly, without
//! silently misreporting one as the other, is what this milestone commits
//! to.

use knot_canonical::ast::{CAnnotation, Ref};
use knot_syntax::ast::expr::BinOp;
use knot_syntax::ast::pattern::PatternLiteral;
use knot_syntax::span::Span;

use crate::var::TypeVarId;

/// A bare `TExpr`/`TPattern` node plus the `Span`/`TypeVarId` every node
/// needs — see module docs on why this lives outside the enums themselves.
#[derive(Debug, Clone, PartialEq)]
pub struct Typed<T> {
    pub span: Span,
    pub ty: TypeVarId,
    pub node: T,
}

pub type TypedExpr = Typed<TExpr>;
pub type TypedPattern = Typed<TPattern>;

#[derive(Debug, Clone, PartialEq)]
pub enum TExpr {
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    Unit,
    /// See `constrain::expr`'s own doc comment: a hole's type is
    /// deliberately unconstrained, always a compile error to actually
    /// leave in a program, but that's a separate diagnostic concern from
    /// typing.
    Hole,
    Var(Ref),
    Ctor(Ref),
    Lambda(Vec<TypedPattern>, Box<TypedExpr>),
    App(Box<TypedExpr>, Box<TypedExpr>),
    /// `Vec<(String, TypeVarId)>` — this operator's own interface
    /// obligation(s), known already at generation time (see module docs).
    /// Empty for the operators with no interface at all (`Cons`, `And`/
    /// `Or`, `Pipe`, `Compose`).
    BinOp(
        BinOp,
        Box<TypedExpr>,
        Box<TypedExpr>,
        Vec<(String, TypeVarId)>,
    ),
    Negate(Box<TypedExpr>, Vec<(String, TypeVarId)>),
    If(Box<TypedExpr>, Box<TypedExpr>, Box<TypedExpr>),
    Let(Vec<(TypedPattern, TypedExpr)>, Box<TypedExpr>),
    Case(Box<TypedExpr>, Vec<(TypedPattern, TypedExpr)>),
    List(Vec<TypedExpr>),
    Tuple(Vec<TypedExpr>),
    Record(Vec<(String, TypedExpr)>),
    RecordUpdate(Box<TypedExpr>, Vec<(String, TypedExpr)>),
    FieldAccess(Box<TypedExpr>, String),
    /// Deliberately unelaborated — see module docs.
    Annotated(Vec<CAnnotation>, Box<TypedExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TPattern {
    Wildcard(Option<String>),
    Var(String),
    Literal(PatternLiteral),
    Ctor(Ref, Vec<TypedPattern>),
    Tuple(Vec<TypedPattern>),
    Cons(Box<TypedPattern>, Box<TypedPattern>),
    Nil,
    As(Box<TypedPattern>, String),
    Unit,
}

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

/// One obligation's resolution, post-solving — see module docs on why this
/// isn't just `Result<Dictionary, TypeError>`.
#[derive(Debug, Clone, PartialEq)]
pub enum ObligationResolution {
    Concrete(Dictionary),
    /// `interface::instance::check_instance` confirmed this obligation
    /// genuinely holds, but it's headed by a structural type (`Tuple`/
    /// `Record`/`Unit`), which has no single named instance to put in a
    /// `Dictionary`'s own `head` — `List Weird`'s `Eq` fails a `Dictionary`
    /// exactly the same way, if `Weird` itself is Eq-able, `(Int, Bool)`'s
    /// `Eq` genuinely holds, there's just no runtime artifact for it yet
    /// (see `elaborate.rs`'s own doc comment on why dictionary
    /// *construction* for structural types is separate, future work).
    /// Never a type error — the type-checking verdict is correct either
    /// way, this only says elaboration can't go the rest of the way yet.
    Structural,
    StillAbstract,
}
