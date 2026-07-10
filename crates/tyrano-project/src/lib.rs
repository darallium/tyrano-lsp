//! Project layer for TyranoScript analysis.
//!
//! Pipeline position: `tyrano-db` (text → parse) → `tyrano-parser-core`
//! (file-local semantics) → **this crate** (files, paths, project inputs)
//! → `tyrano-module-resolver` / `tyrano-semantic` (cross-file semantics).
//!
//! This crate owns everything the file-local layers deliberately left out:
//! file identity ([`File`], [`ProjectPath`]), the project-wide inputs
//! ([`Project`]) and the concrete database ([`ProjectDatabase`]) that all
//! queries of the workspace meet in. Mutation goes through the inherent
//! methods of [`ProjectDatabase`] ([`apply_file_change`]) — query code
//! never mutates.
//!
//! [`apply_file_change`]: ProjectDatabase::apply_file_change

pub mod files;
pub mod loader;
pub mod macros;
pub mod path;
pub mod project;
pub mod registry;
pub mod testing;

pub use files::{File, FileStatus, Files};
pub use loader::{load_project, load_project_with};
pub use macros::{MacroDef, MacroRegistry, macro_names, macro_registry, project_macros};
pub use path::{ProjectPath, ProjectPathError};
pub use project::{AssetIndex, AssetKind, Project, ProjectMetadata, ProjectSettings};
pub use registry::{ExtraParams, ParamSpec, TagRegistry, TagSpec, ValueKind, builtin_registry};

use salsa::Setter as _;

/// The project-layer database trait. Extends [`tyrano_db::Db`], so any
/// `&dyn ProjectDb` coerces to `&dyn tyrano_db::Db` and the file-local
/// queries of `tyrano-db` / `tyrano-parser-core` run unchanged on it.
#[salsa::db]
pub trait ProjectDb: tyrano_db::Db {
    /// The path → [`File`] side table.
    fn files(&self) -> &Files;
    /// The single [`Project`] of this database.
    fn project(&self) -> Project;
}

/// One external file-system event, as reported by a loader or LSP host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChange {
    /// The file appeared with the given text.
    Created(String),
    /// The file's text changed.
    Modified(String),
    /// The file disappeared.
    Deleted,
}

/// The concrete database of the whole pipeline: the one type implementing
/// every layer's `#[salsa::db]` trait. Tests, the CLI, and the LSP session
/// hold one of these.
#[salsa::db]
#[derive(Clone)]
pub struct ProjectDatabase {
    storage: salsa::Storage<Self>,
    files: Files,
    /// Always `Some` outside of `new` itself (a [`Project`] input cannot
    /// be created before the database exists).
    project: Option<Project>,
}

impl ProjectDatabase {
    /// A database with no files and default settings.
    pub fn empty() -> ProjectDatabase {
        ProjectDatabase::new(ProjectMetadata::default())
    }

    /// Builds a database from loader/test-produced [`ProjectMetadata`].
    pub fn new(metadata: ProjectMetadata) -> ProjectDatabase {
        let mut db = ProjectDatabase {
            storage: salsa::Storage::default(),
            files: Files::default(),
            project: None,
        };
        let project = Project::new(
            &db,
            metadata.settings,
            Vec::new(),
            AssetIndex::new(metadata.assets),
        );
        db.project = Some(project);
        for (path, text) in metadata.scenario_sources {
            db.apply_file_change(&path, FileChange::Created(text));
        }
        db
    }

    /// The already-known [`File`] for `path` (no placeholder creation).
    pub fn file(&self, path: &ProjectPath) -> Option<File> {
        self.files.get(path)
    }

    /// Applies one external file event: updates the [`File`] input (its
    /// status, text, and options), the [`Project::scenario_files`]
    /// snapshot, and the [`AssetIndex`]. The single mutation entry point
    /// for loaders and the LSP host.
    pub fn apply_file_change(&mut self, path: &ProjectPath, change: FileChange) {
        let file = self.files.clone().ensure(self, path.clone());
        let settings = self.project().settings(self).clone();
        // Input writes always count as changes (they never backdate), so
        // fields that are already correct are left alone — re-setting
        // `status` on every keystroke would needlessly invalidate every
        // query that looked this file up.
        match change {
            FileChange::Created(text) | FileChange::Modified(text) => {
                let source = file.source(self);
                source.set_text(self).to(text);
                if source.parse_options(self) != settings.parse_options {
                    source.set_parse_options(self).to(settings.parse_options.clone());
                }
                if source.interpret_options(self) != settings.interpret_options {
                    source.set_interpret_options(self).to(settings.interpret_options.clone());
                }
                if file.status(self) != FileStatus::Exists {
                    file.set_status(self).to(FileStatus::Exists);
                }
                self.set_membership(path, file, &settings, true);
            }
            FileChange::Deleted => {
                file.source(self).set_text(self).to(String::new());
                if file.status(self) != FileStatus::NotFound {
                    file.set_status(self).to(FileStatus::NotFound);
                }
                self.set_membership(path, file, &settings, false);
            }
        }
    }

    /// Inserts or removes `file` in the scenario snapshot and asset index
    /// according to which roots `path` falls under.
    fn set_membership(
        &mut self,
        path: &ProjectPath,
        file: File,
        settings: &ProjectSettings,
        present: bool,
    ) {
        let project = self.project();

        let is_scenario = path.extension() == Some("ks")
            && settings.scenario_roots.iter().any(|r| path.strip_prefix(r).is_some());
        if is_scenario {
            let mut snapshot = project.scenario_files(self).clone();
            let known = snapshot.contains(&file);
            if present && !known {
                snapshot.push(file);
                snapshot.sort_by(|a, b| a.path(self).cmp(b.path(self)));
                project.set_scenario_files(self).to(snapshot);
            } else if !present && known {
                snapshot.retain(|f| *f != file);
                project.set_scenario_files(self).to(snapshot);
            }
        }

        let asset_kinds: Vec<AssetKind> = settings
            .asset_roots
            .iter()
            .filter(|(_, roots)| roots.iter().any(|r| path.strip_prefix(r).is_some()))
            .map(|(&kind, _)| kind)
            .collect();
        if !asset_kinds.is_empty() {
            let mut index = project.asset_index(self).clone();
            for kind in asset_kinds {
                if present {
                    index.insert(kind, path.clone());
                } else {
                    index.remove(kind, path);
                }
            }
            project.set_asset_index(self).to(index);
        }
    }
}

#[salsa::db]
impl salsa::Database for ProjectDatabase {}

#[salsa::db]
impl tyrano_db::Db for ProjectDatabase {}

#[salsa::db]
impl ProjectDb for ProjectDatabase {
    fn files(&self) -> &Files {
        &self.files
    }

    fn project(&self) -> Project {
        self.project.expect("ProjectDatabase::new sets the project input")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ProjectBuilder;

    #[test]
    fn parser_core_queries_run_via_upcast() {
        let db = ProjectDatabase::empty();
        let file = db
            .files()
            .ensure(&db, ProjectPath::new("data/scenario/a.ks").unwrap());
        // `&dyn ProjectDb` (or the concrete db) works directly with the
        // lower layers' queries thanks to the supertrait.
        let idx = tyrano_parser_core::semantic_index(&db, file.source(&db));
        assert!(idx.symbols().is_empty(), "placeholder file is empty");
    }

    #[test]
    fn builder_populates_project() {
        let db = ProjectBuilder::new()
            .file("data/scenario/b.ks", "*start\n")
            .file("data/scenario/a.ks", "*top\n")
            .asset(AssetKind::BgImage, "room.jpg")
            .build();
        let project = db.project();

        let paths: Vec<&str> = project
            .scenario_files(&db)
            .iter()
            .map(|f| f.path(&db).as_str())
            .collect();
        assert_eq!(paths, ["data/scenario/a.ks", "data/scenario/b.ks"], "sorted by path");

        let a = db.file(&ProjectPath::new("data/scenario/a.ks").unwrap()).unwrap();
        assert_eq!(a.status(&db), FileStatus::Exists);
        assert_eq!(a.source(&db).text(&db), "*top\n");

        let room = ProjectPath::new("data/bgimage/room.jpg").unwrap();
        assert!(project.asset_index(&db).contains(AssetKind::BgImage, &room));
        assert!(!project.asset_index(&db).contains(AssetKind::Image, &room));
    }

    #[test]
    fn file_change_created_then_deleted_updates_snapshot() {
        let mut db = ProjectBuilder::new().file("data/scenario/a.ks", "*a\n").build();
        let path = ProjectPath::new("data/scenario/new.ks").unwrap();

        db.apply_file_change(&path, FileChange::Created("*fresh\n".to_string()));
        let file = db.file(&path).unwrap();
        assert_eq!(file.status(&db), FileStatus::Exists);
        assert_eq!(db.project().scenario_files(&db).len(), 2);

        db.apply_file_change(&path, FileChange::Deleted);
        assert_eq!(file.status(&db), FileStatus::NotFound);
        assert_eq!(file.source(&db).text(&db), "");
        assert_eq!(db.project().scenario_files(&db).len(), 1);
    }

    #[test]
    fn non_scenario_paths_do_not_join_the_snapshot() {
        let mut db = ProjectBuilder::new().build();
        // An asset creation event: recorded in the asset index only.
        let bg = ProjectPath::new("data/bgimage/room.jpg").unwrap();
        db.apply_file_change(&bg, FileChange::Created(String::new()));
        assert!(db.project().scenario_files(&db).is_empty());
        assert!(db.project().asset_index(&db).contains(AssetKind::BgImage, &bg));

        // A .ks file outside every scenario root: a File exists, but the
        // project does not include it.
        let stray = ProjectPath::new("elsewhere/x.ks").unwrap();
        db.apply_file_change(&stray, FileChange::Created("*x\n".to_string()));
        assert!(db.project().scenario_files(&db).is_empty());
        assert_eq!(db.file(&stray).unwrap().status(&db), FileStatus::Exists);
    }

    /// Editing one file must not disturb sibling files' memoized queries.
    #[test]
    fn single_file_edit_invalidates_only_that_file() {
        let mut db = ProjectBuilder::new()
            .file("data/scenario/a.ks", "*a\n")
            .file("data/scenario/b.ks", "*b\n")
            .build();
        let a = db.file(&ProjectPath::new("data/scenario/a.ks").unwrap()).unwrap();
        let b = db.file(&ProjectPath::new("data/scenario/b.ks").unwrap()).unwrap();

        let idx_a = tyrano_parser_core::semantic_index(&db, a.source(&db));
        let idx_b = tyrano_parser_core::semantic_index(&db, b.source(&db));

        db.apply_file_change(
            &ProjectPath::new("data/scenario/a.ks").unwrap(),
            FileChange::Modified("*a2\n".to_string()),
        );

        let idx_a2 = tyrano_parser_core::semantic_index(&db, a.source(&db));
        let idx_b2 = tyrano_parser_core::semantic_index(&db, b.source(&db));
        assert!(!std::sync::Arc::ptr_eq(&idx_a, &idx_a2), "edited file recomputes");
        assert!(std::sync::Arc::ptr_eq(&idx_b, &idx_b2), "sibling stays memoized");
    }
}
