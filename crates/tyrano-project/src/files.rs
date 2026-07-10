//! The path → [`File`] side table (ruff's `Files` pattern).
//!
//! `tyrano-db` deliberately knows nothing about paths; its [`SourceFile`]
//! input is pure text. This module adds the project layer's file identity
//! without editing `tyrano-db`: a [`File`] input pairs a [`ProjectPath`]
//! with its [`SourceFile`], and the [`Files`] table guarantees the pair is
//! created exactly once per path.
//!
//! [`Files`] lives *outside* salsa (it is not an input) but stays
//! deterministic because it is **monotone**: a path's [`File`] handle is
//! created once and never replaced — later changes only touch the handle's
//! fields, which are ordinary salsa inputs. Queries may therefore call
//! [`Files::ensure`] freely: asking for a missing file creates a
//! `NotFound` placeholder whose `status` field the query then reads,
//! making the file's future existence a tracked dependency.
//!
//! [`SourceFile`]: tyrano_db::SourceFile

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::path::ProjectPath;

/// Whether a [`File`] currently exists in the project.
///
/// `NotFound` files are placeholders: they carry an empty [`SourceFile`]
/// and exist so that "file X is missing" is itself a recorded dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileStatus {
    Exists,
    NotFound,
}

/// One project file: the project layer's `FileId`.
///
/// Created only via [`Files::ensure`] so that `path`, the [`Files`] table
/// entry, and the backing [`SourceFile`](tyrano_db::SourceFile) stay in
/// sync. The handle is `Copy` and stable for the life of the database;
/// edits and creation/deletion mutate its fields.
#[salsa::input]
pub struct File {
    /// The normalized project-relative path (immutable in practice).
    #[returns(ref)]
    pub path: ProjectPath,
    /// Whether the file exists; queries that look files up must read this.
    pub status: FileStatus,
    /// The text-level input consumed by `tyrano-db` queries.
    pub source: tyrano_db::SourceFile,
}

impl std::fmt::Debug for File {
    // Not salsa's `debug` option: that would require `SourceFile: Debug`,
    // which `tyrano-db` does not provide. The raw id is enough here.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "File({:?})", self.0)
    }
}

/// The monotone path → [`File`] table shared by one database.
///
/// Cloning shares the table (the database itself is cloned for snapshots).
#[derive(Clone, Default)]
pub struct Files {
    inner: Arc<Mutex<HashMap<ProjectPath, File>>>,
}

impl Files {
    /// The [`File`] for `path`, creating a `NotFound` placeholder (with an
    /// empty, default-options [`SourceFile`](tyrano_db::SourceFile)) on
    /// first sight. Idempotent: one handle per path, forever.
    pub fn ensure(&self, db: &dyn tyrano_db::Db, path: ProjectPath) -> File {
        let mut map = self.inner.lock().expect("Files mutex poisoned");
        if let Some(&file) = map.get(&path) {
            return file;
        }
        let source = tyrano_db::SourceFile::with_defaults(db, String::new());
        let file = File::new(db, path.clone(), FileStatus::NotFound, source);
        map.insert(path, file);
        file
    }

    /// The already-known [`File`] for `path`, without creating one.
    pub fn get(&self, path: &ProjectPath) -> Option<File> {
        self.inner.lock().expect("Files mutex poisoned").get(path).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProjectDatabase, ProjectDb as _};
    use salsa::Setter as _;

    fn path(s: &str) -> ProjectPath {
        ProjectPath::new(s).unwrap()
    }

    #[test]
    fn ensure_is_idempotent() {
        let db = ProjectDatabase::empty();
        let files = db.files().clone();
        let a = files.ensure(&db, path("data/scenario/a.ks"));
        let b = files.ensure(&db, path("data\\scenario\\a.ks"));
        assert_eq!(a, b, "normalized paths share one File handle");
        assert_eq!(files.get(&path("data/scenario/a.ks")), Some(a));
        assert_eq!(files.get(&path("data/scenario/b.ks")), None);
    }

    #[test]
    fn placeholder_is_not_found_and_empty() {
        let db = ProjectDatabase::empty();
        let file = db.files().ensure(&db, path("missing.ks"));
        assert_eq!(file.status(&db), FileStatus::NotFound);
        assert_eq!(file.source(&db).text(&db), "");
        assert_eq!(file.path(&db).as_str(), "missing.ks");
    }

    /// Reading `status` must be a salsa dependency: a query that saw
    /// `NotFound` recomputes once the file comes into existence.
    #[salsa::tracked]
    fn status_probe(db: &dyn crate::ProjectDb, file: File) -> bool {
        file.status(db) == FileStatus::Exists
    }

    #[test]
    fn status_read_is_a_tracked_dependency() {
        let mut db = ProjectDatabase::empty();
        let file = db.files().ensure(&db, path("later.ks"));
        assert!(!status_probe(&db, file));
        file.set_status(&mut db).to(FileStatus::Exists);
        assert!(status_probe(&db, file));
    }
}
