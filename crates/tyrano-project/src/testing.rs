//! In-memory project construction for tests (this crate's and its
//! dependents'): no file system, deterministic, one line per file.

use crate::project::{AssetKind, ProjectMetadata, ProjectSettings};
use crate::{ProjectDatabase, ProjectPath};

/// Builds a [`ProjectDatabase`] from in-memory sources.
///
/// ```
/// use tyrano_project::testing::ProjectBuilder;
/// use tyrano_project::AssetKind;
///
/// let db = ProjectBuilder::new()
///     .file("data/scenario/first.ks", "*start\n[jump target=*start]\n")
///     .asset(AssetKind::BgImage, "room.jpg")
///     .build();
/// ```
#[derive(Debug, Default)]
pub struct ProjectBuilder {
    metadata: ProjectMetadata,
}

impl ProjectBuilder {
    /// A builder with default (standard-layout) settings and no files.
    pub fn new() -> ProjectBuilder {
        ProjectBuilder::default()
    }

    /// Replaces the settings.
    pub fn settings(mut self, settings: ProjectSettings) -> ProjectBuilder {
        self.metadata.settings = settings;
        self
    }

    /// Adds a scenario source at a full project-relative path.
    pub fn file(mut self, path: &str, text: &str) -> ProjectBuilder {
        let path = ProjectPath::new(path).expect("valid test path");
        self.metadata.scenario_sources.push((path, text.to_string()));
        self
    }

    /// Registers an asset by `name` relative to the first configured root
    /// of `kind` (`"room.jpg"` → `data/bgimage/room.jpg`).
    pub fn asset(mut self, kind: AssetKind, name: &str) -> ProjectBuilder {
        let root = self
            .metadata
            .settings
            .asset_roots
            .get(&kind)
            .and_then(|roots| roots.first())
            .expect("settings define a root for every asset kind");
        let path = root.join(name).expect("valid test asset name");
        self.metadata.assets.entry(kind).or_default().insert(path);
        self
    }

    /// Instantiates the database.
    pub fn build(self) -> ProjectDatabase {
        ProjectDatabase::new(self.metadata)
    }
}
