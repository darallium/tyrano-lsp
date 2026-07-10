//! The inverse mapping: [`File`] → canonical storage name.

use tyrano_project::{File, ProjectDb};

use crate::storage_name::StorageName;

/// A scenario file seen as a module: the file plus the canonical storage
/// name (`"sub/ev.ks"`) under which sibling scripts address it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, salsa::Update)]
pub struct ScriptModule {
    pub file: File,
    /// Path relative to the first matching scenario root.
    pub storage: StorageName,
}

/// The module of `file`: its path stripped of the first scenario root
/// that contains it. `None` for files outside every scenario root — they
/// have no storage name and cannot be jumped to.
#[salsa::tracked]
pub fn script_module(db: &dyn ProjectDb, file: File) -> Option<ScriptModule> {
    let settings = db.project().settings(db);
    let path = file.path(db);
    settings.scenario_roots.iter().find_map(|root| {
        let rest = path.strip_prefix(root)?;
        Some(ScriptModule { file, storage: StorageName::new(db, rest.to_string()) })
    })
}
