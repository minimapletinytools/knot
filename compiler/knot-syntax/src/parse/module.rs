//! Module header, imports, and `exposing`. Wraps `parse_decls` (decl.rs) with
//! the pieces that only a full module needs; the bare declaration list is kept
//! reusable on its own since most of the test corpus is fragments below the
//! module level (see decl.rs's doc comment).

use crate::ast::decl::{ExposedItem, Exposing, Import, Module};
use crate::error::{ErrorKind, ParseError};
use crate::span::Span;
use crate::state::ParseState;

impl<'a> ParseState<'a> {
    /// A full module: header, imports, then declarations. Bails on the first
    /// error and on any trailing content left after the last declaration,
    /// matching Elm's own v0 behavior rather than attempting multi-error
    /// recovery.
    pub fn parse_module(&mut self) -> Result<Module, ParseError> {
        self.skip_trivia()?;
        let (name, exposing) = self.module_header()?;
        self.skip_trivia()?;
        let mut imports = Vec::new();
        loop {
            let checkpoint = self.clone();
            self.skip_trivia()?;
            if self.peek_keyword("import") {
                imports.push(self.import_decl()?);
            } else {
                *self = checkpoint;
                break;
            }
        }
        let decls = self.parse_decls()?;
        self.skip_trivia()?;
        if !self.is_eof() {
            return Err(ParseError::new(
                ErrorKind::Custom(
                    "unexpected trailing content after the last declaration".to_string(),
                ),
                Span::new(self.pos.offset, self.pos.offset),
            ));
        }
        Ok(Module {
            name,
            exposing,
            imports,
            decls,
        })
    }

    fn module_header(&mut self) -> Result<(Vec<String>, Exposing), ParseError> {
        self.expect_keyword("module")?;
        self.skip_trivia()?;
        let name = self.dotted_upper_path()?;
        self.skip_trivia()?;
        self.expect_keyword("exposing")?;
        self.skip_trivia()?;
        let exposing = self.exposing_list()?;
        Ok((name, exposing))
    }

    fn import_decl(&mut self) -> Result<Import, ParseError> {
        self.expect_keyword("import")?;
        self.skip_trivia()?;
        let module = self.dotted_upper_path()?;
        self.skip_trivia()?;
        let alias = if self.peek_keyword("as") {
            self.expect_keyword("as")?;
            self.skip_trivia()?;
            Some(self.upper_ident_segment()?.node)
        } else {
            None
        };
        self.skip_trivia()?;
        let exposing = if self.peek_keyword("exposing") {
            self.expect_keyword("exposing")?;
            self.skip_trivia()?;
            Some(self.exposing_list()?)
        } else {
            None
        };
        Ok(Import {
            module,
            alias,
            exposing,
        })
    }

    /// A dot-separated all-uppercase path (`Geometry`, `Geometry.Shapes`). Not
    /// the same helper as `qualified_upper_start` (used for *value* references
    /// like `List.map`) — a module path never ends in a lowercase segment.
    fn dotted_upper_path(&mut self) -> Result<Vec<String>, ParseError> {
        let mut segments = vec![self.upper_ident_segment()?.node];
        while self.peek() == Some(b'.')
            && matches!(self.peek_at(1), Some(b) if b.is_ascii_uppercase())
        {
            self.bump();
            segments.push(self.upper_ident_segment()?.node);
        }
        Ok(segments)
    }

    fn exposing_list(&mut self) -> Result<Exposing, ParseError> {
        self.expect_byte(b'(')?;
        self.skip_trivia()?;
        if self.peek() == Some(b'.') && self.peek_at(1) == Some(b'.') {
            self.expect_dotdot()?;
            self.skip_trivia()?;
            self.expect_byte(b')')?;
            return Ok(Exposing::All);
        }
        let mut items = vec![self.exposed_item()?];
        self.skip_trivia()?;
        while self.peek() == Some(b',') {
            self.bump();
            self.skip_trivia()?;
            items.push(self.exposed_item()?);
            self.skip_trivia()?;
        }
        self.expect_byte(b')')?;
        Ok(Exposing::Some(items))
    }

    fn exposed_item(&mut self) -> Result<ExposedItem, ParseError> {
        self.skip_trivia()?;
        if matches!(self.peek(), Some(b) if b.is_ascii_lowercase()) {
            return Ok(ExposedItem::Value(self.lower_ident()?.node));
        }
        let name = self.upper_ident_segment()?.node;
        self.skip_trivia()?;
        if self.peek() == Some(b'(') {
            self.bump();
            self.skip_trivia()?;
            self.expect_dotdot()?;
            self.skip_trivia()?;
            self.expect_byte(b')')?;
            Ok(ExposedItem::TypeWithVariants(name))
        } else {
            Ok(ExposedItem::TypeOnly(name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_header_explicit_exposing() {
        let mut s = ParseState::new("module Geometry exposing (Point, distance, Shape(..))");
        let m = s.parse_module().unwrap();
        assert_eq!(m.name, vec!["Geometry".to_string()]);
        let Exposing::Some(items) = m.exposing else {
            panic!("expected Some")
        };
        assert_eq!(
            items,
            vec![
                ExposedItem::TypeOnly("Point".to_string()),
                ExposedItem::Value("distance".to_string()),
                ExposedItem::TypeWithVariants("Shape".to_string()),
            ]
        );
    }

    #[test]
    fn module_header_wildcard_exposing() {
        let src = "module Geometry exposing (..)\n\ntype alias Point = { x : Float, y : Float }";
        let m = ParseState::new(src).parse_module().unwrap();
        assert_eq!(m.exposing, Exposing::All);
        assert_eq!(m.decls.len(), 1);
    }

    #[test]
    fn dotted_module_name() {
        let src = "module Geometry.Shapes exposing (..)\n\nx = 1";
        let m = ParseState::new(src).parse_module().unwrap();
        assert_eq!(m.name, vec!["Geometry".to_string(), "Shapes".to_string()]);
    }

    #[test]
    fn import_qualified() {
        let src = "module Example exposing (..)\n\nimport List\n\ndouble xs = List.map xs xs";
        let m = ParseState::new(src).parse_module().unwrap();
        assert_eq!(m.imports.len(), 1);
        assert_eq!(m.imports[0].module, vec!["List".to_string()]);
        assert!(m.imports[0].alias.is_none());
        assert!(m.imports[0].exposing.is_none());
    }

    #[test]
    fn import_aliased() {
        let src = "module Example exposing (..)\n\nimport Map as M\n\nx = 1";
        let m = ParseState::new(src).parse_module().unwrap();
        assert_eq!(m.imports[0].alias, Some("M".to_string()));
    }

    #[test]
    fn import_exposing_some() {
        let src =
            "module Example exposing (..)\n\nimport String exposing (length, concat)\n\nx = 1";
        let m = ParseState::new(src).parse_module().unwrap();
        let Some(Exposing::Some(items)) = &m.imports[0].exposing else {
            panic!("expected Some(Some)")
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn import_exposing_wildcard() {
        let src = "module Example exposing (..)\n\nimport String exposing (..)\n\nx = 1";
        let m = ParseState::new(src).parse_module().unwrap();
        assert_eq!(m.imports[0].exposing, Some(Exposing::All));
    }

    #[test]
    fn multiple_imports() {
        let src = "module Example exposing (..)\n\nimport List\nimport Map as M\n\nx = 1";
        let m = ParseState::new(src).parse_module().unwrap();
        assert_eq!(m.imports.len(), 2);
    }

    #[test]
    fn full_module_with_multiple_declarations() {
        let src = "module Geometry exposing (Point, distance)\n\nimport List\n\ntype alias Point = { x : Float, y : Float }\n\ndistance :: Point -> Point -> Float\ndistance a b = 0.0";
        let m = ParseState::new(src).parse_module().unwrap();
        assert_eq!(m.decls.len(), 2);
    }

    #[test]
    fn trailing_content_after_last_declaration_is_rejected() {
        let src = "module Example exposing (..)\n\nx = 1\n@@@";
        assert!(ParseState::new(src).parse_module().is_err());
    }
}
