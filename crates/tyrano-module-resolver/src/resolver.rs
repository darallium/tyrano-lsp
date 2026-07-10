//! `storage=` → [`File`] resolution.

use tyrano_project::{File, FileStatus, ProjectDb, ProjectPath, ProjectSettings};

use crate::storage_name::StorageName;

/// The paths a storage name could denote, in search order: for each
/// scenario root (in order), the name as written, then — when the name
/// has no extension — with `.ks` appended (the engine's completion).
/// Names that do not form valid project paths (absolute, `..`) yield no
/// candidates.
pub fn candidate_paths(settings: &ProjectSettings, name: &str) -> Vec<ProjectPath> {
    let mut out = Vec::new();
    for root in &settings.scenario_roots {
        let Ok(exact) = root.join(name) else { continue };
        let needs_completion = exact.extension().is_none();
        out.push(exact);
        if needs_completion
            && let Ok(completed) = root.join(&format!("{name}.ks"))
        {
            out.push(completed);
        }
    }
    out
}

/// Resolves a storage name to the first existing candidate [`File`].
///
/// Every candidate is materialized via [`Files::ensure`] and its `status`
/// read, so missing candidates become `NotFound` placeholder
/// dependencies: creating one of those files later re-runs this query and
/// the resolution flips without any manual invalidation.
///
/// [`Files::ensure`]: tyrano_project::Files::ensure
#[salsa::tracked]
pub fn resolve_storage(db: &dyn ProjectDb, name: StorageName) -> Option<File> {
    let settings = db.project().settings(db);
    for path in candidate_paths(settings, name.text(db)) {
        let file = db.files().ensure(db, path);
        if file.status(db) == FileStatus::Exists {
            return Some(file);
        }
    }
    None
}

/// Convenience wrapper: resolve raw `&str` names against one database.
pub struct StorageResolver<'db> {
    db: &'db dyn ProjectDb,
}

impl<'db> StorageResolver<'db> {
    pub fn new(db: &'db dyn ProjectDb) -> StorageResolver<'db> {
        StorageResolver { db }
    }

    /// Resolves `name` to an existing [`File`], if any.
    pub fn resolve(&self, name: &str) -> Option<File> {
        resolve_storage(self.db, StorageName::new(self.db, name.to_string()))
    }
}
