//! Cross-file checks: `storage=` targets, labels in other files, and
//! asset existence.

use tyrano_db::FileDiagnostic;
use tyrano_module_resolver::{StorageName, resolve_storage};
use tyrano_parser_core::Resolution;
use tyrano_project::ValueKind;
use tyrano_project::registry::builtin_registry;
use tyrano_project::{File, ProjectDb};
use tyrano_syntax::ast::AstNode as _;
use tyrano_syntax::diagnostics::Severity;

use crate::check::all_tags;
use crate::check::kind::{ValueClass, classify};
use crate::codes;
use crate::labels::exported_labels;
use crate::model::SemanticModel;

pub(crate) fn check_xfile(db: &dyn ProjectDb, file: File) -> Vec<FileDiagnostic> {
    let mut out = Vec::new();
    check_storage_references(db, file, &mut out);
    check_asset_references(db, file, &mut out);
    out
}

/// `storage=` on jump-family tags: the file must exist, and an explicit
/// `target=*label` must be exported by it.
fn check_storage_references(db: &dyn ProjectDb, file: File, out: &mut Vec<FileDiagnostic>) {
    let index = tyrano_parser_core::semantic_index(db, file.source(db));
    for use_ in index.use_def().uses() {
        let Resolution::External { storage, target } = &use_.resolution else { continue };
        let Some(dep) = resolve_storage(db, StorageName::new(db, storage.clone())) else {
            out.push(FileDiagnostic {
                code: codes::UNKNOWN_STORAGE,
                severity: Severity::Error,
                range: use_.range,
                message: format!("scenario file `{storage}` not found"),
            });
            continue;
        };
        // The raw target keeps its `*`; only `*`-prefixed targets are
        // label references (same convention as `SemanticModel::lift`).
        if let Some(label) = target.as_deref().and_then(|t| t.strip_prefix('*'))
            && !exported_labels(db, dep).contains(label)
        {
            out.push(FileDiagnostic {
                code: codes::UNKNOWN_LABEL_IN_STORAGE,
                severity: Severity::Error,
                range: use_.range,
                message: format!("label `{label}` not found in `{storage}`"),
            });
        }
    }
}

/// Static `Asset(kind)` parameter values on builtin tags must name an
/// existing asset. Warning: projects routinely reference assets that
/// arrive later in development.
fn check_asset_references(db: &dyn ProjectDb, file: File, out: &mut Vec<FileDiagnostic>) {
    let module = tyrano_db::parsed_module(db, file.source(db));
    let opts = file.source(db).interpret_options(db);
    let model = SemanticModel::new(db, file);
    let registry = builtin_registry();

    for tag in all_tags(&module.scenario()) {
        let Some(spec) = registry.get(&tag.name()) else { continue };
        for param in tag.params() {
            let Some(ps) = spec.param(&param.name()) else { continue };
            let ValueKind::Asset(kind) = ps.kind else { continue };
            let ValueClass::Static(value) = classify(&param, &opts) else { continue };
            if value.is_empty() || model.resolve_asset(kind, &value).is_some() {
                continue;
            }
            out.push(FileDiagnostic {
                code: codes::MISSING_ASSET,
                severity: Severity::Warning,
                range: param
                    .value_node()
                    .map_or(param.syntax().text_range(), |v| v.syntax().text_range()),
                message: format!(
                    "asset `{value}` not found under the `{}` roots",
                    kind.dir_name()
                ),
            });
        }
    }
}
