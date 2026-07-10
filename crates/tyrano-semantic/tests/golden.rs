//! Golden (snapshot) tests: full `check_project` output over hand-reviewed
//! multi-file project fixtures.
//!
//! Each `tests/data/<case>/` holds:
//! - `project/…` — the project tree; every `.ks` under it becomes an
//!   in-memory scenario source (paths relative to `project/`);
//! - `assets.list` (optional) — one `<kind-dir> <name>` line per asset
//!   (`bgimage room.jpg`), registered without any real file;
//! - `expected.diag` — the expected diagnostics, one
//!   `path:line:col: severity code: message` line each.
//!
//! Regenerate the expectation files with:
//! ```sh
//! UPDATE_EXPECT=1 cargo test -p tyrano-semantic --test golden
//! ```
//! Review every regenerated file before committing — the goldens ARE the
//! specification of cross-file diagnostic behaviour.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use tyrano_project::testing::ProjectBuilder;
use tyrano_project::{AssetKind, ProjectDatabase};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// Walks `dir` recursively, returning file paths sorted for determinism.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else { continue };
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn build_case(case: &Path) -> ProjectDatabase {
    let project_dir = case.join("project");
    let mut builder = ProjectBuilder::new();
    for file in files_under(&project_dir) {
        if file.extension().is_some_and(|e| e == "ks") {
            let rel = file.strip_prefix(&project_dir).expect("under project/");
            let text = std::fs::read_to_string(&file).expect("fixture is UTF-8");
            builder = builder.file(rel.to_str().expect("UTF-8 fixture path"), &text);
        }
    }
    let assets = case.join("assets.list");
    if let Ok(list) = std::fs::read_to_string(&assets) {
        for line in list.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (kind, name) = line.split_once(' ').expect("`<kind-dir> <name>` line");
            let kind = AssetKind::from_dir_name(kind)
                .unwrap_or_else(|| panic!("unknown asset kind dir {kind:?} in {assets:?}"));
            builder = builder.asset(kind, name);
        }
    }
    builder.build()
}

fn render_diagnostics(db: &ProjectDatabase) -> String {
    let mut out = String::new();
    for (file, diags) in tyrano_semantic::check_project(db) {
        let path = file.path(db).clone();
        let index = tyrano_db::line_index(db, file.source(db));
        for diag in diags.iter() {
            let _ = writeln!(out, "{path}:{}", diag.render_with_location(&index));
        }
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
        panic!(
            "golden mismatch for {path:?}\n== expected ==\n{expected}== actual ==\n{actual}\
             (rerun with UPDATE_EXPECT=1 after reviewing)"
        );
    }
}

#[test]
fn corpus_matches_goldens() {
    let dir = data_dir();
    let mut cases: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("missing corpus dir {dir:?}"))
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "empty corpus dir {dir:?}");

    for case in cases {
        let db = build_case(&case);
        let actual = render_diagnostics(&db);
        check_golden(&case.join("expected.diag"), &actual);
    }
}
