//! The Canonical AST: mirrors `knot_syntax::ast` shape-for-shape, except every
//! name-carrying leaf (`Var`, `Ctor`, a type's `Named`) has been resolved to a
//! `Ref` explaining *where that name comes from*, rather than a bare string that
//! might not even exist. Operators, literals, and layout are already fully
//! resolved by the parser, so there's nothing left to do there — this stage is
//! scope resolution only, not type checking (see crate root docs).

use knot_syntax::ast::decl::{Exposing, Import};
use knot_syntax::ast::expr::BinOp;
use knot_syntax::ast::pattern::PatternLiteral;
use knot_syntax::span::Spanned;

/// Where a resolved name actually comes from. Local variable *names* are kept
/// as plain strings rather than uniquified/de-Bruijn-indexed — this stage's job
/// is to confirm every name resolves somewhere and record which case it was,
/// not to rename anything (a future stage can add uniquification if a specific
/// later need, e.g. the node-identity hash, ever requires it).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ref {
    /// Bound by an enclosing lambda param, `let`, `case` pattern, or do-bind.
    Local(String),
    /// A function/type/constructor defined at this module's own top level.
    TopLevel(String),
    /// Brought in from another module, either via qualification (`List.map`)
    /// or an `exposing` list — `module` is the real dotted path, already
    /// resolved through any `as` alias.
    Imported { module: Vec<String>, name: String },
    /// Provided by the always-in-scope prelude (built-in types, constructors,
    /// and closed-interface methods) — see `prelude.rs`. Never needs an import.
    Builtin(String),
    /// Resolution failed (unbound, ambiguous, unknown qualifier, ...) — an
    /// error has already been recorded against this name. Exists purely so
    /// resolution can keep walking the rest of the tree and collect every
    /// error in one pass instead of stopping at the first, rather than as a
    /// value any later stage should ever treat as legitimate.
    Unresolved(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CExpr {
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    Unit,
    Var(Ref),
    Ctor(Ref),
    Hole,
    Lambda(Vec<Spanned<CPattern>>, Box<Spanned<CExpr>>),
    App(Box<Spanned<CExpr>>, Box<Spanned<CExpr>>),
    BinOp(BinOp, Box<Spanned<CExpr>>, Box<Spanned<CExpr>>),
    /// A bare operator reference (`(+)`) -- see `knot_syntax::ast::expr::
    /// Expr::OpRef`. No name to resolve (keyed by the closed `BinOp` enum,
    /// not a string), so this is a straight passthrough from the surface AST.
    OpRef(BinOp),
    Negate(Box<Spanned<CExpr>>),
    If(
        Box<Spanned<CExpr>>,
        Box<Spanned<CExpr>>,
        Box<Spanned<CExpr>>,
    ),
    Let(
        Vec<(Spanned<CPattern>, Spanned<CExpr>)>,
        Box<Spanned<CExpr>>,
    ),
    Case(
        Box<Spanned<CExpr>>,
        Vec<(Spanned<CPattern>, Spanned<CExpr>)>,
    ),
    Do(Vec<CDoStmt>, Box<Spanned<CExpr>>),
    List(Vec<Spanned<CExpr>>),
    Tuple(Vec<Spanned<CExpr>>),
    Record(Vec<(String, Spanned<CExpr>)>),
    RecordUpdate(Box<Spanned<CExpr>>, Vec<(String, Spanned<CExpr>)>),
    FieldAccess(Box<Spanned<CExpr>>, String),
    Annotated(Vec<CAnnotation>, Box<Spanned<CExpr>>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CDoStmt {
    Bind(Spanned<CPattern>, Spanned<CExpr>),
    Expr(Spanned<CExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CAnnotation {
    pub key: String,
    pub value: Spanned<CExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CPattern {
    Wildcard(Option<String>),
    Var(String),
    Literal(PatternLiteral),
    /// Arity against the constructor's declared field count is already checked
    /// during resolution (see `resolve::pattern`) — by the time a `CPattern`
    /// exists, `Vec<Spanned<CPattern>>`'s length is known-correct.
    Ctor(Ref, Vec<Spanned<CPattern>>),
    Tuple(Vec<Spanned<CPattern>>),
    Cons(Box<Spanned<CPattern>>, Box<Spanned<CPattern>>),
    Nil,
    /// `{ x, y }` — see `knot_syntax::ast::pattern::Pattern::Record`. Field
    /// names are never resolved against anything global (same as
    /// `CExpr::Record`'s own fields) — this is a straight passthrough, with
    /// duplicate-name checking already done by `resolve::pattern` (each name
    /// goes through the same `bind_checked` a `Var` pattern does).
    Record(Vec<String>),
    As(Box<Spanned<CPattern>>, String),
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CType {
    Named(Ref, Vec<CType>),
    Var(String),
    /// `f a`, `f a b` — a constructor-*variable* application (spec §10.6),
    /// only ever meaningful for a variable a signature's own constraint
    /// list gives `Collection`/`Context`. The head is a bare `String`, not
    /// a `Ref` — unlike `Named`'s head, it's never resolved against
    /// anything (no lookup makes sense for an ordinary type variable name),
    /// exactly like `Var`'s own head.
    VarApp(String, Vec<CType>),
    Fn(Box<CType>, Box<CType>),
    Tuple(Vec<CType>),
    /// Fields, unresolved spread targets (`{ ..Name, ... }`), and an
    /// optional extension row variable. The spread list is a transient,
    /// resolution-only detail: `resolve::alias::expand_aliases` eliminates
    /// every entry (merging each target's own fields into the first list)
    /// before canonicalization finishes, so it's always empty by the time
    /// anything outside `knot-canonical` — or anything in this crate past
    /// that pass — ever inspects a `CType::Record`.
    Record(Vec<(String, CType)>, Vec<Ref>, Option<String>),
    Unit,
}

/// `interface` stays a plain `String`, not a `Ref` — interfaces are never
/// qualified or imported (the closed built-in set is always in scope by name),
/// so the only thing to check is membership in that set, done once during
/// resolution rather than carried around as a resolved reference afterward.
#[derive(Debug, Clone, PartialEq)]
pub struct CConstraint {
    pub interface: String,
    pub type_var: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CTypeSignature {
    pub constraints: Vec<CConstraint>,
    pub ty: CType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CFnDef {
    pub name: String,
    pub signature: Option<Spanned<CTypeSignature>>,
    pub params: Vec<Spanned<CPattern>>,
    pub body: Spanned<CExpr>,
    pub annotations: Vec<CAnnotation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CInstanceDecl {
    pub interface: String,
    pub constraints: Vec<CConstraint>,
    pub target: CType,
    pub methods: Vec<CFnDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CDecl {
    Fn(CFnDef),
    TypeAlias(String, Vec<String>, CType),
    /// Name, type params, variants, and derived interfaces (mirrors
    /// `knot_syntax::ast::decl::Decl::TypeDecl` — see its own doc comment).
    /// Each name here is already confirmed to be a real, known interface
    /// (`UnknownInterface` reported otherwise, same as an instance's own
    /// constraint list); whether the ADT's own shape actually qualifies for
    /// it is a `knot-checker` policy question, not checked here.
    TypeDecl(String, Vec<String>, Vec<(String, Vec<CType>)>, Vec<String>),
    Instance(CInstanceDecl),
}

/// `imports`/`exposing` are carried over verbatim from the source module —
/// they're bookkeeping consumed while building the `Env` (see `env.rs`), not
/// something with names left to resolve on the `CModule` itself.
#[derive(Debug, Clone, PartialEq)]
pub struct CModule {
    pub name: Vec<String>,
    pub exposing: Exposing,
    pub imports: Vec<Import>,
    pub decls: Vec<Spanned<CDecl>>,
}
