//! Walks `corpus/semantic/` (at the repo root, sibling to `knot-syntax`'s own
//! `corpus/syntax/` corpus test) and checks that every `valid/` fixture
//! parses, canonicalizes, and type-checks (`check::check_module`) with zero
//! errors, and every `invalid/` fixture fails at one of those three stages
//! with the expected error kind.
//!
//! See `corpus/semantic/README.md` for why this tier exists (it exercises
//! feature *interactions* a hand-built unit test rarely thinks to combine)
//! and the interaction matrix each fixture is meant to cover.
//!
//! **Fixture convention**, mirroring `corpus/syntax/invalid/`'s own leading
//! `-- expect: <reason>` comment:
//! - Every fixture starts with `-- expect: <human-readable reason>`.
//! - Every `invalid/` fixture also has a second line, `-- error-kind:
//!   <Name>`, where `<Name>` is the failing error's own enum variant name
//!   (from whichever of `CanonErrorKind`/`TypeErrorKind` actually produced
//!   it — a fixture can fail at canonicalization *or* at `check_module`).
//!   Matched against each error's own `{:?}` `Debug` output by prefix
//!   (`NoInstance` matches `NoInstance { interface: "Eq" }`) — checking the
//!   exact kind, not just "did something fail," is the point: this corpus
//!   exists because "did it accept/reject" isn't enough to catch a bug that
//!   makes the checker fail for the *wrong* reason (as happened twice this
//!   session before this harness existed).

use std::path::{Path, PathBuf};

use knot_canonical::ast::CDecl;
use knot_syntax::span::Spanned;

fn corpus_root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/semantic"
    ))
    .to_path_buf()
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

/// The `-- error-kind: <Name>` tag from an `invalid/` fixture's own second
/// line, if present.
fn expected_error_kind(source: &str) -> Option<&str> {
    source
        .lines()
        .find_map(|line| line.strip_prefix("-- error-kind:"))
        .map(|s| s.trim())
}

enum Outcome {
    Ok,
    CanonFailed(Vec<String>),
    CheckFailed(Vec<String>),
}

/// Runs one fixture all the way through `knot_syntax::parse_decls` ->
/// `knot_canonical::canonicalize_decls` -> `knot_checker::check::
/// check_module`, stopping at whichever stage first reports a problem.
/// Every debug-formatted error kind from the failing stage is returned so
/// `expected_error_kind` can be matched against it.
fn run(source: &str) -> Outcome {
    let mut state = knot_syntax::ParseState::new(source);
    let decls = state
        .parse_decls()
        .unwrap_or_else(|e| panic!("parse error: {e:?}\nsource:\n{source}"));
    assert!(state.is_eof(), "leftover input after parsing:\n{source}");

    let cdecls: Vec<Spanned<CDecl>> = match knot_canonical::canonicalize_decls(&decls) {
        Ok(cdecls) => cdecls,
        Err(errs) => {
            return Outcome::CanonFailed(errs.iter().map(|e| format!("{:?}", e.kind)).collect());
        }
    };

    let errors = knot_checker::check::check_module(&cdecls);
    if errors.is_empty() {
        Outcome::Ok
    } else {
        Outcome::CheckFailed(errors.iter().map(|e| format!("{:?}", e.kind)).collect())
    }
}

#[test]
fn valid_fixtures_type_check_with_no_errors() {
    let dir = corpus_root().join("valid");
    assert!(
        dir.is_dir(),
        "corpus/semantic/valid not found at {}",
        dir.display()
    );
    let mut failures = Vec::new();
    for path in knot_files(&dir) {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        match run(&source) {
            Outcome::Ok => {}
            Outcome::CanonFailed(kinds) => failures.push(format!(
                "{}: canonicalization failed: {kinds:?}",
                path.display()
            )),
            Outcome::CheckFailed(kinds) => {
                failures.push(format!("{}: type errors: {kinds:?}", path.display()))
            }
        }
    }
    assert!(
        failures.is_empty(),
        "expected these valid fixtures to type-check cleanly, but they didn't:\n{}",
        failures.join("\n")
    );
}

#[test]
fn invalid_fixtures_fail_with_the_expected_error_kind() {
    let dir = corpus_root().join("invalid");
    assert!(
        dir.is_dir(),
        "corpus/semantic/invalid not found at {}",
        dir.display()
    );
    let mut failures = Vec::new();
    for path in knot_files(&dir) {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let Some(expected) = expected_error_kind(&source) else {
            failures.push(format!(
                "{}: missing a `-- error-kind: <Name>` tag",
                path.display()
            ));
            continue;
        };
        let kinds = match run(&source) {
            Outcome::Ok => {
                failures.push(format!(
                    "{}: expected `{expected}`, but this fixture type-checked cleanly",
                    path.display()
                ));
                continue;
            }
            Outcome::CanonFailed(kinds) | Outcome::CheckFailed(kinds) => kinds,
        };
        if !kinds.iter().any(|k| k.starts_with(expected)) {
            failures.push(format!(
                "{}: expected an error starting with `{expected}`, got {kinds:?}",
                path.display()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "invalid fixtures didn't fail as expected:\n{}",
        failures.join("\n")
    );
}
