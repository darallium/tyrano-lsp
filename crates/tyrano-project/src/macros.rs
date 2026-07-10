//! The project-wide macro registry.
//!
//! Two-stage query for invalidation hygiene: [`macro_names`] projects
//! *only the macro names* out of a file's semantic index, so an edit that
//! does not touch macro definitions produces an equal projection, salsa
//! backdates it, and the [`macro_registry`] memo built on top survives
//! untouched (the same equality-backdating chain `tyrano-parser-core`
//! already relies on).

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::files::{File, FileStatus};
use crate::project::Project;
use crate::ProjectDb;

/// One project-visible macro definition site.
///
/// Identifies the winning definition by `(file, name)` rather than by a
/// `DefinitionId`: definition ids shift with unrelated edits to the same
/// file, and pinning one here would defeat the registry's backdating.
/// Resolve to a definition on demand via the file's semantic index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroDef {
    pub file: File,
    pub name: String,
}

/// Every macro visible project-wide: name → winning definition.
///
/// First definition wins, in [`Project::scenario_files`] order (sorted by
/// path) and document order within a file — mirroring the engine, which
/// keeps the first `[macro name=…]` it executes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MacroRegistry {
    by_name: BTreeMap<String, MacroDef>,
}

impl MacroRegistry {
    /// The winning definition of macro `name`, if any file defines it.
    pub fn get(&self, name: &str) -> Option<&MacroDef> {
        self.by_name.get(name)
    }

    /// All `(name, definition)` pairs, sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &MacroDef)> {
        self.by_name.iter().map(|(n, d)| (n.as_str(), d))
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// The names of the macros `file` defines, in document order (first
/// definition site per name). Empty for `NotFound` placeholders.
///
/// This is the narrow projection [`macro_registry`] reads per file.
#[salsa::tracked]
pub fn macro_names(db: &dyn ProjectDb, file: File) -> Arc<[String]> {
    if file.status(db) == FileStatus::NotFound {
        return Arc::new([]);
    }
    let index = tyrano_parser_core::semantic_index(db, file.source(db));
    index
        .symbols()
        .iter()
        .filter(|s| s.kind == tyrano_parser_core::SymbolKind::Macro)
        .map(|s| s.name.clone())
        .collect()
}

/// The project-wide [`MacroRegistry`] over `project`'s scenario files.
#[salsa::tracked]
pub fn macro_registry(db: &dyn ProjectDb, project: Project) -> Arc<MacroRegistry> {
    let mut registry = MacroRegistry::default();
    for &file in project.scenario_files(db) {
        for name in macro_names(db, file).iter() {
            registry
                .by_name
                .entry(name.clone())
                .or_insert_with(|| MacroDef { file, name: name.clone() });
        }
    }
    Arc::new(registry)
}

/// Convenience: the registry of `db`'s project.
pub fn project_macros(db: &dyn ProjectDb) -> Arc<MacroRegistry> {
    macro_registry(db, db.project())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ProjectBuilder;
    use crate::{FileChange, ProjectPath};

    fn path(s: &str) -> ProjectPath {
        ProjectPath::new(s).unwrap()
    }

    #[test]
    fn collects_macros_first_definition_wins() {
        let db = ProjectBuilder::new()
            // Path order: a.ks < b.ks, so a.ks's `greet` wins even though
            // b.ks also defines it.
            .file("data/scenario/a.ks", "[macro name=greet]hello[endmacro]\n")
            .file(
                "data/scenario/b.ks",
                "[macro name=greet]hi[endmacro]\n[macro name=bye]bye[endmacro]\n",
            )
            .build();
        let registry = project_macros(&db);

        assert_eq!(registry.len(), 2);
        let a = db.file(&path("data/scenario/a.ks")).unwrap();
        let b = db.file(&path("data/scenario/b.ks")).unwrap();
        assert_eq!(registry.get("greet"), Some(&MacroDef { file: a, name: "greet".into() }));
        assert_eq!(registry.get("bye"), Some(&MacroDef { file: b, name: "bye".into() }));
        assert_eq!(registry.get("nope"), None);
    }

    #[test]
    fn non_macro_edit_keeps_registry_memo() {
        let mut db = ProjectBuilder::new()
            .file("data/scenario/a.ks", "*start\n[macro name=greet]hello[endmacro]\n")
            .build();
        let before = project_macros(&db);

        // Text changes, macro set does not: macro_names backdates, so the
        // registry memo must survive (same Arc).
        db.apply_file_change(
            &path("data/scenario/a.ks"),
            FileChange::Modified(
                "*start\nまったく別の本文\n[macro name=greet]yo[endmacro]\n*end\n".to_string(),
            ),
        );
        let after = project_macros(&db);
        assert!(Arc::ptr_eq(&before, &after), "registry must be backdated");
    }

    #[test]
    fn macro_edit_recomputes_registry() {
        let mut db = ProjectBuilder::new()
            .file("data/scenario/a.ks", "[macro name=greet]hello[endmacro]\n")
            .build();
        let before = project_macros(&db);
        assert!(before.get("greet").is_some());

        db.apply_file_change(
            &path("data/scenario/a.ks"),
            FileChange::Modified("[macro name=salute]hello[endmacro]\n".to_string()),
        );
        let after = project_macros(&db);
        assert!(after.get("greet").is_none());
        assert!(after.get("salute").is_some());
    }

    #[test]
    fn created_file_extends_registry() {
        let mut db = ProjectBuilder::new().build();
        assert!(project_macros(&db).is_empty());

        db.apply_file_change(
            &path("data/scenario/new.ks"),
            FileChange::Created("[macro name=fresh][endmacro]\n".to_string()),
        );
        assert!(project_macros(&db).get("fresh").is_some());
    }
}
