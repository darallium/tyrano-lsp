//! Cross-file label visibility.

use std::collections::BTreeSet;
use std::sync::Arc;

use tyrano_project::{File, FileStatus, ProjectDb};

/// The label names `file` defines — the set other files can jump to.
///
/// A names-only projection (like `tyrano_project::macro_names`): body
/// edits that leave the label set unchanged backdate here, so dependents
/// checking `storage=`+`target=` pairs against this set stay memoized.
#[salsa::tracked]
pub fn exported_labels(db: &dyn ProjectDb, file: File) -> Arc<BTreeSet<String>> {
    if file.status(db) == FileStatus::NotFound {
        return Arc::new(BTreeSet::new());
    }
    let index = tyrano_parser_core::semantic_index(db, file.source(db));
    Arc::new(
        index
            .symbols()
            .iter()
            .filter(|s| s.kind == tyrano_parser_core::SymbolKind::Label)
            .map(|s| s.name.clone())
            .collect(),
    )
}
