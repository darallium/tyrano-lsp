//! Incremental-correctness regressions: which edits may — and, just as
//! importantly, may **not** — recompute cross-file results.
//!
//! `Arc::ptr_eq` on query results is the observable: salsa returns the
//! same `Arc` exactly when the memo survived (possibly revalidated via
//! backdated inputs), so pointer identity IS the "no recomputation
//! downstream" property these tests pin.

use std::sync::Arc;

use tyrano_project::testing::ProjectBuilder;
use tyrano_project::{File, FileChange, ProjectDatabase, ProjectPath};
use tyrano_semantic::{check_file, exported_labels, file_dependencies};

fn path(s: &str) -> ProjectPath {
    ProjectPath::new(s).unwrap()
}

fn file(db: &ProjectDatabase, p: &str) -> File {
    db.file(&path(p)).expect("fixture file exists")
}

/// first.ks references scene2.ks's `*top`; both clean.
fn linked_project() -> ProjectDatabase {
    ProjectBuilder::new()
        .file("data/scenario/first.ks", "*start\n[jump storage=scene2.ks target=*top]\n[s]\n")
        .file("data/scenario/scene2.ks", "*top\nこんにちは[p]\n*bottom\n[s]\n")
        .build()
}

#[test]
fn non_label_edit_of_dependency_keeps_dependent_diagnostics() {
    let mut db = linked_project();
    let first = file(&db, "data/scenario/first.ks");
    let before = check_file(&db, first);
    assert!(before.is_empty(), "fixture is clean: {before:?}");

    // Rewrite scene2's body — labels and macros unchanged. The label and
    // macro projections backdate, so first.ks's checks must revalidate
    // without recomputing.
    db.apply_file_change(
        &path("data/scenario/scene2.ks"),
        FileChange::Modified("*top\n全く別の身体です[l][p]\n*bottom\n[wait time=100]\n[s]\n".to_string()),
    );
    let after = check_file(&db, first);
    assert!(Arc::ptr_eq(&before, &after), "dependent's diagnostics must be backdated");
}

#[test]
fn label_rename_in_dependency_invalidates_dependent() {
    let mut db = linked_project();
    let first = file(&db, "data/scenario/first.ks");
    assert!(check_file(&db, first).is_empty());

    db.apply_file_change(
        &path("data/scenario/scene2.ks"),
        FileChange::Modified("*renamed\ntext\n[s]\n".to_string()),
    );
    let after = check_file(&db, first);
    assert_eq!(after.len(), 1, "{after:?}");
    assert_eq!(after[0].code, "xsem-unknown-label-in-storage");

    // Renaming it back heals the diagnostic.
    db.apply_file_change(
        &path("data/scenario/scene2.ks"),
        FileChange::Modified("*top\ntext\n[s]\n".to_string()),
    );
    assert!(check_file(&db, first).is_empty());
}

#[test]
fn creating_missing_storage_heals_diagnostics_and_edges() {
    let mut db = ProjectBuilder::new()
        .file("data/scenario/first.ks", "*start\n[jump storage=gone.ks target=*top]\n")
        .build();
    let first = file(&db, "data/scenario/first.ks");

    let before = check_file(&db, first);
    assert_eq!(before.len(), 1, "{before:?}");
    assert_eq!(before[0].code, "xsem-unknown-storage");
    assert!(file_dependencies(&db, first).is_empty(), "unresolved storage: no edge");

    db.apply_file_change(
        &path("data/scenario/gone.ks"),
        FileChange::Created("*top\n[s]\n".to_string()),
    );
    assert!(check_file(&db, first).is_empty(), "created file resolves the reference");
    let gone = file(&db, "data/scenario/gone.ks");
    assert_eq!(&*file_dependencies(&db, first), &[gone], "edge appears with the file");

    db.apply_file_change(&path("data/scenario/gone.ks"), FileChange::Deleted);
    let after = check_file(&db, first);
    assert_eq!(after.len(), 1, "deletion re-breaks the reference: {after:?}");
    assert_eq!(after[0].code, "xsem-unknown-storage");
}

#[test]
fn asset_appearance_scopes_invalidation_to_asset_users() {
    let mut db = ProjectBuilder::new()
        .file("data/scenario/pics.ks", "*start\n[bg storage=room.jpg]\n")
        .file("data/scenario/plain.ks", "*start\nテキストだけ[s]\n")
        .build();
    let pics = file(&db, "data/scenario/pics.ks");
    let plain = file(&db, "data/scenario/plain.ks");

    let pics_before = check_file(&db, pics);
    let plain_before = check_file(&db, plain);
    assert_eq!(pics_before.len(), 1);
    assert_eq!(pics_before[0].code, "xsem-missing-asset");
    assert!(plain_before.is_empty());

    // The asset shows up on disk.
    db.apply_file_change(&path("data/bgimage/room.jpg"), FileChange::Created(String::new()));

    assert!(check_file(&db, pics).is_empty(), "asset now exists");
    assert!(
        Arc::ptr_eq(&plain_before, &check_file(&db, plain)),
        "files without asset references must not recompute"
    );
}

#[test]
fn identical_rewrite_of_dependency_keeps_exported_labels() {
    let mut db = linked_project();
    let scene2 = file(&db, "data/scenario/scene2.ks");
    let before = exported_labels(&db, scene2);

    let text = scene2.source(&db).text(&db).to_string();
    db.apply_file_change(&path("data/scenario/scene2.ks"), FileChange::Modified(text));
    let after = exported_labels(&db, scene2);
    assert!(Arc::ptr_eq(&before, &after), "identical reparse must backdate the projection");
}
