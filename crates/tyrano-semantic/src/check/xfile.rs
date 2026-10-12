//! Cross-file checks: `storage=` targets, labels in other files, and
//! asset existence.

use tyrano_db::FileDiagnostic;
use tyrano_module_resolver::{StorageName, resolve_storage};
use tyrano_parser_core::Resolution;
use tyrano_project::registry::builtin_registry;
use tyrano_project::{AssetKind, ValueKind};
use tyrano_project::{File, ProjectDb};
use tyrano_syntax::ast::AstNode as _;
use tyrano_syntax::diagnostics::Severity;

use crate::check::all_tags;
use crate::check::kind::{ValueClass, classify};
use crate::codes;
use crate::flow::macro_at;
use crate::labels::exported_labels;
use crate::model::SemanticModel;
use crate::{analyze_macro_value_bindings, value_candidates_for_parameter_with_mp};

#[derive(Debug)]
enum ParameterValues {
    Absent,
    Known(Vec<String>),
    Unknown,
}

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
    let module = tyrano_db::parsed_module(db, file.source(db));
    let tags = all_tags(&module.scenario());
    let options = file.source(db).interpret_options(db);
    for use_ in index.use_def().uses() {
        let Resolution::External { storage, target } = &use_.resolution else {
            continue;
        };
        let tag = tags
            .iter()
            .find(|tag| contains_range(tag.syntax().text_range(), use_.range));
        let storage_values = if storage.trim_start().starts_with(['&', '%']) {
            let Some(tag) = tag else { continue };
            match parameter_values(db, file, tag, "storage", &options) {
                ParameterValues::Known(values) => values,
                ParameterValues::Absent | ParameterValues::Unknown => continue,
            }
        } else {
            vec![storage.clone()]
        };
        let resolved = storage_values
            .iter()
            .filter_map(|candidate| {
                resolve_storage(db, StorageName::new(db, candidate.clone()))
                    .map(|file| (candidate, file))
            })
            .collect::<Vec<_>>();
        if resolved.is_empty() {
            out.push(FileDiagnostic {
                code: codes::UNKNOWN_STORAGE,
                severity: Severity::Error,
                range: use_.range,
                message: if storage_values.len() == 1 {
                    format!("scenario file `{}` not found", storage_values[0])
                } else {
                    format!(
                        "no possible scenario storage was found: {}",
                        storage_values.join(", ")
                    )
                },
            });
            continue;
        }
        let target_values = match target.as_deref() {
            None => continue,
            Some(target) if target.trim_start().starts_with(['&', '%']) => {
                let Some(tag) = tag else { continue };
                match parameter_values(db, file, tag, "target", &options) {
                    ParameterValues::Known(values) => values,
                    ParameterValues::Absent | ParameterValues::Unknown => continue,
                }
            }
            Some(target) => vec![target.to_owned()],
        };
        let has_label_candidate = resolved.iter().any(|(_, dependency)| {
            target_values.iter().any(|target| {
                let label = target.strip_prefix('*').unwrap_or(target);
                exported_labels(db, *dependency).contains(label)
            })
        });
        if !has_label_candidate {
            let static_pair = storage_values.len() == 1
                && target_values.len() == 1
                && !storage.trim_start().starts_with(['&', '%'])
                && target
                    .as_deref()
                    .is_some_and(|target| !target.trim_start().starts_with(['&', '%']));
            out.push(FileDiagnostic {
                code: codes::UNKNOWN_LABEL_IN_STORAGE,
                severity: Severity::Error,
                range: use_.range,
                message: if static_pair {
                    format!(
                        "label `{}` not found in `{}`",
                        target_values[0]
                            .strip_prefix('*')
                            .unwrap_or(&target_values[0]),
                        storage_values[0]
                    )
                } else {
                    format!(
                        "none of the possible labels ({}) exist in the resolved storage candidates",
                        target_values.join(", ")
                    )
                },
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
        let Some(spec) = registry.get(&tag.name()) else {
            continue;
        };
        for param in tag.params() {
            let Some(ps) = spec.param(&param.name()) else {
                continue;
            };
            let ValueKind::Asset(kind) = ps.kind else {
                continue;
            };
            let static_reference = matches!(classify(&param, &opts), ValueClass::Static(_));
            let values = match parameter_values(db, file, &tag, &param.name(), &opts) {
                ParameterValues::Known(values) => values,
                ParameterValues::Absent | ParameterValues::Unknown => continue,
            };
            if values.is_empty()
                || values.iter().any(|value| {
                    value.is_empty() || asset_exists(db, file, &model, &tag, kind, value, &opts)
                })
            {
                continue;
            }
            out.push(FileDiagnostic {
                code: codes::MISSING_ASSET,
                severity: Severity::Warning,
                range: param
                    .value_node()
                    .map_or(param.syntax().text_range(), |v| v.syntax().text_range()),
                message: if static_reference && values.len() == 1 {
                    format!(
                        "asset `{}` not found under the `{}` roots",
                        values[0],
                        kind.dir_name()
                    )
                } else {
                    format!("no possible asset was found: {}", values.join(", "))
                },
            });
        }
    }
}

fn parameter_values(
    db: &dyn ProjectDb,
    file: File,
    tag: &tyrano_syntax::ast::AnyTag,
    name: &str,
    options: &tyrano_syntax::ast::InterpretOptions,
) -> ParameterValues {
    let Some(param) = tag.param(name) else {
        return ParameterValues::Absent;
    };
    match classify(&param, options) {
        ValueClass::Static(value) => ParameterValues::Known(vec![value]),
        ValueClass::Entity => {
            let tag_range = tag.syntax().text_range();
            let bindings = analyze_macro_value_bindings(db);
            let mp = macro_at(db, file, tag_range)
                .and_then(|macro_name| bindings.by_macro.get(&macro_name).cloned())
                .unwrap_or_default();
            value_candidates_for_parameter_with_mp(db, file, tag_range, name, &mp)
        }
        .filter(|candidates| candidates.is_definitely_finite_strings())
        .map(|candidates| candidates.string_values().map(str::to_owned).collect())
        .map_or(ParameterValues::Unknown, ParameterValues::Known),
        ValueClass::ParamRef | ValueClass::Absent => ParameterValues::Unknown,
    }
}

fn asset_exists(
    db: &dyn ProjectDb,
    file: File,
    model: &SemanticModel<'_>,
    tag: &tyrano_syntax::ast::AnyTag,
    declared_kind: AssetKind,
    value: &str,
    options: &tyrano_syntax::ast::InterpretOptions,
) -> bool {
    if is_external_resource(value) {
        return true;
    }
    match parameter_values(db, file, tag, "folder", options) {
        ParameterValues::Known(folders) if !folders.is_empty() => {
            return folders
                .iter()
                .any(|folder| model.resolve_asset_in_folder(folder, value).is_some());
        }
        ParameterValues::Unknown => return true,
        ParameterValues::Absent | ParameterValues::Known(_) => {}
    }

    if tag.name() == "image" && tag.param("storage").is_some() {
        return match parameter_values(db, file, tag, "layer", options) {
            ParameterValues::Known(layers) => layers.iter().any(|layer| {
                let kind = if layer == "base" {
                    AssetKind::BgImage
                } else {
                    AssetKind::FgImage
                };
                model.resolve_asset(kind, value).is_some()
            }),
            ParameterValues::Absent => model.resolve_asset(AssetKind::BgImage, value).is_some(),
            ParameterValues::Unknown => {
                model.resolve_asset(AssetKind::BgImage, value).is_some()
                    || model.resolve_asset(AssetKind::FgImage, value).is_some()
            }
        };
    }
    model.resolve_asset(declared_kind, value).is_some()
}

fn is_external_resource(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("data:")
        || lower.starts_with("blob:")
}

fn contains_range(
    outer: tyrano_syntax::text::TextRange,
    inner: tyrano_syntax::text::TextRange,
) -> bool {
    outer.start() <= inner.start() && outer.end() >= inner.end()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_project::{ProjectPath, testing::ProjectBuilder};

    fn diagnostics(db: &tyrano_project::ProjectDatabase) -> Vec<FileDiagnostic> {
        let file = db
            .file(&ProjectPath::new("data/scenario/first.ks").unwrap())
            .unwrap();
        crate::check_file(db, file).to_vec()
    }

    #[test]
    fn dynamic_asset_candidates_use_preceding_js_and_image_layer_folder() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[iscript]\ntf.base = 'config/';\n[endscript]\n[image layer=1 storage=\"&tf.base + 'button.png'\"]\n",
            )
            .asset(AssetKind::FgImage, "config/button.png")
            .build();
        assert!(
            !diagnostics(&db)
                .iter()
                .any(|diagnostic| diagnostic.code == codes::MISSING_ASSET)
        );
    }

    #[test]
    fn image_uses_fgimage_for_non_base_and_accepts_engine_system_resources() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[image layer=1 storage=builder.png]\n[image layer=base storage=../../tyrano/images/system/bg_base.png]\n",
            )
            .asset(AssetKind::FgImage, "builder.png")
            .resource("tyrano/images/system/bg_base.png")
            .build();
        assert!(
            !diagnostics(&db)
                .iter()
                .any(|diagnostic| diagnostic.code == codes::MISSING_ASSET)
        );
    }

    #[test]
    fn existing_member_of_dynamic_storage_candidates_prevents_false_error() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[iscript]\ntf.next = ready ? 'exists.ks' : 'missing.ks';\n[endscript]\n[jump storage=\"&tf.next\"]\n",
            )
            .file("data/scenario/exists.ks", "*start\n")
            .build();
        assert!(
            !diagnostics(&db)
                .iter()
                .any(|diagnostic| diagnostic.code == codes::UNKNOWN_STORAGE)
        );
    }

    #[test]
    fn macro_mp_storage_is_resolved_from_call_arguments() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[macro name=go][jump storage=\"&mp.name + '.ks'\"][endmacro]\n[go name=exists]\n",
            )
            .file("data/scenario/exists.ks", "*start\n")
            .build();
        assert!(
            !diagnostics(&db)
                .iter()
                .any(|diagnostic| diagnostic.code == codes::UNKNOWN_STORAGE)
        );
    }
}
