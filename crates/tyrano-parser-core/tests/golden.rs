//! Golden (snapshot) tests: full `SemanticIndex` dumps for a hand-reviewed
//! corpus of `.ks` fixtures.
//!
//! Regenerate the expectation files with:
//! ```sh
//! UPDATE_EXPECT=1 cargo test -p tyrano-parser-core --test golden
//! ```
//! Review every regenerated file before committing — the goldens ARE the
//! specification of the semantic-index shapes and of diagnostic stability.

use std::path::{Path, PathBuf};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
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

#[test]
fn corpus_matches_goldens() {
    let dir = data_dir();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("missing corpus dir {dir:?}"))
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "ks"))
        .collect();
    entries.sort();
    assert!(!entries.is_empty(), "empty corpus dir {dir:?}");

    for ks_path in entries {
        let db = tyrano_db::RootDatabase::default();
        let file = tyrano_db::SourceFile::with_defaults(&db, std::fs::read_to_string(&ks_path).unwrap());
        let idx = tyrano_parser_core::semantic_index(&db, file);
        check_golden(&ks_path.with_extension("sem"), &idx.debug_dump());
    }
}
