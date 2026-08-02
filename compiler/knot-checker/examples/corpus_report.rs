//! Walks `corpus/programs/` (realistic, outcome-agnostic whole-program
//! examples — see that directory's own README.md) and reports, per file,
//! whether it parses, canonicalizes, and type-checks cleanly. Unlike
//! `knot-checker/tests/corpus.rs` (which asserts a pre-declared expectation
//! per fixture), this tool doesn't know or assert an expected outcome — the
//! whole point of `corpus/programs/` is finding out what actually happens.
//!
//! Run with `cargo run --example corpus_report -p knot-checker` from
//! `compiler/`. Exits 0 regardless of how many fixtures fail — this is a
//! reporting tool for the iterate-and-fix cycle, not a pass/fail test.

use std::path::{Path, PathBuf};

use knot_canonical::ast::CDecl;
use knot_syntax::span::Spanned;

fn corpus_root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/programs"
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

enum Outcome {
    Ok,
    ParseError(String),
    LeftoverInput(String),
    CanonFailed(Vec<String>),
    CheckFailed(Vec<String>),
}

fn run(source: &str) -> Outcome {
    let mut state = knot_syntax::ParseState::new(source);
    let decls = match state.parse_decls() {
        Ok(d) => d,
        Err(e) => return Outcome::ParseError(format!("{e:?}")),
    };
    if !state.is_eof() {
        return Outcome::LeftoverInput(state.text_since(state.pos.offset).to_string());
    }

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

fn main() {
    let dir = corpus_root();
    if !dir.is_dir() {
        eprintln!("corpus/programs not found at {}", dir.display());
        std::process::exit(1);
    }

    let files = knot_files(&dir);
    let mut ok_count = 0;
    let mut failures: Vec<(PathBuf, String)> = Vec::new();

    for path in &files {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let rel = path.strip_prefix(&dir).unwrap_or(path);
        match run(&source) {
            Outcome::Ok => {
                ok_count += 1;
                println!("OK    {}", rel.display());
            }
            Outcome::ParseError(msg) => {
                println!("PARSE {}: {msg}", rel.display());
                failures.push((rel.to_path_buf(), format!("parse error: {msg}")));
            }
            Outcome::LeftoverInput(text) => {
                let snippet: String = text.chars().take(60).collect();
                println!("PARSE {}: leftover input: {snippet:?}", rel.display());
                failures.push((rel.to_path_buf(), format!("leftover input: {snippet:?}")));
            }
            Outcome::CanonFailed(kinds) => {
                println!("CANON {}: {kinds:?}", rel.display());
                failures.push((rel.to_path_buf(), format!("canon errors: {kinds:?}")));
            }
            Outcome::CheckFailed(kinds) => {
                println!("TYPE  {}: {kinds:?}", rel.display());
                failures.push((rel.to_path_buf(), format!("type errors: {kinds:?}")));
            }
        }
    }

    println!();
    println!(
        "{ok_count}/{} passed cleanly, {} failed",
        files.len(),
        failures.len()
    );
    if !failures.is_empty() {
        println!();
        println!("Failures:");
        for (path, reason) in &failures {
            println!("  {} -- {reason}", path.display());
        }
    }
}
