//! [`SemanticModel`]: the primary interface an LSP layer talks to.
//!
//! A deliberately thin facade — two words of state (`db`, `file`); every
//! piece of data comes out of a salsa query, so holding a model never
//! caches anything staleable.

use std::sync::Arc;

use tyrano_module_resolver::{StorageName, resolve_storage};
use tyrano_parser_core::{DefinitionId, Resolution, SemanticIndex, SymbolKind, UseId};
use tyrano_project::macros::MacroDef;
use tyrano_project::registry::{TagSpec, builtin_registry};
use tyrano_project::{AssetKind, File, ProjectDb, ProjectPath};

use crate::deps::{file_dependencies, project_dependency_graph};

/// A resolved label definition site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelTarget {
    pub file: File,
    /// Index into `semantic_index(file)`'s definition arena.
    pub def: DefinitionId,
}

/// Outcome of resolving `storage=` + `target=*label` across files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelResolution {
    Found(LabelTarget),
    /// The storage name resolves to no existing file.
    FileNotFound,
    /// The file exists but does not define the label.
    LabelNotFound(File),
}

/// What a tag name means at the project level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagResolution {
    /// A builtin tag (builtins shadow same-named macros, as in the
    /// engine, which registers macros only for unknown tag names).
    Builtin(&'static TagSpec),
    /// A file-local or project-wide macro.
    Macro(MacroDef),
    Unknown,
}

/// Where a file-local reference points once the project layer weighs in:
/// `tyrano_parser_core::Resolution` with `External` actually resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectResolution {
    /// A definition in this same file.
    Local(DefinitionId),
    /// Another file, no label target (`[jump storage=…]`).
    ExternalFile(File),
    /// A label in another file.
    ExternalLabel(LabelTarget),
    /// `storage=` names no existing file.
    FileNotFound { storage: String },
    /// The file exists but lacks the target label.
    LabelNotFound { file: File, label: String },
    /// Unresolved within the file and not external (e.g. a character
    /// name never defined anywhere we can see).
    Unknown,
}

/// Semantic queries about one file in one project revision.
#[derive(Clone, Copy)]
pub struct SemanticModel<'db> {
    db: &'db dyn ProjectDb,
    file: File,
}

impl<'db> SemanticModel<'db> {
    pub fn new(db: &'db dyn ProjectDb, file: File) -> SemanticModel<'db> {
        SemanticModel { db, file }
    }

    pub fn file(&self) -> File {
        self.file
    }

    /// This file's local semantic index.
    pub fn index(&self) -> Arc<SemanticIndex> {
        tyrano_parser_core::semantic_index(self.db, self.file.source(self.db))
    }

    /// A label in *this* file (first definition wins).
    pub fn resolve_label(&self, name: &str) -> Option<LabelTarget> {
        label_in(self.db, self.file, name)
    }

    /// A `storage=` + label pair, resolved across files.
    pub fn resolve_label_in(&self, storage: &str, label: &str) -> LabelResolution {
        let Some(file) = self.resolve_storage(storage) else {
            return LabelResolution::FileNotFound;
        };
        match label_in(self.db, file, label) {
            Some(target) => LabelResolution::Found(target),
            None => LabelResolution::LabelNotFound(file),
        }
    }

    /// A macro by name: this file's definitions first (the engine
    /// registers local macros immediately), then the project registry.
    pub fn resolve_macro(&self, name: &str) -> Option<MacroDef> {
        if self.index().macro_(name).is_some() {
            return Some(MacroDef { file: self.file, name: name.to_string() });
        }
        tyrano_project::project_macros(self.db).get(name).cloned()
    }

    /// What tag `name` invokes: builtin, macro, or nothing we know.
    pub fn resolve_tag(&self, name: &str) -> TagResolution {
        if let Some(spec) = builtin_registry().get(name) {
            return TagResolution::Builtin(spec);
        }
        match self.resolve_macro(name) {
            Some(def) => TagResolution::Macro(def),
            None => TagResolution::Unknown,
        }
    }

    /// The existing asset `name` refers to in namespace `kind`, searched
    /// across the configured roots in order.
    pub fn resolve_asset(&self, kind: AssetKind, name: &str) -> Option<ProjectPath> {
        let project = self.db.project();
        let settings = project.settings(self.db);
        let index = project.asset_index(self.db);
        settings.asset_roots.get(&kind)?.iter().find_map(|root| {
            let path = root.join(name).ok()?;
            index.contains(kind, &path).then_some(path)
        })
    }

    /// A scenario file by raw storage name.
    pub fn resolve_storage(&self, name: &str) -> Option<File> {
        resolve_storage(self.db, StorageName::new(self.db, name.to_string()))
    }

    /// Lifts a file-local reference to its project-level resolution.
    pub fn resolve_reference(&self, use_id: UseId) -> ProjectResolution {
        self.lift(self.index().use_def().resolution(use_id))
    }

    /// Every reference of the file lifted to its project-level
    /// resolution, in document order (parallel to `index().uses()`).
    pub fn resolved_references(&self) -> Vec<ProjectResolution> {
        self.index().uses().iter().map(|use_| self.lift(&use_.resolution)).collect()
    }

    fn lift(&self, resolution: &Resolution) -> ProjectResolution {
        match resolution {
            Resolution::Def(def) => ProjectResolution::Local(*def),
            Resolution::Unknown => ProjectResolution::Unknown,
            Resolution::External { storage, target } => {
                let Some(file) = self.resolve_storage(storage) else {
                    return ProjectResolution::FileNotFound { storage: storage.clone() };
                };
                // `External::target` is the raw cooked value (`"*top"`);
                // only a `*`-prefixed target is a label reference.
                match target.as_deref().and_then(|t| t.strip_prefix('*')) {
                    None => ProjectResolution::ExternalFile(file),
                    Some(label) => match label_in(self.db, file, label) {
                        Some(t) => ProjectResolution::ExternalLabel(t),
                        None => ProjectResolution::LabelNotFound {
                            file,
                            label: label.to_string(),
                        },
                    },
                }
            }
        }
    }

    /// Files this file references via `storage=`.
    pub fn dependencies(&self) -> Arc<[File]> {
        file_dependencies(self.db, self.file)
    }

    /// Scenario files that reference this file.
    pub fn dependents(&self) -> Vec<File> {
        project_dependency_graph(self.db, self.db.project()).dependents(self.file).to_vec()
    }

    /// Every diagnostic of this file, file-local and cross-file merged
    /// (see [`crate::check_file`]).
    pub fn diagnostics(&self) -> Arc<[tyrano_db::FileDiagnostic]> {
        crate::check_file(self.db, self.file)
    }
}

/// The winning definition of label `name` in `file`, if any.
fn label_in(db: &dyn ProjectDb, file: File, name: &str) -> Option<LabelTarget> {
    let index = tyrano_parser_core::semantic_index(db, file.source(db));
    let symbol = index.label(name)?;
    let def = *index.symbol(symbol).defs.first()?;
    debug_assert_eq!(index.symbol(symbol).kind, SymbolKind::Label);
    Some(LabelTarget { file, def })
}
