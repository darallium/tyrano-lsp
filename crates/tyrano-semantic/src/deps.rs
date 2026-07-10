//! The cross-file dependency graph: which scenario files reference which.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tyrano_module_resolver::{StorageName, resolve_storage};
use tyrano_parser_core::Resolution;
use tyrano_project::{File, FileStatus, Project, ProjectDb};

/// The files `file` references via `storage=` (jump/call/…), resolved,
/// deduplicated, in first-occurrence order. Unresolvable storage names
/// contribute no edge (they surface as diagnostics instead), but their
/// candidate placeholders anchor recomputation for when the file appears.
#[salsa::tracked]
pub fn file_dependencies(db: &dyn ProjectDb, file: File) -> Arc<[File]> {
    if file.status(db) == FileStatus::NotFound {
        return Arc::new([]);
    }
    let index = tyrano_parser_core::semantic_index(db, file.source(db));
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for use_ in index.use_def().uses() {
        if let Resolution::External { storage, .. } = &use_.resolution
            && let Some(dep) = resolve_storage(db, StorageName::new(db, storage.clone()))
            && seen.insert(dep)
        {
            out.push(dep);
        }
    }
    out.into()
}

/// Forward and reverse `storage=` edges over one project's scenario files.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DependencyGraph {
    edges: HashMap<File, Arc<[File]>>,
    reverse: HashMap<File, Vec<File>>,
}

impl DependencyGraph {
    /// The files `file` references.
    pub fn dependencies(&self, file: File) -> &[File] {
        self.edges.get(&file).map(|deps| &**deps).unwrap_or(&[])
    }

    /// The files that reference `file`, in scenario-file (path) order.
    pub fn dependents(&self, file: File) -> &[File] {
        self.reverse.get(&file).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// The [`DependencyGraph`] over `project`'s scenario files.
#[salsa::tracked]
pub fn project_dependency_graph(db: &dyn ProjectDb, project: Project) -> Arc<DependencyGraph> {
    let mut graph = DependencyGraph::default();
    for &file in project.scenario_files(db) {
        let deps = file_dependencies(db, file);
        for &dep in deps.iter() {
            graph.reverse.entry(dep).or_default().push(file);
        }
        graph.edges.insert(file, deps);
    }
    Arc::new(graph)
}
