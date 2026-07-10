//! Golden (snapshot) tests: full tree dumps and diagnostic dumps for a
//! hand-reviewed corpus of valid and invalid inputs.
//!
//! Regenerate the expectation files with:
//! ```sh
//! UPDATE_EXPECT=1 cargo test -p tyrano-syntax --test golden
//! ```
//! Review every regenerated file before committing — the goldens ARE the
//! specification of the tree shapes and of diagnostic stability.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use tyrano_syntax::diagnostics::{Lang, render};
use tyrano_syntax::red::{SyntaxElement, SyntaxNode};
use tyrano_syntax::{Parse, parse};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// Full-fidelity tree dump: every node and token with ranges, token texts,
/// missing markers, and attached trivia.
fn tree_dump(parsed: &Parse) -> String {
    let root = SyntaxNode::new_root(parsed.green().clone());
    let mut out = String::new();
    dump_node(&root, 0, &mut out);
    out
}

fn dump_node(node: &SyntaxNode, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let _ = writeln!(out, "{pad}{}@{:?}", node.kind(), node.text_range());
    for el in node.children_with_tokens() {
        match el {
            SyntaxElement::Node(n) => dump_node(&n, depth + 1, out),
            SyntaxElement::Token(t) => {
                let pad = "  ".repeat(depth + 1);
                let _ = write!(out, "{pad}{}@{:?} {:?}", t.kind(), t.text_range(), t.text());
                if t.is_missing() {
                    let _ = write!(out, " (missing)");
                }
                for (kind, range) in t.leading_trivia_ranges() {
                    let _ = write!(out, " lead({kind}@{range:?})");
                }
                for (kind, range) in t.trailing_trivia_ranges() {
                    let _ = write!(out, " trail({kind}@{range:?})");
                }
                out.push('\n');
            }
        }
    }
}

/// Stable diagnostic dump, one per line, with the rendered EN message.
fn diag_dump(parsed: &Parse) -> String {
    let mut out = String::new();
    for d in parsed.diagnostics() {
        let _ = write!(out, "{} {:?} @{:?}", d.code.as_str(), d.severity, d.primary);
        for (range, kind) in &d.secondary {
            let _ = write!(out, " secondary({kind:?}@{range:?})");
        }
        if !d.expected.is_empty() {
            let names: Vec<_> = d.expected.iter().map(|k| k.name()).collect();
            let _ = write!(out, " expected=[{}]", names.join(","));
        }
        let _ = writeln!(out, " :: {}", render(d, Lang::En));
    }
    out
}

fn check_golden(path: &Path, actual: &str) {
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        std::fs::write(path, actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("missing golden {path:?}; run with UPDATE_EXPECT=1"));
    if expected != actual {
        let diff_line = expected
            .lines()
            .zip(actual.lines())
            .position(|(e, a)| e != a)
            .map_or(expected.lines().count().min(actual.lines().count()), |i| i);
        panic!(
            "golden mismatch for {path:?} at line {}:\n expected: {:?}\n actual:   {:?}\n(rerun with UPDATE_EXPECT=1 after reviewing)",
            diff_line + 1,
            expected.lines().nth(diff_line).unwrap_or(""),
            actual.lines().nth(diff_line).unwrap_or(""),
        );
    }
}

fn run_dir(sub: &str, expect_diags: bool) {
    let dir = data_dir().join(sub);
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("missing corpus dir {dir:?}"))
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "ks"))
        .collect();
    entries.sort();
    assert!(!entries.is_empty(), "empty corpus dir {dir:?}");

    for path in entries {
        let source = std::fs::read_to_string(&path).expect("readable corpus file");
        let parsed = parse(&source);
        assert_eq!(parsed.to_source(), source, "{path:?}: round-trip failed");
        if expect_diags {
            assert!(
                !parsed.diagnostics().is_empty(),
                "{path:?}: invalid input must produce diagnostics"
            );
            check_golden(&path.with_extension("diag"), &diag_dump(&parsed));
        } else {
            assert!(
                parsed.diagnostics().is_empty(),
                "{path:?}: valid input must be clean, got {:?}",
                parsed.diagnostics()
            );
        }
        check_golden(&path.with_extension("tree"), &tree_dump(&parsed));
    }
}

#[test]
fn valid_corpus_matches_goldens() {
    run_dir("valid", false);
}

#[test]
fn invalid_corpus_matches_goldens() {
    run_dir("invalid", true);
}
