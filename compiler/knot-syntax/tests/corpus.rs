//! Walks `corpus/` (at the repo root, shared with future crates — see
//! `knot-ast-parser-plan.md` §6) and checks that every `valid/` fixture parses
//! cleanly and completely, and every `invalid/` fixture is rejected (either a
//! parse error, or parsing only part of the file and leaving the rest
//! unconsumed).
//!
//! Not yet doing AST snapshot comparison (`insta`) — that's a reasonable
//! follow-up once there's a stable AST worth pinning down file by file. For
//! now this is the first end-to-end check that the whole pipeline agrees with
//! the fixtures written while designing the grammar, before that investment.
//!
//! Every fixture outside `modules/` is a bare declaration (or short list of
//! them) with no module header — that's deliberate, matching the build order
//! (grammar layers below the module system were designed and tested before it
//! existed) — so those go through `parse_decls`, while `modules/` fixtures
//! (which do have real headers) go through the full `knot_syntax::parse`.

use std::path::{Path, PathBuf};

fn corpus_root() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus")).to_path_buf()
}

fn knot_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = walk(dir);
    files.retain(|p| p.extension().and_then(|e| e.to_str()) == Some("knot"));
    files.sort();
    files
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn is_module_fixture(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "modules")
}

/// Annotation grammar is M6, not yet implemented (see `knot-ast-parser-plan.md`
/// §5) — its fixtures are expected to fail for now. Excluding a whole category
/// like this is meant to be a visible, temporary carve-out, not a quiet one:
/// remove it the moment M6 lands, and if any *other* category starts needing
/// this treatment that's a real regression, not something to paper over the
/// same way.
fn not_yet_implemented(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "annotations")
}

/// True if `source` parses successfully, the parser fully consumed it (leftover
/// input -- e.g. from a token the grammar simply doesn't recognize, like the
/// removed `.` composition operator or `$` -- counts as failure too, same as
/// the `ex`/`err` test helpers used throughout the grammar's own unit tests),
/// and it survives post-parse validation (tuple arity, duplicate bindings).
fn parses_completely(source: &str, path: &Path) -> bool {
    if is_module_fixture(path) {
        knot_syntax::parse(source).is_ok()
    } else {
        let mut state = knot_syntax::ParseState::new(source);
        let Ok(decls) = state.parse_decls() else {
            return false;
        };
        if !state.is_eof() {
            return false;
        }
        let module = knot_syntax::ast::decl::Module {
            name: Vec::new(),
            exposing: knot_syntax::ast::decl::Exposing::All,
            imports: Vec::new(),
            decls,
        };
        knot_syntax::validate::validate_module(&module).is_empty()
    }
}

#[test]
fn valid_fixtures_parse_completely() {
    let dir = corpus_root().join("valid");
    assert!(dir.is_dir(), "corpus/valid not found at {}", dir.display());
    let mut failures = Vec::new();
    for path in knot_files(&dir) {
        if not_yet_implemented(&path) {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        if !parses_completely(&source, &path) {
            failures.push(path.display().to_string());
        }
    }
    assert!(
        failures.is_empty(),
        "expected these valid fixtures to parse completely, but they didn't:\n{}",
        failures.join("\n")
    );
}

#[test]
fn invalid_fixtures_are_rejected() {
    let dir = corpus_root().join("invalid");
    assert!(
        dir.is_dir(),
        "corpus/invalid not found at {}",
        dir.display()
    );
    let mut failures = Vec::new();
    for path in knot_files(&dir) {
        if not_yet_implemented(&path) {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        if parses_completely(&source, &path) {
            failures.push(path.display().to_string());
        }
    }
    assert!(
        failures.is_empty(),
        "expected these invalid fixtures to be rejected, but they parsed cleanly:\n{}",
        failures.join("\n")
    );
}
