//! Declarations: signature+def merging into `FnDef`, `type`/`type alias`,
//! `instance` declarations, and the top-level declaration-list layout block
//! shared by both a bare fragment (most of the test corpus) and a full module
//! (which wraps this with a header and imports — see `module.rs`).

use crate::ast::decl::{Decl, FnDef, InstanceDecl};
use crate::ast::ty::Type;
use crate::error::{ErrorKind, ParseError};
use crate::span::{Span, Spanned};
use crate::state::ParseState;

/// Mirrors `Decl::TypeDecl`'s own field shape: name, type params, variants
/// (each a constructor name plus its argument types), and derived interfaces.
type TypeDeclParts = (String, Vec<String>, Vec<(String, Vec<Type>)>, Vec<String>);

impl<'a> ParseState<'a> {
    /// A declaration list with no module header/imports required — most of the
    /// test corpus is fragments at this level (M2–M4 grammar layers were tested
    /// before the module system existed), and `parse_module` builds on top of
    /// this for the real thing.
    pub fn parse_decls(&mut self) -> Result<Vec<Spanned<Decl>>, ParseError> {
        self.skip_trivia()?;
        if self.is_eof() {
            return Ok(Vec::new());
        }
        self.with_indent(|state| {
            let col = state.pos.col;
            let mut decls = vec![state.top_level_decl(col)?];
            loop {
                let checkpoint = state.clone();
                state.skip_trivia()?;
                if state.is_eof() {
                    break;
                }
                if state.check_aligned(col).is_err() {
                    *state = checkpoint;
                    break;
                }
                decls.push(state.top_level_decl(col)?);
            }
            Ok(decls)
        })
    }

    /// A leading `@name(args)`/`@{...}` (spec §13.1) attaches to `FnDef`'s own
    /// `annotations` field. `type`/`type alias`/`instance` declarations don't
    /// carry annotations in v0 (a resolved open question — possible V2
    /// feature), so one appearing there is a clear error rather than being
    /// silently accepted and dropped.
    pub(crate) fn top_level_decl(&mut self, col: u32) -> Result<Spanned<Decl>, ParseError> {
        let start = self.pos;
        self.skip_trivia()?;
        let annotations = if self.peek() == Some(b'@') {
            self.annotation_list()?
        } else {
            Vec::new()
        };
        self.skip_trivia()?;
        let decl = if self.peek_keyword("type") {
            self.type_or_alias_decl()?
        } else if self.peek_keyword("instance") {
            Decl::Instance(self.instance_decl()?)
        } else {
            Decl::Fn(self.fn_decl(col)?)
        };
        let decl = match decl {
            Decl::Fn(mut f) => {
                f.annotations = annotations;
                Decl::Fn(f)
            }
            other if annotations.is_empty() => other,
            _ => {
                return Err(ParseError::new(
                    ErrorKind::Custom(
                        "annotations are not supported on type/instance declarations in v0"
                            .to_string(),
                    ),
                    Span::new(start.offset, start.offset),
                )
                .fatal());
            }
        };
        Ok(Spanned::new(Span::new(start.offset, self.pos.offset), decl))
    }

    /// A signature (`name :: Type`) and its definition (`name arg = expr`) are
    /// two source lines but one `FnDef` — after an optional signature, this
    /// requires the *next* aligned item to share the same name and treats it as
    /// the definition, rather than leaving the outer declaration-list loop to
    /// (incorrectly) treat it as an unrelated sibling declaration.
    pub(crate) fn fn_decl(&mut self, col: u32) -> Result<FnDef, ParseError> {
        let name = self.decl_name()?.node;
        self.skip_trivia()?;

        let signature = if self.peek() == Some(b':') && self.peek_at(1) == Some(b':') {
            let sig_start = self.pos;
            self.bump();
            self.bump();
            self.skip_trivia()?;
            let sig = self.type_signature()?;
            let sig_span = Span::new(sig_start.offset, self.pos.offset);
            self.skip_trivia()?;
            self.check_aligned(col)?;
            let def_name = self.decl_name()?.node;
            if def_name != name {
                return Err(ParseError::new(
                    ErrorKind::Custom(format!(
                        "expected a definition for `{name}` after its signature, found `{def_name}`"
                    )),
                    Span::new(self.pos.offset, self.pos.offset),
                ));
            }
            Some(Spanned::new(sig_span, sig))
        } else {
            None
        };

        let mut params = Vec::new();
        loop {
            let checkpoint = self.clone();
            self.skip_trivia()?;
            if self.peek() == Some(b'=') || !self.continues_layout() {
                *self = checkpoint;
                break;
            }
            match self.pattern_atom() {
                Ok(p) => params.push(p),
                Err(err) if err.fatal => return Err(err),
                Err(_) => {
                    *self = checkpoint;
                    break;
                }
            }
        }
        self.expect_byte(b'=')?;
        self.skip_trivia()?;
        let body = self.expr()?;
        Ok(FnDef {
            name,
            signature,
            params,
            body,
            annotations: Vec::new(),
        })
    }

    /// A declaration's name: an ordinary `lower_ident`, or a closed-set operator
    /// symbol wrapped in parens (`(==)`, `(+)`) — needed because interface
    /// methods are named after the operation they implement, e.g. `Eq`'s
    /// `(==)`. The parser doesn't check the symbol against the actual closed
    /// operator table here (that's one grammar production's job, not a
    /// semantic-adjacent one); it just recognizes the shape.
    fn decl_name(&mut self) -> Result<Spanned<String>, ParseError> {
        self.skip_trivia()?;
        if self.peek() != Some(b'(') {
            return self.lower_ident();
        }
        let start = self.pos;
        self.bump(); // '('
        let sym_start = self.pos;
        while matches!(
            self.peek(),
            Some(b'+' | b'-' | b'*' | b'/' | b'^' | b'=' | b'<' | b'>' | b'&' | b'|' | b':')
        ) {
            self.bump();
        }
        if self.pos.offset == sym_start.offset {
            return Err(ParseError::new(
                ErrorKind::Expected("an operator symbol"),
                Span::new(self.pos.offset, self.pos.offset),
            ));
        }
        let symbol = self.text_since(sym_start.offset).to_string();
        self.expect_byte(b')')?;
        Ok(Spanned::new(
            Span::new(start.offset, self.pos.offset),
            symbol,
        ))
    }

    fn type_or_alias_decl(&mut self) -> Result<Decl, ParseError> {
        let checkpoint = self.clone();
        self.expect_keyword("type")?;
        self.skip_trivia()?;
        let is_alias = self.peek_keyword("alias");
        *self = checkpoint;
        if is_alias {
            let (name, params, ty) = self.type_alias_decl()?;
            Ok(Decl::TypeAlias(name, params, ty))
        } else {
            let (name, params, variants, deriving) = self.type_decl()?;
            Ok(Decl::TypeDecl(name, params, variants, deriving))
        }
    }

    fn type_params(&mut self) -> Result<Vec<String>, ParseError> {
        let mut params = Vec::new();
        loop {
            self.skip_trivia()?;
            if !matches!(self.peek(), Some(b) if b.is_ascii_lowercase()) {
                break;
            }
            params.push(self.lower_ident()?.node);
        }
        Ok(params)
    }

    fn type_alias_decl(&mut self) -> Result<(String, Vec<String>, Type), ParseError> {
        self.expect_keyword("type")?;
        self.skip_trivia()?;
        self.expect_keyword("alias")?;
        self.skip_trivia()?;
        let name = self.upper_ident_segment()?.node;
        let params = self.type_params()?;
        self.expect_byte(b'=')?;
        self.skip_trivia()?;
        let ty = self.type_arrow()?;
        Ok((name, params, ty))
    }

    fn type_decl(&mut self) -> Result<TypeDeclParts, ParseError> {
        self.expect_keyword("type")?;
        self.skip_trivia()?;
        let name = self.upper_ident_segment()?.node;
        let params = self.type_params()?;
        self.expect_byte(b'=')?;
        self.skip_trivia()?;
        let mut variants = vec![self.type_variant()?];
        loop {
            let checkpoint = self.clone();
            self.skip_trivia()?;
            if self.peek() != Some(b'|') {
                *self = checkpoint;
                break;
            }
            self.bump();
            self.skip_trivia()?;
            variants.push(self.type_variant()?);
        }
        let checkpoint = self.clone();
        self.skip_trivia()?;
        let deriving = if self.peek_keyword("deriving") {
            self.deriving_clause()?
        } else {
            *self = checkpoint;
            Vec::new()
        };
        Ok((name, params, variants, deriving))
    }

    /// `deriving (Eq, Ord, Show)` or the single-name form `deriving Eq`, with
    /// the `deriving` keyword itself not yet consumed. Which interfaces can
    /// actually be derived (and whether a given ADT's own shape qualifies)
    /// is a semantic question for `knot-checker`, not this layer -- this
    /// only ever collects whatever uppercase names appear here, exactly
    /// like `single_constraint`'s own interface name doesn't get checked
    /// against the closed set until canonicalization either.
    fn deriving_clause(&mut self) -> Result<Vec<String>, ParseError> {
        self.expect_keyword("deriving")?;
        self.skip_trivia()?;
        if self.peek() == Some(b'(') {
            self.bump();
            self.skip_trivia()?;
            let mut names = vec![self.upper_ident_segment()?.node];
            self.skip_trivia()?;
            while self.peek() == Some(b',') {
                self.bump();
                self.skip_trivia()?;
                names.push(self.upper_ident_segment()?.node);
                self.skip_trivia()?;
            }
            self.expect_byte(b')')?;
            Ok(names)
        } else {
            Ok(vec![self.upper_ident_segment()?.node])
        }
    }

    fn type_variant(&mut self) -> Result<(String, Vec<Type>), ParseError> {
        let ctor = self.upper_ident_segment()?.node;
        let mut args = Vec::new();
        loop {
            let checkpoint = self.clone();
            self.skip_trivia()?;
            if !self.continues_layout() {
                *self = checkpoint;
                break;
            }
            match self.type_atom() {
                Ok(ty) => args.push(ty),
                Err(err) if err.fatal => return Err(err),
                Err(_) => {
                    *self = checkpoint;
                    break;
                }
            }
        }
        Ok((ctor, args))
    }

    /// Haskell-style: `instance Eq Shape where (==) a b = ...`. Methods reuse
    /// `fn_decl` directly — since an instance method never repeats its own `::`
    /// signature (it's inherited from the interface), `fn_decl` naturally just
    /// takes the "no signature" branch for each one.
    fn instance_decl(&mut self) -> Result<InstanceDecl, ParseError> {
        self.expect_keyword("instance")?;
        self.skip_trivia()?;
        let constraints = self
            .try_parse(Self::constraint_list_arrow)
            .unwrap_or_default();
        let interface = self.upper_ident_segment()?.node;
        self.skip_trivia()?;
        let target = self.type_atom()?;
        self.skip_trivia()?;
        self.expect_keyword("where")?;
        self.skip_trivia()?;
        let methods = self.with_indent(|state| {
            let col = state.pos.col;
            let mut methods = vec![state.fn_decl(col)?];
            loop {
                let checkpoint = state.clone();
                state.skip_trivia()?;
                if state.check_aligned(col).is_err() {
                    *state = checkpoint;
                    break;
                }
                methods.push(state.fn_decl(col)?);
            }
            Ok(methods)
        })?;
        Ok(InstanceDecl {
            interface,
            constraints,
            target,
            methods,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::expr::Expr;
    use crate::ast::pattern::Pattern;
    use crate::ast::ty::Type;

    fn decls(src: &str) -> Vec<Decl> {
        let mut s = ParseState::new(src);
        let result = s.parse_decls().unwrap();
        assert!(
            s.is_eof(),
            "leftover input: {:?}",
            s.text_since(s.pos.offset)
        );
        result.into_iter().map(|d| d.node).collect()
    }

    #[test]
    fn fn_def_without_signature() {
        let ds = decls("myFunc x = x + 1");
        let [Decl::Fn(f)] = ds.as_slice() else {
            panic!("expected one Fn decl")
        };
        assert_eq!(f.name, "myFunc");
        assert!(f.signature.is_none());
        assert_eq!(f.params.len(), 1);
        assert!(matches!(f.body.node, Expr::BinOp(_, _, _)));
    }

    #[test]
    fn fn_def_with_signature_merges_into_one_decl() {
        let ds = decls("myFunc :: Int -> Int\nmyFunc x = x + 1");
        assert_eq!(ds.len(), 1);
        let [Decl::Fn(f)] = ds.as_slice() else {
            panic!("expected one merged Fn decl")
        };
        assert_eq!(f.name, "myFunc");
        assert!(f.signature.is_some());
        assert_eq!(f.params.len(), 1);
    }

    #[test]
    fn fn_def_with_constrained_signature() {
        let ds = decls("myMax :: Ord a => a -> a -> a\nmyMax x y = if x > y then x else y");
        let [Decl::Fn(f)] = ds.as_slice() else {
            panic!("expected one Fn decl")
        };
        let sig = f.signature.as_ref().unwrap();
        assert_eq!(sig.node.constraints.len(), 1);
    }

    #[test]
    fn signature_without_matching_definition_errors() {
        let mut s = ParseState::new("myFunc :: Int -> Int\nother x = x");
        assert!(s.parse_decls().is_err());
    }

    #[test]
    fn multiple_top_level_decls() {
        let ds = decls("x = 1\ny = 2");
        assert_eq!(ds.len(), 2);
    }

    #[test]
    fn fn_params_do_not_swallow_the_next_sibling_declaration() {
        // Regression guard for exactly the bug `continues_layout` prevents in
        // `fn_decl`'s params loop: without it, a malformed/missing `=` could
        // greedily consume the next declaration's name as a parameter.
        let mut s = ParseState::new("x\ny = 2");
        assert!(s.parse_decls().is_err());
    }

    #[test]
    fn type_alias_declaration() {
        let ds = decls("type alias Point = { x : Float, y : Float }");
        let [Decl::TypeAlias(name, params, ty)] = ds.as_slice() else {
            panic!("expected TypeAlias")
        };
        assert_eq!(name, "Point");
        assert!(params.is_empty());
        assert!(matches!(ty, Type::Record(_, _, None)));
    }

    #[test]
    fn adt_multi_variant_declaration() {
        let src = "type Shape\n  = Circle Float\n  | Rectangle Float Float\n  | Triangle Float Float Float";
        let ds = decls(src);
        let [Decl::TypeDecl(name, params, variants, _deriving)] = ds.as_slice() else {
            panic!("expected TypeDecl")
        };
        assert_eq!(name, "Shape");
        assert!(params.is_empty());
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0].0, "Circle");
        assert_eq!(variants[0].1.len(), 1);
        assert_eq!(variants[2].1.len(), 3);
    }

    #[test]
    fn a_user_type_can_reuse_true_and_false_as_its_own_constructor_names() {
        // True/False are prelude constructors, not reserved words -- removed
        // from RESERVED_WORDS since they were dead entries there anyway
        // (upper_ident_segment, which parses constructor names, never
        // consulted that lowercase-only list in the first place).
        let ds = decls("type Toggle\n  = True\n  | False");
        let [Decl::TypeDecl(name, params, variants, _deriving)] = ds.as_slice() else {
            panic!("expected TypeDecl")
        };
        assert_eq!(name, "Toggle");
        assert!(params.is_empty());
        assert_eq!(variants[0].0, "True");
        assert_eq!(variants[1].0, "False");
    }

    #[test]
    fn adt_with_no_deriving_clause_has_an_empty_list() {
        let ds = decls("type Shape\n  = Circle Float");
        let [Decl::TypeDecl(_, _, _, deriving)] = ds.as_slice() else {
            panic!("expected TypeDecl")
        };
        assert!(deriving.is_empty());
    }

    #[test]
    fn adt_deriving_a_single_interface_needs_no_parens() {
        let ds = decls("type Shape\n  = Circle Float\n  deriving Eq");
        let [Decl::TypeDecl(_, _, _, deriving)] = ds.as_slice() else {
            panic!("expected TypeDecl")
        };
        assert_eq!(deriving, &vec!["Eq".to_string()]);
    }

    #[test]
    fn adt_deriving_multiple_interfaces_needs_parens() {
        let ds = decls("type Shape\n  = Circle Float\n  deriving (Eq, Ord, Show)");
        let [Decl::TypeDecl(_, _, _, deriving)] = ds.as_slice() else {
            panic!("expected TypeDecl")
        };
        assert_eq!(
            deriving,
            &vec!["Eq".to_string(), "Ord".to_string(), "Show".to_string()]
        );
    }

    #[test]
    fn generic_adt_declaration() {
        let ds = decls("type Option a\n  = Some a\n  | None");
        let [Decl::TypeDecl(name, params, variants, _deriving)] = ds.as_slice() else {
            panic!("expected TypeDecl")
        };
        assert_eq!(name, "Option");
        assert_eq!(params, &vec!["a".to_string()]);
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[1].1.len(), 0);
    }

    #[test]
    fn instance_declaration() {
        let src = "instance Eq Shape where\n  (==) a b = True";
        let ds = decls(src);
        let [Decl::Instance(inst)] = ds.as_slice() else {
            panic!("expected Instance")
        };
        assert_eq!(inst.interface, "Eq");
        assert!(inst.constraints.is_empty());
        assert_eq!(inst.target, Type::Named("Shape".to_string(), Vec::new()));
        assert_eq!(inst.methods.len(), 1);
        assert_eq!(inst.methods[0].name, "==");
    }

    #[test]
    fn instance_declaration_with_constraint_and_parenthesized_target() {
        let src = "instance Eq a => Eq (Maybe a) where\n  (==) a b = True";
        let ds = decls(src);
        let [Decl::Instance(inst)] = ds.as_slice() else {
            panic!("expected Instance")
        };
        assert_eq!(inst.constraints.len(), 1);
        assert_eq!(
            inst.target,
            Type::Named("Maybe".to_string(), vec![Type::Var("a".to_string())])
        );
    }

    #[test]
    fn instance_method_reuses_pattern_grammar_for_params() {
        let src = "instance Eq Shape where\n  (==) a b =\n    case (a, b) of\n      (x, y) -> True";
        let ds = decls(src);
        let [Decl::Instance(inst)] = ds.as_slice() else {
            panic!("expected Instance")
        };
        assert!(matches!(inst.methods[0].params[0].node, Pattern::Var(_)));
        assert!(matches!(inst.methods[0].body.node, Expr::Case(_, _)));
    }

    #[test]
    fn stacked_annotations_attach_to_fn_def() {
        let src = "@nodeId(\"f1\")\n@position(100, 200)\nmyFunc :: Int -> Int\nmyFunc x = x + 1";
        let ds = decls(src);
        let [Decl::Fn(f)] = ds.as_slice() else {
            panic!("expected one Fn decl")
        };
        assert_eq!(f.name, "myFunc");
        assert_eq!(f.annotations.len(), 2);
        assert_eq!(f.annotations[0].key, "nodeId");
        assert!(f.signature.is_some());
    }

    #[test]
    fn block_annotation_attaches_to_fn_def_without_signature() {
        let src = "@{ nodeId = \"f1\", color = \"blue\" }\nmyFunc x = x + 1";
        let ds = decls(src);
        let [Decl::Fn(f)] = ds.as_slice() else {
            panic!("expected one Fn decl")
        };
        assert_eq!(f.annotations.len(), 2);
    }

    #[test]
    fn annotation_on_type_alias_is_rejected() {
        let src = "@doc(\"a point\")\ntype alias Point = { x : Float, y : Float }";
        let mut s = ParseState::new(src);
        assert!(s.parse_decls().is_err());
    }

    #[test]
    fn annotation_on_instance_is_rejected() {
        let src = "@doc(\"eq for shapes\")\ninstance Eq Shape where\n  (==) a b = True";
        let mut s = ParseState::new(src);
        assert!(s.parse_decls().is_err());
    }

    #[test]
    fn multiple_annotated_declarations_in_sequence() {
        // Regression guard: one declaration's annotation list must not bleed
        // into the next sibling declaration's.
        let src = "@nodeId(\"f1\")\nfirst x = x\n\n@nodeId(\"f2\")\nsecond y = y";
        let ds = decls(src);
        assert_eq!(ds.len(), 2);
        let [Decl::Fn(a), Decl::Fn(b)] = ds.as_slice() else {
            panic!("expected two Fn decls")
        };
        assert_eq!(a.annotations[0].key, "nodeId");
        assert_eq!(b.annotations[0].key, "nodeId");
        assert_ne!(a.annotations[0].value.node, b.annotations[0].value.node);
    }
}
