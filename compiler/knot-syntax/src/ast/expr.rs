use crate::ast::pattern::Pattern;
use crate::ast::Name;
use crate::span::Spanned;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    Unit,
    /// Lowercase, possibly qualified: `x`, `List.map`.
    Var(Name),
    /// Uppercase, possibly qualified: `Circle`, `Shape.Circle` — also covers
    /// `True`/`False`, which are just 0-arity constructors (no separate `BoolLit`,
    /// consistent with how `Pattern::Ctor` already treats them).
    Ctor(Name),
    /// `_` only — named holes (`_name`) are pattern/binding-only, never an
    /// expression placeholder (`f a _b c` is invalid; see language-spec-notes.md §15).
    Hole,
    Lambda(Vec<Spanned<Pattern>>, Box<Spanned<Expr>>),
    App(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
    /// Already precedence-resolved — never a flat operator chain (unlike Elm, which
    /// defers that to a later canonicalization pass because operator fixity can come
    /// from imports; Knot's operator set is closed, so this parser resolves it
    /// directly via precedence-climbing over the §7.4 table).
    BinOp(BinOp, Box<Spanned<Expr>>, Box<Spanned<Expr>>),
    /// A bare operator section (`(+)`, `(:)`, `(<>)`, ...): a first-class
    /// reference to that operator's own function, spelled the same
    /// parenthesized way `decl_name` names one. This does **not** desugar
    /// into a lambda (spec §7.6) — it's a second, "regular function call"
    /// representation of the same operator alongside `BinOp`'s infix one;
    /// `(+) a b` is ordinary application (`App(App(OpRef(Add), a), b)`),
    /// ordinary `expr_app` handles building that, nothing here does.
    OpRef(BinOp),
    /// Unary negation — whitespace-disambiguated from subtraction, see spec §7.5.
    Negate(Box<Spanned<Expr>>),
    If(Box<Spanned<Expr>>, Box<Spanned<Expr>>, Box<Spanned<Expr>>),
    Let(Vec<(Spanned<Pattern>, Spanned<Expr>)>, Box<Spanned<Expr>>),
    Case(Box<Spanned<Expr>>, Vec<(Spanned<Pattern>, Spanned<Expr>)>),
    Do(Vec<DoStmt>, Box<Spanned<Expr>>),
    List(Vec<Spanned<Expr>>),
    /// Arity is checked post-parse (≤ 3) — see `validate.rs`.
    Tuple(Vec<Spanned<Expr>>),
    Record(Vec<(String, Spanned<Expr>)>),
    RecordUpdate(Box<Spanned<Expr>>, Vec<(String, Spanned<Expr>)>),
    FieldAccess(Box<Spanned<Expr>>, String),
    /// Prefix `@ann` attached to the closest-*following* atom only — never to a
    /// wider application or operator expression (spec §13.3's binding rule).
    Annotated(Vec<Annotation>, Box<Spanned<Expr>>),
}

/// A single `do`-block statement: `pat <- expr` or a plain `expr`.
#[derive(Debug, Clone, PartialEq)]
pub enum DoStmt {
    Bind(Spanned<Pattern>, Spanned<Expr>),
    Expr(Spanned<Expr>),
}

/// Mirrors the fixed precedence table in spec §7.4 exactly — no `Compose`/`.` or `$`
/// variants, since neither operator exists in Knot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Pow,
    Mul,
    Div,
    IntDiv, // `div`
    Mod,    // `mod`
    Add,
    Sub,
    Append, // `<>`
    Cons,   // `:`
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    And,     // `&&`
    Or,      // `||`
    Pipe,    // `|>`
    Compose, // `>>`
}

/// `@name(args)` desugars into this shape at parse time (single arg -> bare value,
/// multiple args -> tuple, per spec §13 desugaring rule) — one representation, not two.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub key: String,
    pub value: Spanned<Expr>,
}
