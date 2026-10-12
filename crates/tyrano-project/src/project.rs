//! The project-wide salsa input: settings, the scenario-file snapshot,
//! and the asset index.
//!
//! [`Project`] is one input struct with coarse fields; the loader and
//! [`crate::ProjectDatabase::apply_file_change`] keep the snapshots in
//! sync with the outside world. Queries that want finer-grained
//! invalidation project out of these fields (see `macros.rs`).

use std::collections::{BTreeMap, BTreeSet};

use tyrano_syntax::ParseOptions;
use tyrano_syntax::ast::InterpretOptions;

use crate::files::File;
use crate::path::ProjectPath;

/// Configurable severity for engine-compatible, non-canonical syntax.
///
/// `Default` deliberately remains a separate value so clients can follow
/// future project defaults without rewriting their configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Note,
    #[default]
    Default,
}

impl DiagnosticLevel {
    /// Resolves the configured level. Compatibility diagnostics default to
    /// notes because Tyrano accepts the syntax at runtime.
    pub const fn severity(self) -> tyrano_syntax::diagnostics::Severity {
        use tyrano_syntax::diagnostics::Severity;
        match self {
            DiagnosticLevel::Error => Severity::Error,
            DiagnosticLevel::Warning => Severity::Warning,
            DiagnosticLevel::Note | DiagnosticLevel::Default => Severity::Info,
        }
    }
}

/// Policies for syntax that is valid in Tyrano but less explicit than the
/// canonical form used by this analyzer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompatibilityDiagnostics {
    /// CSS/named colors such as `white` instead of `#ffffff`.
    pub non_hex_color: DiagnosticLevel,
    /// Label targets written as `target=start` instead of `target=*start`.
    pub bare_label_target: DiagnosticLevel,
}

/// The asset namespaces of a standard TyranoScript project, each backed
/// by one `data/…` directory by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetKind {
    BgImage,
    Image,
    FgImage,
    Sound,
    Bgm,
    Video,
    Scenario,
    Others,
}

impl AssetKind {
    /// Every kind, in a fixed order.
    pub const ALL: [AssetKind; 8] = [
        AssetKind::BgImage,
        AssetKind::Image,
        AssetKind::FgImage,
        AssetKind::Sound,
        AssetKind::Bgm,
        AssetKind::Video,
        AssetKind::Scenario,
        AssetKind::Others,
    ];

    /// The standard `data/` subdirectory name for this kind.
    pub fn dir_name(self) -> &'static str {
        match self {
            AssetKind::BgImage => "bgimage",
            AssetKind::Image => "image",
            AssetKind::FgImage => "fgimage",
            AssetKind::Sound => "sound",
            AssetKind::Bgm => "bgm",
            AssetKind::Video => "video",
            AssetKind::Scenario => "scenario",
            AssetKind::Others => "others",
        }
    }

    /// The inverse of [`AssetKind::dir_name`].
    pub fn from_dir_name(name: &str) -> Option<AssetKind> {
        AssetKind::ALL.into_iter().find(|k| k.dir_name() == name)
    }
}

/// Project-level configuration: where files live and how they are read.
///
/// The future `Config.tyjs` reader will produce one of these; until then
/// [`ProjectSettings::default`] describes the standard TyranoScript
/// layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSettings {
    /// Directories searched (in order) for scenario files; `storage=`
    /// resolution and [`Project::scenario_files`] membership both use
    /// these.
    pub scenario_roots: Vec<ProjectPath>,
    /// Roots scanned for auxiliary `.ks` and `.js` sources (plugins and
    /// files loaded through `[loadjs]`). Scenario roots remain the only
    /// roots used by `storage=` resolution.
    pub source_roots: Vec<ProjectPath>,
    /// Directories searched (in order) per asset namespace.
    pub asset_roots: BTreeMap<AssetKind, Vec<ProjectPath>>,
    /// Tree-shape options for every scenario file.
    pub parse_options: ParseOptions,
    /// Engine-quirk interpretation options for every scenario file.
    pub interpret_options: InterpretOptions,
    /// Severity policy for engine-compatible forms.
    pub compatibility_diagnostics: CompatibilityDiagnostics,
}

impl Default for ProjectSettings {
    /// The standard layout: `data/scenario` plus one `data/<kind>`
    /// directory per asset namespace.
    fn default() -> Self {
        let data = ProjectPath::new("data").expect("static path");
        ProjectSettings {
            scenario_roots: vec![data.join("scenario").expect("static path")],
            source_roots: vec![
                data.clone(),
                ProjectPath::new("tyrano/plugins/kag").expect("static path"),
            ],
            asset_roots: AssetKind::ALL
                .into_iter()
                .map(|k| (k, vec![data.join(k.dir_name()).expect("static path")]))
                .collect(),
            parse_options: ParseOptions::default(),
            interpret_options: InterpretOptions::default(),
            compatibility_diagnostics: CompatibilityDiagnostics::default(),
        }
    }
}

/// Which asset files exist, by namespace. A snapshot (not per-file
/// inputs): for assets only existence matters, so one coarse value with
/// cheap equality is the right granularity.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AssetIndex {
    by_kind: BTreeMap<AssetKind, BTreeSet<ProjectPath>>,
    all_files: BTreeSet<ProjectPath>,
}

impl AssetIndex {
    /// Builds an index from full project-relative asset paths per kind.
    pub fn new(
        by_kind: BTreeMap<AssetKind, BTreeSet<ProjectPath>>,
        mut all_files: BTreeSet<ProjectPath>,
    ) -> AssetIndex {
        all_files.extend(by_kind.values().flatten().cloned());
        AssetIndex { by_kind, all_files }
    }

    /// Whether `path` exists in namespace `kind`.
    pub fn contains(&self, kind: AssetKind, path: &ProjectPath) -> bool {
        self.by_kind.get(&kind).is_some_and(|s| s.contains(path))
    }

    pub fn contains_any(&self, path: &ProjectPath) -> bool {
        self.all_files.contains(path)
    }

    /// All paths of namespace `kind`, in sorted order.
    pub fn of_kind(&self, kind: AssetKind) -> impl Iterator<Item = &ProjectPath> {
        self.by_kind.get(&kind).into_iter().flatten()
    }

    /// Records `path` in namespace `kind`.
    pub fn insert(&mut self, kind: AssetKind, path: ProjectPath) {
        self.by_kind.entry(kind).or_default().insert(path);
    }

    /// Removes `path` from namespace `kind`.
    pub fn remove(&mut self, kind: AssetKind, path: &ProjectPath) {
        if let Some(set) = self.by_kind.get_mut(&kind) {
            set.remove(path);
        }
    }

    /// Records a file even when it is outside a configured asset namespace
    /// (for example `tyrano/images/system`).
    pub fn insert_file(&mut self, path: ProjectPath) {
        self.all_files.insert(path);
    }

    pub fn remove_file(&mut self, path: &ProjectPath) {
        self.all_files.remove(path);
    }
}

/// The project-wide input every cross-file query hangs off.
#[salsa::input]
pub struct Project {
    /// Configuration; changing it invalidates everything downstream.
    #[returns(ref)]
    pub settings: ProjectSettings,
    /// All scenario files under the scenario roots, sorted by path. A
    /// snapshot maintained by the loader / `apply_file_change`, which is
    /// the "included files" set for project-wide queries.
    #[returns(ref)]
    pub scenario_files: Vec<File>,
    /// `.ks` sources outside scenario roots (normally plugin entry files).
    #[returns(ref)]
    pub library_files: Vec<File>,
    /// Standalone `.js` sources under [`ProjectSettings::source_roots`].
    #[returns(ref)]
    pub javascript_files: Vec<File>,
    /// Which asset files exist, per namespace.
    #[returns(ref)]
    pub asset_index: AssetIndex,
}

/// Everything needed to instantiate a [`crate::ProjectDatabase`]: pure
/// data, produced by the fs loader (`loader.rs`) or built in memory by
/// tests (`testing.rs`), so no salsa code ever touches the file system.
#[derive(Debug, Clone, Default)]
pub struct ProjectMetadata {
    pub settings: ProjectSettings,
    /// Scenario sources as `(path, text)`; order does not matter (the
    /// database sorts by path).
    pub scenario_sources: Vec<(ProjectPath, String)>,
    /// Auxiliary `.ks` sources outside scenario roots.
    pub library_sources: Vec<(ProjectPath, String)>,
    /// Standalone JavaScript sources.
    pub javascript_sources: Vec<(ProjectPath, String)>,
    /// Full project-relative asset paths per namespace.
    pub assets: BTreeMap<AssetKind, BTreeSet<ProjectPath>>,
    /// Every regular file under the project root, including engine resources
    /// outside `data/`.
    pub project_files: BTreeSet<ProjectPath>,
}
