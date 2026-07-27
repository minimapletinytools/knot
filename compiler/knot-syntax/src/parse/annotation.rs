//! `@name(args)` / `@{...}` annotation grammar: the stacked single-key form,
//! the block form, and their shared merge rule (spec §10) — a single argument
//! desugars to a bare value, more than one desugars to a tuple, and a
//! `@{...}` block's entries always win over a stacked `@name(...)` entry for
//! the same key, regardless of which was written first.
//!
//! This module only produces the merged `Vec<Annotation>`; where the prefix
//! `@` actually appears and what the resulting list attaches to (a `FnDef`'s
//! own `annotations` field, wrapping a let-bound value in `Expr::Annotated`,
//! or wrapping a prefix-annotated expression atom — including the "holes
//! can't carry annotations" and "binds to the closest *following* atom only"
//! rules from spec §10.3) is each caller's job, in `expr.rs` and `decl.rs`.

use crate::ast::expr::{Annotation, Expr};
use crate::error::ParseError;
use crate::span::{Span, Spanned};
use crate::state::ParseState;

impl<'a> ParseState<'a> {
    /// One or more consecutive `@name(args)` / `@{...}` occurrences (only
    /// trivia between them), merged into one key-deduplicated list. Callers
    /// must already know an annotation is present (peek `@`) before calling.
    pub(crate) fn annotation_list(&mut self) -> Result<Vec<Annotation>, ParseError> {
        let mut entries: Vec<(String, bool, Spanned<Expr>)> = Vec::new();
        loop {
            let checkpoint = self.clone();
            self.skip_trivia()?;
            if self.peek() != Some(b'@') {
                *self = checkpoint;
                break;
            }
            self.bump(); // '@'
            if self.peek() == Some(b'{') {
                for (key, value) in self.annotation_block()? {
                    merge_annotation_entry(&mut entries, key, value, true);
                }
            } else {
                let (key, value) = self.annotation_single()?;
                merge_annotation_entry(&mut entries, key, value, false);
            }
        }
        Ok(entries
            .into_iter()
            .map(|(key, _, value)| Annotation { key, value })
            .collect())
    }

    /// `name(arg, arg, ...)`, with the `@` already consumed. A single
    /// argument desugars to a bare value; more than one desugars to a tuple
    /// (spec §10.1's desugaring rule).
    fn annotation_single(&mut self) -> Result<(String, Spanned<Expr>), ParseError> {
        let key = self.lower_ident()?.node;
        self.expect_byte(b'(')?;
        self.skip_trivia()?;
        let mut args = vec![self.expr()?];
        self.skip_trivia()?;
        while self.peek() == Some(b',') {
            self.bump();
            self.skip_trivia()?;
            args.push(self.expr()?);
            self.skip_trivia()?;
        }
        self.expect_byte(b')')?;
        let value = if args.len() == 1 {
            args.into_iter().next().expect("checked len == 1 above")
        } else {
            let span = Span::new(
                args.first()
                    .expect("at least one arg, guaranteed above")
                    .span
                    .start,
                args.last()
                    .expect("at least one arg, guaranteed above")
                    .span
                    .end,
            );
            Spanned::new(span, Expr::Tuple(args))
        };
        Ok((key, value))
    }

    /// `{ key = value, ... }`, with the `@` already consumed — identical
    /// grammar to record construction (including the empty `{}` case), so
    /// this reuses `expr_record_field_list` directly.
    fn annotation_block(&mut self) -> Result<Vec<(String, Spanned<Expr>)>, ParseError> {
        self.expect_byte(b'{')?;
        self.skip_trivia()?;
        if self.peek() == Some(b'}') {
            self.bump();
            return Ok(Vec::new());
        }
        let fields = self.expr_record_field_list()?;
        self.expect_byte(b'}')?;
        Ok(fields)
    }
}

/// Inserts or overwrites `key` -> `value`. A block-sourced entry always wins;
/// among two stacked entries for the same key (an edge case the spec doesn't
/// really address), the later one wins.
fn merge_annotation_entry(
    entries: &mut Vec<(String, bool, Spanned<Expr>)>,
    key: String,
    value: Spanned<Expr>,
    from_block: bool,
) {
    if let Some(pos) = entries.iter().position(|(k, _, _)| *k == key) {
        let existing_from_block = entries[pos].1;
        if from_block || !existing_from_block {
            entries[pos] = (key, from_block, value);
        }
    } else {
        entries.push((key, from_block, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_arg_desugars_to_bare_value() {
        let mut s = ParseState::new(r#"@label("My Function")"#);
        let list = s.annotation_list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].key, "label");
        assert_eq!(
            list[0].value.node,
            Expr::StringLit("My Function".to_string())
        );
    }

    #[test]
    fn multi_arg_desugars_to_tuple() {
        let mut s = ParseState::new("@position(100, 200)");
        let list = s.annotation_list().unwrap();
        assert_eq!(list[0].key, "position");
        assert_eq!(
            list[0].value.node,
            Expr::Tuple(vec![
                Spanned::new(Span::new(10, 13), Expr::IntLit(100)),
                Spanned::new(Span::new(15, 18), Expr::IntLit(200)),
            ])
        );
    }

    #[test]
    fn stacked_lines_all_collected() {
        let src = "@nodeId(\"f1\")\n@position(100, 200)\n@label(\"My Function\")";
        let mut s = ParseState::new(src);
        let list = s.annotation_list().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(
            list.iter().map(|a| a.key.as_str()).collect::<Vec<_>>(),
            vec!["nodeId", "position", "label"]
        );
    }

    #[test]
    fn block_form_multiple_keys() {
        let src = r##"@{ nodeId = "f1", position = (100.0, 200.0), color = "#a0c4ff" }"##;
        let mut s = ParseState::new(src);
        let list = s.annotation_list().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].key, "nodeId");
        assert_eq!(list[2].key, "color");
    }

    #[test]
    fn empty_block_form() {
        let mut s = ParseState::new("@{}");
        let list = s.annotation_list().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn block_overrides_stacked_on_conflicting_key() {
        let src = r##"@color("red")
@{ color = "#a0c4ff", unravel = myCustomUnraveler }"##;
        let mut s = ParseState::new(src);
        let list = s.annotation_list().unwrap();
        let color = list.iter().find(|a| a.key == "color").unwrap();
        assert_eq!(color.value.node, Expr::StringLit("#a0c4ff".to_string()));
        assert_eq!(list.len(), 2); // color (block-won) + unravel
    }

    #[test]
    fn block_before_stacked_still_wins() {
        // Order in the source shouldn't matter -- block always wins on a
        // conflicting key, even if it happens to be written first.
        let src = r##"@{ color = "#a0c4ff" }
@color("red")"##;
        let mut s = ParseState::new(src);
        let list = s.annotation_list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].value.node, Expr::StringLit("#a0c4ff".to_string()));
    }
}
