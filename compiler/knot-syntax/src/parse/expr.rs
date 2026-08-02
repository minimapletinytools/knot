//! Expression grammar: atoms (literals, identifiers, holes, parens/tuple/unit,
//! list, record/update, field access) -> application -> unary negation ->
//! precedence-climbing binary ops (spec §4.8) -> the layout-heavy forms
//! (`if`/`let`/`case`/lambda/`do`).
//!
//! `if`/`let`/`case`/`do`/lambda are only ever dispatched from the top-level
//! `expr()` entry point, never from `expr_atom` — matching Haskell/Elm, these are
//! not valid as a bare, unparenthesized application argument or operator operand;
//! `f (if c then a else b)` needs the parens.
//!
//! Layout: `expr_app`'s trailing-argument loop and `expr_binop_prec`'s
//! operator-continuation loop both check `continues_layout` before trying to
//! consume more — this is what stops a `let`/`case`/`do` block's *next binding*
//! from being misread as one more application argument or operand of the
//! *previous* one, since it sits at the block's reference column rather than
//! strictly past it.

use crate::ast::expr::{BinOp, DoStmt, Expr};
use crate::error::{ErrorKind, ParseError};
use crate::lex::literal::NumberLiteral;
use crate::span::{Span, Spanned};
use crate::state::ParseState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Assoc {
    Left,
    Right,
    None,
}

struct BinopMatch {
    op: BinOp,
    prec: u8,
    assoc: Assoc,
    len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MinusKind {
    /// Space before, none after: unary negation of the following atom.
    Negation,
    /// Same spacing both sides (both present or both absent): binary subtraction.
    Subtraction,
    /// No space before, but space after: neither pattern, always invalid (spec §4.9).
    Ambiguous,
}

impl<'a> ParseState<'a> {
    /// A full expression. `if`/`let`/`case`/`do`/lambda are recognized here only —
    /// everything else falls through to the operator/application grammar.
    pub fn expr(&mut self) -> Result<Spanned<Expr>, ParseError> {
        self.skip_trivia()?;
        if self.peek_keyword("if") {
            self.expr_if()
        } else if self.peek_keyword("let") {
            self.expr_let()
        } else if self.peek_keyword("case") {
            self.expr_case()
        } else if self.peek_keyword("do") {
            self.expr_do()
        } else if self.peek() == Some(b'\\') {
            self.expr_lambda()
        } else {
            self.expr_binop_prec(0)
        }
    }

    /// Precedence-climbing over the fixed §4.8 table, producing a properly
    /// nested `BinOp` tree directly (never a flat chain to resolve later, unlike
    /// Elm — see `knot-ast-parser-plan.md` §2).
    fn expr_binop_prec(&mut self, min_prec: u8) -> Result<Spanned<Expr>, ParseError> {
        let mut left = self.expr_app()?;
        let mut consumed_nonassoc_at: Option<u8> = None;
        loop {
            let checkpoint = self.clone();
            self.skip_trivia()?;
            if !self.continues_layout() {
                *self = checkpoint;
                break;
            }
            let Some(m) = self.peek_binop() else {
                *self = checkpoint;
                break;
            };
            if m.prec < min_prec {
                *self = checkpoint;
                break;
            }
            if consumed_nonassoc_at == Some(m.prec) {
                return Err(ParseError::new(
                    ErrorKind::Custom(
                        "comparison operators don't chain -- add parentheses to group explicitly"
                            .to_string(),
                    ),
                    Span::new(self.pos.offset, self.pos.offset),
                )
                .fatal());
            }
            for _ in 0..m.len {
                self.bump();
            }
            self.skip_trivia()?;
            let rhs_min_prec = match m.assoc {
                Assoc::Left => m.prec + 1,
                Assoc::Right => m.prec,
                Assoc::None => m.prec + 1,
            };
            let right = self.expr_binop_prec(rhs_min_prec)?;
            let span = Span::new(left.span.start, right.span.end);
            left = Spanned::new(span, Expr::BinOp(m.op, Box::new(left), Box::new(right)));
            if m.assoc == Assoc::None {
                consumed_nonassoc_at = Some(m.prec);
            }
        }
        Ok(left)
    }

    /// Peeks (without consuming) for a recognized binary operator at the current
    /// position. Longest-match-first so e.g. `<=` isn't mistaken for `<`, `>>`
    /// for `>` twice, `::` for cons, etc. `-` defers to `classify_minus` since
    /// it's the one overloaded token in the table.
    fn peek_binop(&self) -> Option<BinopMatch> {
        macro_rules! m {
            ($op:expr, $prec:expr, $assoc:expr, $len:expr) => {
                Some(BinopMatch {
                    op: $op,
                    prec: $prec,
                    assoc: $assoc,
                    len: $len,
                })
            };
        }
        match (self.peek(), self.peek_at(1)) {
            (Some(b'<'), Some(b'=')) => return m!(BinOp::Le, 4, Assoc::None, 2),
            (Some(b'>'), Some(b'=')) => return m!(BinOp::Ge, 4, Assoc::None, 2),
            (Some(b'='), Some(b'=')) => return m!(BinOp::Eq, 4, Assoc::None, 2),
            (Some(b'/'), Some(b'=')) => return m!(BinOp::Neq, 4, Assoc::None, 2),
            (Some(b'<'), Some(b'>')) => return m!(BinOp::Append, 6, Assoc::Left, 2),
            (Some(b'|'), Some(b'>')) => return m!(BinOp::Pipe, 1, Assoc::Left, 2),
            (Some(b'>'), Some(b'>')) => return m!(BinOp::Compose, 1, Assoc::Left, 2),
            (Some(b'&'), Some(b'&')) => return m!(BinOp::And, 3, Assoc::Right, 2),
            (Some(b'|'), Some(b'|')) => return m!(BinOp::Or, 2, Assoc::Right, 2),
            _ => {}
        }
        match self.peek() {
            Some(b'^') => m!(BinOp::Pow, 8, Assoc::Right, 1),
            Some(b'*') => m!(BinOp::Mul, 7, Assoc::Left, 1),
            Some(b'/') => m!(BinOp::Div, 7, Assoc::Left, 1),
            Some(b'+') => m!(BinOp::Add, 6, Assoc::Left, 1),
            Some(b'-') if self.classify_minus() == MinusKind::Subtraction => {
                m!(BinOp::Sub, 6, Assoc::Left, 1)
            }
            Some(b':') if self.peek_at(1) != Some(b':') => m!(BinOp::Cons, 5, Assoc::Right, 1),
            Some(b'<') => m!(BinOp::Lt, 4, Assoc::None, 1),
            Some(b'>') => m!(BinOp::Gt, 4, Assoc::None, 1),
            _ if self.peek_keyword("div") => m!(BinOp::IntDiv, 7, Assoc::Left, 3),
            _ if self.peek_keyword("mod") => m!(BinOp::Mod, 7, Assoc::Left, 3),
            _ => None,
        }
    }

    /// Classifies a `-` at the current position by its surrounding whitespace,
    /// per spec §4.9. Must be called with the cursor positioned exactly at the
    /// `-` (after `skip_trivia`, never before).
    fn classify_minus(&self) -> MinusKind {
        debug_assert_eq!(self.peek(), Some(b'-'));
        let space_before = self.pos.offset > 0
            && matches!(
                self.src[self.pos.offset as usize - 1],
                b' ' | b'\t' | b'\n' | b'\r'
            );
        let space_after = !matches!(self.peek_at(1), Some(b) if !b.is_ascii_whitespace());
        match (space_before, space_after) {
            (true, false) => MinusKind::Negation,
            (false, true) => MinusKind::Ambiguous,
            _ => MinusKind::Subtraction,
        }
    }

    /// One or more atoms in a row (`f a b`), left-associative. Application
    /// arguments must be strictly deeper than the enclosing layout block's
    /// reference column, or a sibling `let`/`case`/`do` item would be misread as
    /// one more argument of the previous one.
    fn expr_app(&mut self) -> Result<Spanned<Expr>, ParseError> {
        let head = self.expr_atom()?;
        let mut args = Vec::new();
        loop {
            let checkpoint = self.clone();
            self.skip_trivia()?;
            if !self.continues_layout() {
                *self = checkpoint;
                break;
            }
            // A `-` shaped like binary subtraction (symmetric spacing) can never
            // start a *new* trailing argument here -- if we let `expr_atom` try
            // it, there is no argument to produce, only an operator for the
            // enclosing `expr_binop_prec` to pick up via `peek_binop`. Bail out
            // before attempting the atom so that case is reached; every other
            // caller of `expr_atom`/`expr_atom_base` has no such fallback (they
            // sit at the very start of a (sub)expression, e.g. right after `(`,
            // `[`, or `=`), so a `-` there is unambiguously negation.
            if self.peek() == Some(b'-') && self.classify_minus() == MinusKind::Subtraction {
                *self = checkpoint;
                break;
            }
            match self.expr_atom() {
                Ok(arg) => args.push(arg),
                Err(err) if err.fatal => return Err(err),
                Err(_) => {
                    *self = checkpoint;
                    break;
                }
            }
        }
        let mut result = head;
        for arg in args {
            let span = Span::new(result.span.start, arg.span.end);
            result = Spanned::new(span, Expr::App(Box::new(result), Box::new(arg)));
        }
        Ok(result)
    }

    /// An atom, plus any immediately-following (no whitespace) `.field` chain —
    /// field access binds tighter than application. A qualified name like
    /// `List.map` is already fully consumed by the identifier lexer itself before
    /// this ever runs; this only handles chains on top of a *value*, e.g.
    /// `point.x.y` or `(getPoint x).x`.
    ///
    /// Also where prefix `@annotation`s attach (spec §10.3): a leading `@`
    /// here binds only to the single atom this call recurses into next
    /// (including its own field-access chain) — never to a wider application
    /// or operator expression, which is exactly why annotating one of those
    /// needs explicit parens (`@{...} (f a b)`, not `@{...} f a b`). Holes
    /// can't carry annotations at all (spec §12.5).
    fn expr_atom(&mut self) -> Result<Spanned<Expr>, ParseError> {
        self.skip_trivia()?;
        if self.peek() == Some(b'@') {
            if self.in_annotation_value {
                return Err(ParseError::new(
                    ErrorKind::Custom(
                        "annotation values cannot themselves carry annotations".to_string(),
                    ),
                    Span::new(self.pos.offset, self.pos.offset),
                )
                .fatal());
            }
            let start = self.pos;
            let annotations = self.annotation_list()?;
            self.skip_trivia()?;
            let target = self.expr_atom()?;
            if target.node == Expr::Hole {
                return Err(ParseError::new(
                    ErrorKind::Custom("holes cannot carry annotations".to_string()),
                    Span::new(start.offset, target.span.end),
                )
                .fatal());
            }
            let span = Span::new(start.offset, target.span.end);
            return Ok(Spanned::new(
                span,
                Expr::Annotated(annotations, Box::new(target)),
            ));
        }
        let mut current = self.expr_atom_base()?;
        loop {
            if self.peek() == Some(b'.')
                && matches!(self.peek_at(1), Some(b) if b.is_ascii_lowercase())
            {
                self.bump(); // '.'
                let field = self.lower_ident()?.node;
                let span = Span::new(current.span.start, self.pos.offset);
                current = Spanned::new(span, Expr::FieldAccess(Box::new(current), field));
            } else {
                break;
            }
        }
        Ok(current)
    }

    fn expr_atom_base(&mut self) -> Result<Spanned<Expr>, ParseError> {
        self.skip_trivia()?;
        let start = self.pos;
        match self.peek() {
            Some(b'_') => {
                let hole = self.hole()?;
                if hole.node.is_some() {
                    return Err(ParseError::new(
                        ErrorKind::Custom(
                            "named holes (_name) are not valid expression placeholders; use a bare `_`"
                                .to_string(),
                        ),
                        hole.span,
                    )
                    .fatal());
                }
                Ok(Spanned::new(hole.span, Expr::Hole))
            }
            Some(b'(') => self.expr_paren_tuple_or_unit(),
            Some(b'[') => self.expr_list(),
            Some(b'{') => self.expr_record_or_update(),
            Some(b'"') => {
                let lit = self.string_literal()?;
                Ok(Spanned::new(lit.span, Expr::StringLit(lit.node)))
            }
            Some(b'-') => match self.classify_minus() {
                // `Subtraction`-shaped spacing (symmetric, e.g. both absent as in
                // `(-40.0)`/`[-5]`, or both present) still means negation here:
                // `expr_atom_base` only ever runs at the start of a (sub)expression
                // -- right after `(`, `[`, `=`, a binop, etc. -- where there is no
                // preceding operand for `-` to subtract from. The one place that
                // spacing pattern legitimately means "stop, this is a binary
                // operator" is `expr_app`'s trailing-argument loop, which checks
                // for it *before* ever calling down into here.
                MinusKind::Negation | MinusKind::Subtraction => {
                    self.bump();
                    let inner = self.expr_atom()?;
                    let span = Span::new(start.offset, inner.span.end);
                    Ok(Spanned::new(span, Expr::Negate(Box::new(inner))))
                }
                MinusKind::Ambiguous => Err(ParseError::new(
                    ErrorKind::Custom(
                        "ambiguous spacing around `-` -- add parentheses or fix spacing to disambiguate \
                         subtraction from negation"
                            .to_string(),
                    ),
                    Span::new(start.offset, start.offset),
                )
                .fatal()),
            },
            Some(b) if b.is_ascii_digit() => {
                let lit = self.number_literal()?;
                let node = match lit.node {
                    NumberLiteral::Int(v) => Expr::IntLit(v),
                    NumberLiteral::Float(v) => Expr::FloatLit(v),
                };
                Ok(Spanned::new(lit.span, node))
            }
            Some(b) if b.is_ascii_uppercase() => {
                let name = self.qualified_upper_start()?;
                // Ends in a lowercase segment (`List.map`) -> qualified value
                // reference; ends in an uppercase segment (`Shape.Circle`, or a
                // bare `Circle`) -> constructor. `qualified_upper_start` always
                // starts uppercase, so only the *last* segment's case matters.
                let last_segment = name.node.rsplit('.').next().unwrap_or(&name.node);
                let is_value_ref = last_segment.starts_with(|c: char| c.is_ascii_lowercase());
                let node = if is_value_ref { Expr::Var(name.node) } else { Expr::Ctor(name.node) };
                Ok(Spanned::new(name.span, node))
            }
            Some(b) if b.is_ascii_lowercase() => {
                let name = self.lower_ident()?;
                Ok(Spanned::new(name.span, Expr::Var(name.node)))
            }
            _ => Err(ParseError::new(ErrorKind::Expected("an expression"), Span::new(start.offset, start.offset))),
        }
    }

    /// `()` (Unit), `(expr)` (transparent grouping), or `(e, e, ...)` (tuple —
    /// arity isn't checked here, see `validate.rs`).
    fn expr_paren_tuple_or_unit(&mut self) -> Result<Spanned<Expr>, ParseError> {
        let start = self.pos;
        self.bump(); // '('
        self.skip_trivia()?;
        if self.peek() == Some(b')') {
            self.bump();
            return Ok(Spanned::new(
                Span::new(start.offset, self.pos.offset),
                Expr::Unit,
            ));
        }
        let first = self.expr()?;
        self.skip_trivia()?;
        if self.peek() == Some(b',') {
            let mut elements = vec![first];
            while self.peek() == Some(b',') {
                self.bump();
                self.skip_trivia()?;
                elements.push(self.expr()?);
                self.skip_trivia()?;
            }
            self.expect_byte(b')')?;
            Ok(Spanned::new(
                Span::new(start.offset, self.pos.offset),
                Expr::Tuple(elements),
            ))
        } else {
            self.expect_byte(b')')?;
            Ok(first)
        }
    }

    fn expr_list(&mut self) -> Result<Spanned<Expr>, ParseError> {
        let start = self.pos;
        self.bump(); // '['
        self.skip_trivia()?;
        if self.peek() == Some(b']') {
            self.bump();
            return Ok(Spanned::new(
                Span::new(start.offset, self.pos.offset),
                Expr::List(Vec::new()),
            ));
        }
        let mut elements = vec![self.expr()?];
        self.skip_trivia()?;
        while self.peek() == Some(b',') {
            self.bump();
            self.skip_trivia()?;
            elements.push(self.expr()?);
            self.skip_trivia()?;
        }
        self.expect_byte(b']')?;
        Ok(Spanned::new(
            Span::new(start.offset, self.pos.offset),
            Expr::List(elements),
        ))
    }

    /// `{ field = expr, ... }` (construction, including `{}`) or
    /// `{ base | field = expr, ... }` (update). Both start with `{`; disambiguated
    /// by one identifier of lookahead then a peek for `=` vs `|`. An update's base
    /// must be a plain lowercase variable reference, matching Elm's own
    /// restriction (not an arbitrary expression).
    fn expr_record_or_update(&mut self) -> Result<Spanned<Expr>, ParseError> {
        let start = self.pos;
        self.bump(); // '{'
        self.skip_trivia()?;
        if self.peek() == Some(b'}') {
            self.bump();
            return Ok(Spanned::new(
                Span::new(start.offset, self.pos.offset),
                Expr::Record(Vec::new()),
            ));
        }
        let first_name = self.lower_ident()?;
        self.skip_trivia()?;
        match self.peek() {
            Some(b'|') => {
                self.bump();
                self.skip_trivia()?;
                let base = Spanned::new(first_name.span, Expr::Var(first_name.node));
                let fields = self.expr_record_field_list()?;
                self.expect_byte(b'}')?;
                let span = Span::new(start.offset, self.pos.offset);
                Ok(Spanned::new(
                    span,
                    Expr::RecordUpdate(Box::new(base), fields),
                ))
            }
            Some(b'=') => {
                self.bump();
                self.skip_trivia()?;
                let value = self.expr()?;
                self.skip_trivia()?;
                let mut fields = vec![(first_name.node, value)];
                while self.peek() == Some(b',') {
                    self.bump();
                    self.skip_trivia()?;
                    fields.push(self.expr_record_field()?);
                    self.skip_trivia()?;
                }
                self.expect_byte(b'}')?;
                let span = Span::new(start.offset, self.pos.offset);
                Ok(Spanned::new(span, Expr::Record(fields)))
            }
            _ => Err(ParseError::new(
                ErrorKind::Expected("`=` or `|`"),
                Span::new(self.pos.offset, self.pos.offset),
            )),
        }
    }

    /// `pub(crate)`: M6's `@{ key = value, ... }` annotation block is
    /// identical grammar to a record's field list, so `annotation.rs` reuses
    /// this directly rather than duplicating it.
    pub(crate) fn expr_record_field_list(
        &mut self,
    ) -> Result<Vec<(String, Spanned<Expr>)>, ParseError> {
        let mut fields = vec![self.expr_record_field()?];
        self.skip_trivia()?;
        while self.peek() == Some(b',') {
            self.bump();
            self.skip_trivia()?;
            fields.push(self.expr_record_field()?);
            self.skip_trivia()?;
        }
        Ok(fields)
    }

    fn expr_record_field(&mut self) -> Result<(String, Spanned<Expr>), ParseError> {
        let name = self.lower_ident()?.node;
        self.skip_trivia()?;
        self.expect_byte(b'=')?;
        self.skip_trivia()?;
        let value = self.expr()?;
        Ok((name, value))
    }

    fn expr_if(&mut self) -> Result<Spanned<Expr>, ParseError> {
        let start = self.pos;
        self.expect_keyword("if")?;
        self.skip_trivia()?;
        let cond = self.expr()?;
        self.skip_trivia()?;
        self.expect_keyword("then")?;
        self.skip_trivia()?;
        let then_branch = self.expr()?;
        self.skip_trivia()?;
        self.expect_keyword("else")?;
        self.skip_trivia()?;
        let else_branch = self.expr()?;
        let span = Span::new(start.offset, else_branch.span.end);
        Ok(Spanned::new(
            span,
            Expr::If(Box::new(cond), Box::new(then_branch), Box::new(else_branch)),
        ))
    }

    fn expr_let(&mut self) -> Result<Spanned<Expr>, ParseError> {
        let start = self.pos;
        self.expect_keyword("let")?;
        self.skip_trivia()?;
        let bindings = self.with_indent(|state| {
            let col = state.pos.col;
            let mut bindings = vec![state.let_binding()?];
            loop {
                let checkpoint = state.clone();
                state.skip_trivia()?;
                if state.check_aligned(col).is_err() {
                    *state = checkpoint;
                    break;
                }
                bindings.push(state.let_binding()?);
            }
            Ok(bindings)
        })?;
        self.skip_trivia()?;
        self.expect_keyword("in")?;
        self.skip_trivia()?;
        let body = self.expr()?;
        let span = Span::new(start.offset, body.span.end);
        Ok(Spanned::new(span, Expr::Let(bindings, Box::new(body))))
    }

    /// A `let`-bound value has no dedicated declaration struct to hold
    /// annotations the way `FnDef` does, so a leading `@` here (spec §10.1's
    /// stacked/block forms, above the binding just like a top-level
    /// signature) wraps the *value* in `Expr::Annotated` instead — the same
    /// representation prefix inline annotations already use, rather than
    /// inventing a second one.
    fn let_binding(
        &mut self,
    ) -> Result<(Spanned<crate::ast::pattern::Pattern>, Spanned<Expr>), ParseError> {
        self.skip_trivia()?;
        let start = self.pos;
        let annotations = if self.peek() == Some(b'@') {
            self.annotation_list()?
        } else {
            Vec::new()
        };
        self.skip_trivia()?;
        let pat = self.pattern()?;
        self.skip_trivia()?;
        self.expect_byte(b'=')?;
        self.skip_trivia()?;
        let value = self.expr()?;
        let value = if annotations.is_empty() {
            value
        } else {
            let span = Span::new(start.offset, value.span.end);
            Spanned::new(span, Expr::Annotated(annotations, Box::new(value)))
        };
        Ok((pat, value))
    }

    fn expr_case(&mut self) -> Result<Spanned<Expr>, ParseError> {
        let start = self.pos;
        self.expect_keyword("case")?;
        self.skip_trivia()?;
        let scrutinee = self.expr()?;
        self.skip_trivia()?;
        self.expect_keyword("of")?;
        self.skip_trivia()?;
        let arms = self.with_indent(|state| {
            let col = state.pos.col;
            let mut arms = vec![state.case_arm()?];
            loop {
                let checkpoint = state.clone();
                state.skip_trivia()?;
                if state.check_aligned(col).is_err() {
                    *state = checkpoint;
                    break;
                }
                arms.push(state.case_arm()?);
            }
            Ok(arms)
        })?;
        let end = arms
            .last()
            .expect("with_indent always produces >= 1 arm")
            .1
            .span
            .end;
        let span = Span::new(start.offset, end);
        Ok(Spanned::new(span, Expr::Case(Box::new(scrutinee), arms)))
    }

    fn case_arm(
        &mut self,
    ) -> Result<(Spanned<crate::ast::pattern::Pattern>, Spanned<Expr>), ParseError> {
        let pat = self.pattern()?;
        self.skip_trivia()?;
        self.expect_arrow()?;
        self.skip_trivia()?;
        let body = self.expr()?;
        Ok((pat, body))
    }

    fn expr_lambda(&mut self) -> Result<Spanned<Expr>, ParseError> {
        let start = self.pos;
        self.bump(); // '\'
        self.skip_trivia()?;
        let mut params = vec![self.pattern_atom()?];
        loop {
            self.skip_trivia()?;
            if self.peek() == Some(b'-') && self.peek_at(1) == Some(b'>') {
                break;
            }
            params.push(self.pattern_atom()?);
        }
        self.expect_arrow()?;
        self.skip_trivia()?;
        let body = self.expr()?;
        let span = Span::new(start.offset, body.span.end);
        Ok(Spanned::new(span, Expr::Lambda(params, Box::new(body))))
    }

    fn expr_do(&mut self) -> Result<Spanned<Expr>, ParseError> {
        let start = self.pos;
        self.expect_keyword("do")?;
        self.skip_trivia()?;
        let mut stmts = self.with_indent(|state| {
            let col = state.pos.col;
            let mut stmts = vec![state.do_stmt()?];
            loop {
                let checkpoint = state.clone();
                state.skip_trivia()?;
                if state.check_aligned(col).is_err() {
                    *state = checkpoint;
                    break;
                }
                stmts.push(state.do_stmt()?);
            }
            Ok(stmts)
        })?;
        let last = stmts.pop().expect("with_indent always produces >= 1 stmt");
        let final_expr = match last {
            DoStmt::Expr(e) => e,
            DoStmt::Bind(_, e) => {
                return Err(ParseError::new(
                    ErrorKind::Custom(
                        "a `do` block must end with a plain expression, not a `<-` bind"
                            .to_string(),
                    ),
                    e.span,
                ));
            }
        };
        let span = Span::new(start.offset, final_expr.span.end);
        Ok(Spanned::new(span, Expr::Do(stmts, Box::new(final_expr))))
    }

    fn do_stmt(&mut self) -> Result<DoStmt, ParseError> {
        let bind_attempt = self.try_parse(|state| {
            let pat = state.pattern()?;
            state.skip_trivia()?;
            if !(state.peek() == Some(b'<') && state.peek_at(1) == Some(b'-')) {
                return Err(ParseError::new(
                    ErrorKind::Expected("`<-`"),
                    Span::new(state.pos.offset, state.pos.offset),
                ));
            }
            state.bump();
            state.bump();
            state.skip_trivia()?;
            let value = state.expr()?;
            Ok(DoStmt::Bind(pat, value))
        });
        match bind_attempt {
            Ok(stmt) => Ok(stmt),
            Err(err) if err.fatal => Err(err),
            Err(_) => {
                let value = self.expr()?;
                Ok(DoStmt::Expr(value))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::pattern::{Pattern, PatternLiteral};

    fn ex(src: &str) -> Expr {
        let mut s = ParseState::new(src);
        let result = s.expr().unwrap();
        assert!(
            s.is_eof(),
            "leftover input: {:?}",
            s.text_since(s.pos.offset)
        );
        result.node
    }

    fn err(src: &str) -> ParseError {
        let mut s = ParseState::new(src);
        match s.expr() {
            Err(e) => e,
            Ok(v) if !s.is_eof() => {
                // Treat "parsed something but left input over" as a failure too,
                // for the fixtures where that's how invalidity actually shows up
                // (e.g. `.`/`$` aren't recognized tokens at all).
                ParseError::new(ErrorKind::Custom("leftover input".to_string()), v.span)
            }
            Ok(v) => panic!("expected a parse failure, got {v:?}"),
        }
    }

    #[test]
    fn int_and_float_literals() {
        assert_eq!(ex("42"), Expr::IntLit(42));
        // 2.5, not e.g. 3.14: exactly representable in binary (see lex::literal's
        // plain_float test for the same reasoning).
        assert_eq!(ex("2.5"), Expr::FloatLit(2.5));
    }

    #[test]
    fn bool_ctors() {
        assert_eq!(ex("True"), Expr::Ctor("True".to_string()));
        assert_eq!(ex("False"), Expr::Ctor("False".to_string()));
    }

    #[test]
    fn application() {
        let Expr::App(f, y) = ex("f x y") else {
            panic!("expected App")
        };
        let Expr::App(f2, x) = f.node else {
            panic!("expected nested App")
        };
        assert_eq!(f2.node, Expr::Var("f".to_string()));
        assert_eq!(x.node, Expr::Var("x".to_string()));
        assert_eq!(y.node, Expr::Var("y".to_string()));
    }

    #[test]
    fn field_access_chain() {
        let Expr::FieldAccess(inner, z) = ex("scene.camera.z") else {
            panic!("expected FieldAccess")
        };
        assert_eq!(z, "z");
        let Expr::FieldAccess(base, y) = inner.node else {
            panic!("expected nested FieldAccess")
        };
        assert_eq!(y, "camera");
        assert_eq!(base.node, Expr::Var("scene".to_string()));
    }

    #[test]
    fn qualified_name_is_var() {
        assert_eq!(ex("List.map"), Expr::Var("List.map".to_string()));
    }

    #[test]
    fn field_access_binds_tighter_than_application() {
        // "f point.x" is "f (point.x)", not "(f point).x"
        let Expr::App(f, arg) = ex("f point.x") else {
            panic!("expected App")
        };
        assert_eq!(f.node, Expr::Var("f".to_string()));
        assert!(matches!(arg.node, Expr::FieldAccess(_, _)));
    }

    #[test]
    fn negate_literal_argument() {
        let Expr::App(f, arg) = ex("f -1") else {
            panic!("expected App")
        };
        assert_eq!(f.node, Expr::Var("f".to_string()));
        assert_eq!(
            arg.node,
            Expr::Negate(Box::new(Spanned::new(Span::new(3, 4), Expr::IntLit(1))))
        );
    }

    #[test]
    fn negate_variable_argument() {
        let Expr::App(_, arg) = ex("f -y") else {
            panic!("expected App")
        };
        assert_eq!(
            arg.node,
            Expr::Negate(Box::new(Spanned::new(
                Span::new(3, 4),
                Expr::Var("y".to_string())
            )))
        );
    }

    #[test]
    fn subtraction_spaced() {
        assert_eq!(
            ex("a - 1"),
            Expr::BinOp(
                BinOp::Sub,
                Box::new(Spanned::new(Span::new(0, 1), Expr::Var("a".to_string()))),
                Box::new(Spanned::new(Span::new(4, 5), Expr::IntLit(1)))
            )
        );
    }

    #[test]
    fn subtraction_unspaced() {
        assert_eq!(
            ex("a-1"),
            Expr::BinOp(
                BinOp::Sub,
                Box::new(Spanned::new(Span::new(0, 1), Expr::Var("a".to_string()))),
                Box::new(Spanned::new(Span::new(2, 3), Expr::IntLit(1)))
            )
        );
    }

    #[test]
    fn ambiguous_minus_spacing_is_fatal_not_swallowed() {
        // "f- 1" -- must hard-fail, not be silently reinterpreted as `f` applied
        // to nothing (which is what a non-fatal treatment would produce).
        let mut s = ParseState::new("f- 1");
        let e = s.expr().unwrap_err();
        assert!(e.fatal);
    }

    #[test]
    fn negative_literal_immediately_inside_parens() {
        // "(-40.0)" -- the `-` sits at the very start of a fresh atom (right
        // after `(`), symmetric-spacing-wise indistinguishable from `a-b`'s
        // own `-`, but there is no left operand here for it to subtract
        // from. Used to be a hard "Expected an expression" parse error
        // (found via `corpus/programs/numeric/clamp-and-abs.knot`).
        let Expr::Negate(inner) = ex("(-40.0)") else {
            panic!("expected Negate")
        };
        assert_eq!(inner.node, Expr::FloatLit(40.0));
    }

    #[test]
    fn negative_literal_as_first_list_element() {
        // "[-5, -6]" -- same root cause as the parens case above, just after
        // `[` instead. The second element (after `, `, so genuinely
        // space-before/no-space-after) already worked before this fix.
        let Expr::List(elems) = ex("[-5, -6]") else {
            panic!("expected List")
        };
        assert_eq!(elems.len(), 2);
        assert!(matches!(elems[0].node, Expr::Negate(_)));
        assert!(matches!(elems[1].node, Expr::Negate(_)));
    }

    #[test]
    fn negative_literal_as_a_parenthesized_application_argument() {
        // "f (-5)" -- an application argument that's itself a parenthesized
        // negative literal; distinct from `negate_literal_argument`'s bare
        // `f -1` (no parens at all).
        let Expr::App(f, arg) = ex("f (-5)") else {
            panic!("expected App")
        };
        assert_eq!(f.node, Expr::Var("f".to_string()));
        assert!(matches!(arg.node, Expr::Negate(_)));
    }

    #[test]
    fn subtraction_still_wins_over_negation_for_a_real_left_operand() {
        // Fixing the parenthesized/bracketed cases above must not turn `f -
        // g` (a trailing application argument shaped like subtraction) back
        // into `f` applied to `(-g)` -- it's still subtraction, resolved by
        // `expr_app`'s own trailing-argument loop backing off before ever
        // calling into `expr_atom`.
        assert_eq!(
            ex("f - g"),
            Expr::BinOp(
                BinOp::Sub,
                Box::new(Spanned::new(Span::new(0, 1), Expr::Var("f".to_string()))),
                Box::new(Spanned::new(Span::new(4, 5), Expr::Var("g".to_string())))
            )
        );
    }

    #[test]
    fn precedence_mul_over_add() {
        let Expr::BinOp(BinOp::Add, _, rhs) = ex("1 + 2 * 3") else {
            panic!("expected top-level Add")
        };
        assert!(matches!(rhs.node, Expr::BinOp(BinOp::Mul, _, _)));
    }

    #[test]
    fn exponent_is_right_associative() {
        let Expr::BinOp(BinOp::Pow, _, rhs) = ex("2 ^ 3 ^ 2") else {
            panic!("expected top-level Pow")
        };
        assert!(matches!(rhs.node, Expr::BinOp(BinOp::Pow, _, _)));
    }

    #[test]
    fn cons_right_assoc_in_expression_position() {
        let Expr::BinOp(BinOp::Cons, _, rhs) = ex("1 : 2 : []") else {
            panic!("expected top-level Cons")
        };
        assert!(matches!(rhs.node, Expr::BinOp(BinOp::Cons, _, _)));
    }

    #[test]
    fn logical_precedence() {
        // (a && b) || c -- && (prec 3) binds tighter than || (prec 2)
        let Expr::BinOp(BinOp::Or, lhs, _) = ex("a && b || c") else {
            panic!("expected top-level Or")
        };
        assert!(matches!(lhs.node, Expr::BinOp(BinOp::And, _, _)));
    }

    #[test]
    fn pipe_and_compose_chain_left_associatively() {
        let Expr::BinOp(BinOp::Pipe, lhs, _) = ex("a |> f |> g") else {
            panic!("expected top-level Pipe")
        };
        assert!(matches!(lhs.node, Expr::BinOp(BinOp::Pipe, _, _)));
    }

    #[test]
    fn comparison_does_not_chain() {
        assert!(err("a == b == c").fatal);
    }

    #[test]
    fn mixed_comparisons_at_same_precedence_also_rejected() {
        assert!(err("a == b < c").fatal);
    }

    #[test]
    fn double_colon_is_not_cons_in_expressions_either() {
        // leftover "::" input makes this fail via the `ex`/`err` harness's
        // is_eof check, same mechanism as the corpus's cons-double-colon fixture
        let mut s = ParseState::new("1 :: []");
        let result = s.expr().unwrap();
        assert_eq!(result.node, Expr::IntLit(1));
        assert!(!s.is_eof());
    }

    #[test]
    fn dot_composition_is_gone() {
        // "f . g" -- `.` is never a recognized standalone token, so this leaves
        // " . g" unconsumed after parsing just `f`.
        let mut s = ParseState::new("f . g");
        let result = s.expr().unwrap();
        assert_eq!(result.node, Expr::Var("f".to_string()));
        assert!(!s.is_eof());
    }

    #[test]
    fn dollar_is_not_supported() {
        let mut s = ParseState::new("f $ y");
        let result = s.expr().unwrap();
        assert_eq!(result.node, Expr::Var("f".to_string()));
        assert!(!s.is_eof());
    }

    #[test]
    fn if_then_else() {
        let Expr::If(cond, then_b, else_b) = ex("if x < 0 then negate x else x") else {
            panic!("expected If")
        };
        assert!(matches!(cond.node, Expr::BinOp(BinOp::Lt, _, _)));
        assert!(matches!(then_b.node, Expr::App(_, _)));
        assert_eq!(else_b.node, Expr::Var("x".to_string()));
    }

    #[test]
    fn let_multi_binding() {
        let src = "let\n  radius = 5.0\n  pi = 3.14159\nin pi * radius * radius";
        let Expr::Let(bindings, body) = ex(src) else {
            panic!("expected Let")
        };
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].0.node, Pattern::Var("radius".to_string()));
        assert_eq!(bindings[1].0.node, Pattern::Var("pi".to_string()));
        assert!(matches!(body.node, Expr::BinOp(BinOp::Mul, _, _)));
    }

    #[test]
    fn let_nested() {
        let src =
            "let\n  outer =\n    let\n      inner = 10\n    in\n      inner + 1\nin outer * 2";
        let Expr::Let(bindings, _) = ex(src) else {
            panic!("expected outer Let")
        };
        assert_eq!(bindings.len(), 1);
        assert!(matches!(bindings[0].1.node, Expr::Let(_, _)));
    }

    #[test]
    fn let_does_not_swallow_next_binding_as_application_argument() {
        // The critical layout-correctness case: without the `continues_layout`
        // check, `expr_app` would happily try to treat "pi" as another argument
        // applied to `5.0`, since nothing about `5.0`'s *type* rules that out
        // syntactically -- only its column (not indented past the block's own
        // reference column) does.
        let src = "let\n  radius = 5.0\n  pi = 3.14159\nin radius";
        let Expr::Let(bindings, _) = ex(src) else {
            panic!("expected Let")
        };
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].1.node, Expr::FloatLit(5.0));
    }

    #[test]
    fn case_multi_arm() {
        let src = "case shape of\n  Circle r -> r\n  Rectangle w h -> w";
        let Expr::Case(scrutinee, arms) = ex(src) else {
            panic!("expected Case")
        };
        assert_eq!(scrutinee.node, Expr::Var("shape".to_string()));
        assert_eq!(arms.len(), 2);
        assert!(matches!(arms[0].0.node, Pattern::Ctor(_, _)));
    }

    #[test]
    fn lambda_single_arg() {
        let Expr::Lambda(params, body) = ex(r"\x -> x + 1") else {
            panic!("expected Lambda")
        };
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].node, Pattern::Var("x".to_string()));
        assert!(matches!(body.node, Expr::BinOp(BinOp::Add, _, _)));
    }

    #[test]
    fn lambda_multi_arg() {
        let Expr::Lambda(params, _) = ex(r"\x y -> x + y") else {
            panic!("expected Lambda")
        };
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn do_block_with_bind_and_final_expr() {
        let src = "do\n  x <- someOption\n  y <- anotherOption\n  pure (x + y)";
        let Expr::Do(stmts, final_expr) = ex(src) else {
            panic!("expected Do")
        };
        assert_eq!(stmts.len(), 2);
        assert!(matches!(stmts[0], DoStmt::Bind(_, _)));
        assert!(matches!(final_expr.node, Expr::App(_, _)));
    }

    #[test]
    fn do_block_must_end_with_plain_expression() {
        let src = "do\n  x <- someOption\n  y <- anotherOption";
        let mut s = ParseState::new(src);
        assert!(s.expr().is_err());
    }

    #[test]
    fn record_construct() {
        let Expr::Record(fields) = ex("{ x = 0.0, y = 0.0 }") else {
            panic!("expected Record")
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0, "x");
    }

    #[test]
    fn empty_record() {
        assert_eq!(ex("{}"), Expr::Record(Vec::new()));
    }

    #[test]
    fn record_update() {
        let Expr::RecordUpdate(base, fields) = ex("{ point | x = 1.0 }") else {
            panic!("expected RecordUpdate")
        };
        assert_eq!(base.node, Expr::Var("point".to_string()));
        assert_eq!(fields.len(), 1);
    }

    #[test]
    fn list_literal() {
        assert_eq!(
            ex("[2, 3, 5]"),
            Expr::List(vec![
                Spanned::new(Span::new(1, 2), Expr::IntLit(2)),
                Spanned::new(Span::new(4, 5), Expr::IntLit(3)),
                Spanned::new(Span::new(7, 8), Expr::IntLit(5)),
            ])
        );
    }

    #[test]
    fn empty_list() {
        assert_eq!(ex("[]"), Expr::List(Vec::new()));
    }

    #[test]
    fn tuple_and_unit_and_grouping() {
        assert_eq!(ex("()"), Expr::Unit);
        assert_eq!(ex("(1)"), Expr::IntLit(1));
        assert_eq!(
            ex("(1, \"a\")"),
            Expr::Tuple(vec![
                Spanned::new(Span::new(1, 2), Expr::IntLit(1)),
                Spanned::new(Span::new(4, 7), Expr::StringLit("a".to_string())),
            ])
        );
    }

    #[test]
    fn named_hole_in_expression_position_is_fatal() {
        let mut s = ParseState::new("zipWith _f _xs _ys");
        let e = s.expr().unwrap_err();
        assert!(e.fatal);
    }

    #[test]
    fn anonymous_hole_in_expression_position_is_fine() {
        // Left-associative: App(App(App(f, a), _), c)
        let Expr::App(inner1, c) = ex("f a _ c") else {
            panic!("expected App")
        };
        let Expr::App(inner2, hole) = inner1.node else {
            panic!("expected nested App")
        };
        let Expr::App(f, a) = inner2.node else {
            panic!("expected doubly-nested App")
        };
        assert_eq!(f.node, Expr::Var("f".to_string()));
        assert_eq!(a.node, Expr::Var("a".to_string()));
        assert_eq!(hole.node, Expr::Hole);
        assert_eq!(c.node, Expr::Var("c".to_string()));
    }

    #[test]
    fn map_fromlist_style_qualified_application() {
        let Expr::App(f, arg) = ex("Map.fromList xs") else {
            panic!("expected App")
        };
        assert_eq!(f.node, Expr::Var("Map.fromList".to_string()));
        assert_eq!(arg.node, Expr::Var("xs".to_string()));
    }

    #[test]
    fn negative_pattern_literal_still_parses_via_pattern_grammar() {
        // sanity check that pattern parsing (used by case arms) is reachable and
        // unaffected by expr-level minus classification
        let mut s = ParseState::new("-1");
        let p = s.pattern().unwrap();
        assert_eq!(p.node, Pattern::Literal(PatternLiteral::Int(-1)));
    }

    #[test]
    fn prefix_annotation_binds_to_the_immediately_following_atom() {
        // "@nodeId("n1") f y" -- the annotation attaches to `f`, the closest
        // *following* atom, not to the whole `f y` application.
        let Expr::App(annotated_f, y) = ex(r#"@nodeId("n1") f y"#) else {
            panic!("expected App")
        };
        assert_eq!(y.node, Expr::Var("y".to_string()));
        let Expr::Annotated(anns, target) = &annotated_f.node else {
            panic!("expected Annotated")
        };
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].key, "nodeId");
        assert_eq!(target.node, Expr::Var("f".to_string()));
    }

    #[test]
    fn prefix_annotation_on_an_argument() {
        // "f @nodeId("n1") y" -- annotates `y`, not the whole `f y`.
        let Expr::App(f, annotated_y) = ex(r#"f @nodeId("n1") y"#) else {
            panic!("expected App")
        };
        assert_eq!(f.node, Expr::Var("f".to_string()));
        let Expr::Annotated(_, target) = &annotated_y.node else {
            panic!("expected Annotated")
        };
        assert_eq!(target.node, Expr::Var("y".to_string()));
    }

    #[test]
    fn parens_widen_annotation_scope_to_a_whole_application() {
        let Expr::Annotated(_, target) = ex(r#"@{ nodeId = "n1" } (f a b)"#) else {
            panic!("expected Annotated")
        };
        assert!(matches!(target.node, Expr::App(_, _)));
    }

    #[test]
    fn annotation_desugars_single_and_multi_arg_the_same_as_block_form() {
        let single = ex(r#"@label("My Function") f"#);
        let Expr::Annotated(anns, _) = &single else {
            panic!("expected Annotated")
        };
        assert_eq!(
            anns[0].value.node,
            Expr::StringLit("My Function".to_string())
        );

        let multi = ex("@position(100, 200) f");
        let Expr::Annotated(anns, _) = &multi else {
            panic!("expected Annotated")
        };
        assert!(matches!(anns[0].value.node, Expr::Tuple(_)));
    }

    #[test]
    fn hole_cannot_carry_an_annotation() {
        let mut s = ParseState::new(r#"@nodeId("x") _"#);
        let e = s.expr().unwrap_err();
        assert!(e.fatal);
    }

    #[test]
    fn let_binding_annotation_wraps_the_value_in_annotated() {
        let src = "let\n  @nodeId(\"n1\")\n  @position(150, 300)\n  result = myFunc 42\nin result";
        let Expr::Let(bindings, _) = ex(src) else {
            panic!("expected Let")
        };
        assert_eq!(bindings.len(), 1);
        let Expr::Annotated(anns, target) = &bindings[0].1.node else {
            panic!("expected Annotated value")
        };
        assert_eq!(anns.len(), 2);
        assert!(matches!(target.node, Expr::App(_, _)));
    }

    #[test]
    fn let_binding_without_annotation_is_unaffected() {
        let src = "let\n  radius = 5.0\nin radius";
        let Expr::Let(bindings, _) = ex(src) else {
            panic!("expected Let")
        };
        assert_eq!(bindings[0].1.node, Expr::FloatLit(5.0));
    }
}
