//! Tag-level checks against the builtin registry: unknown tags, missing
//! and undeclared parameters, kind mismatches.

use tyrano_db::FileDiagnostic;
use tyrano_js_analysis::JsTypeSet;
use tyrano_project::registry::{ExtraParams, GLOBAL_PARAMS, TagSpec};
use tyrano_project::{CompatibilityDiagnostics, File, ProjectDb, ValueKind};
use tyrano_syntax::ast::{AnyTag, AstNode as _, InterpretOptions, Param};
use tyrano_syntax::diagnostics::Severity;
use tyrano_syntax::text::TextRange;

use crate::check::all_tags;
use crate::check::kind::{ValueClass, check_static, classify};
use crate::codes;
use crate::model::{SemanticModel, TagResolution};
use crate::{
    JsTagSpec, MacroSignature, ScalarValue, ValueCandidates, macro_signature,
    value_candidates_for_parameter,
};

struct BuiltinCheckContext<'a> {
    db: &'a dyn ProjectDb,
    file: File,
    options: &'a InterpretOptions,
    compatibility: CompatibilityDiagnostics,
}

pub(crate) fn check_tags(db: &dyn ProjectDb, file: File) -> Vec<FileDiagnostic> {
    let module = tyrano_db::parsed_module(db, file.source(db));
    let opts = file.source(db).interpret_options(db);
    let model = SemanticModel::new(db, file);
    let compatibility = db.project().settings(db).compatibility_diagnostics;
    let builtin_context = BuiltinCheckContext {
        db,
        file,
        options: &opts,
        compatibility,
    };

    let mut out = Vec::new();
    for tag in all_tags(&module.scenario()) {
        let name = tag.name();
        if name.is_empty() {
            continue; // the parser already reported the missing name
        }
        match model.resolve_tag(&name) {
            TagResolution::Builtin(spec) => {
                let implementation = crate::project_javascript_tag_spec(db, db.project(), &name);
                check_builtin(
                    &builtin_context,
                    &tag,
                    spec,
                    implementation.as_ref(),
                    &mut out,
                )
            }
            TagResolution::Macro(definition) => {
                if let Some(signature) = macro_signature(db, &definition) {
                    check_macro(db, file, &tag, &signature, &opts, &mut out);
                }
            }
            TagResolution::Plugin(spec) => {
                check_javascript_tag(db, file, &tag, &spec, &opts, &mut out)
            }
            TagResolution::Unknown => out.push(FileDiagnostic {
                code: codes::UNKNOWN_TAG,
                severity: Severity::Warning,
                range: name_range(&tag),
                message: format!("unknown tag `{name}` (not a builtin or a visible macro)"),
            }),
        }
    }
    out
}

fn check_builtin(
    context: &BuiltinCheckContext<'_>,
    tag: &AnyTag,
    spec: &'static TagSpec,
    implementation: Option<&JsTagSpec>,
    out: &mut Vec<FileDiagnostic>,
) {
    let params = tag.params();
    // `*` forwards the whole caller parameter object (`mp`); any required
    // parameter may arrive that way, so presence checks are meaningless.
    let forwards_all = params.iter().any(|p| p.is_macro_star());

    if spec.extra == ExtraParams::Deny {
        for param in &params {
            let name = param.name();
            if name.is_empty() || name == "*" {
                continue;
            }
            let implemented = implementation.is_some_and(|implementation| {
                implementation
                    .parameters
                    .iter()
                    .any(|parameter| parameter.name == name)
            });
            if spec.param(&name).is_none()
                && !implemented
                && !GLOBAL_PARAMS.contains(&name.as_str())
            {
                out.push(FileDiagnostic {
                    code: codes::UNKNOWN_PARAM,
                    severity: Severity::Warning,
                    range: param.syntax().text_range(),
                    message: format!("unknown parameter `{name}` on `[{}]`", spec.name),
                });
            }
        }
    }

    if !forwards_all {
        for missing in spec
            .params
            .iter()
            .filter(|ps| ps.required && !params.iter().any(|p| p.name() == ps.name))
        {
            out.push(FileDiagnostic {
                code: codes::MISSING_PARAM,
                severity: Severity::Error,
                range: name_range(tag),
                message: format!(
                    "`[{}]` is missing its required `{}=` parameter",
                    spec.name, missing.name
                ),
            });
        }
        if let Some(implementation) = implementation {
            for missing in implementation.parameters.iter().filter(|parameter| {
                parameter.required
                    && spec.param(&parameter.name).is_none()
                    && !params.iter().any(|param| param.name() == parameter.name)
            }) {
                out.push(FileDiagnostic {
                    code: codes::MISSING_PARAM,
                    severity: Severity::Error,
                    range: name_range(tag),
                    message: format!(
                        "`[{}]` is missing its required `{}=` parameter from the master_tag implementation",
                        spec.name, missing.name
                    ),
                });
            }
        }
    }

    for param in &params {
        let Some(ps) = spec.param(&param.name()) else {
            continue;
        };
        match classify(param, context.options) {
            ValueClass::Static(value) => {
                let compatibility_message = compatibility_diagnostic(ps.kind, &value);
                if let Some(message) = compatibility_message {
                    let severity = compatibility_severity(ps.kind, context.compatibility);
                    out.push(FileDiagnostic {
                        code: codes::KIND_MISMATCH,
                        severity,
                        range: value_range(param),
                        message,
                    });
                } else if let Err(mismatch) = check_static(ps.kind, &value) {
                    out.push(FileDiagnostic {
                        code: codes::KIND_MISMATCH,
                        severity: Severity::Error,
                        range: value_range(param),
                        message: format!(
                            "`{}=` expects {}, got `{value}`",
                            ps.name, mismatch.expectation
                        ),
                    });
                }
            }
            ValueClass::Entity => {
                if let Some(candidates) = value_candidates_for_parameter(
                    context.db,
                    context.file,
                    tag.syntax().text_range(),
                    &param.name(),
                ) && !candidates_match_kind(&candidates, ps.kind)
                {
                    out.push(FileDiagnostic {
                        code: codes::KIND_MISMATCH,
                        severity: Severity::Error,
                        range: value_range(param),
                        message: format!(
                            "`{}=` cannot have the JavaScript value types inferred for this expression",
                            ps.name
                        ),
                    });
                }
            }
            ValueClass::ParamRef | ValueClass::Absent => {}
        }
    }

    if let Some(implementation) = implementation {
        for param in &params {
            let Some(expected) = implementation
                .parameters
                .iter()
                .find(|expected| expected.name == param.name())
            else {
                continue;
            };
            let valid = match classify(param, context.options) {
                ValueClass::Static(value) => static_matches_js_types(&value, expected.types),
                ValueClass::Entity => value_candidates_for_parameter(
                    context.db,
                    context.file,
                    tag.syntax().text_range(),
                    &param.name(),
                )
                .is_none_or(|candidates| candidates_match_js_types(&candidates, expected.types)),
                ValueClass::ParamRef | ValueClass::Absent => true,
            };
            if !valid {
                out.push(FileDiagnostic {
                    code: codes::KIND_MISMATCH,
                    severity: Severity::Error,
                    range: value_range(param),
                    message: format!(
                        "`{}=` does not match the JavaScript type inferred from `[{}]`'s master_tag implementation",
                        param.name(), spec.name
                    ),
                });
            }
        }
    }
}

fn check_javascript_tag(
    db: &dyn ProjectDb,
    file: File,
    tag: &AnyTag,
    spec: &JsTagSpec,
    options: &InterpretOptions,
    out: &mut Vec<FileDiagnostic>,
) {
    let params = tag.params();
    let forwards_all = params.iter().any(|param| param.is_macro_star());
    if !forwards_all {
        for missing in spec.parameters.iter().filter(|parameter| {
            parameter.required && !params.iter().any(|param| param.name() == parameter.name)
        }) {
            out.push(FileDiagnostic {
                code: codes::MISSING_PARAM,
                severity: Severity::Error,
                range: name_range(tag),
                message: format!(
                    "`[{}]` is missing its required `{}=` parameter",
                    tag.name(),
                    missing.name
                ),
            });
        }
    }
    for param in &params {
        if param.name().is_empty()
            || param.name() == "*"
            || GLOBAL_PARAMS.contains(&param.name().as_str())
        {
            continue;
        }
        let Some(expected) = spec
            .parameters
            .iter()
            .find(|expected| expected.name == param.name())
        else {
            out.push(FileDiagnostic {
                code: codes::UNKNOWN_PARAM,
                severity: Severity::Warning,
                range: param.syntax().text_range(),
                message: format!("unknown parameter `{}` on `[{}]`", param.name(), tag.name()),
            });
            continue;
        };
        let valid = match classify(param, options) {
            ValueClass::Static(value) => static_matches_js_types(&value, expected.types),
            ValueClass::Entity => {
                value_candidates_for_parameter(db, file, tag.syntax().text_range(), &param.name())
                    .is_none_or(|candidates| candidates_match_js_types(&candidates, expected.types))
            }
            ValueClass::ParamRef | ValueClass::Absent => true,
        };
        if !valid {
            out.push(FileDiagnostic {
                code: codes::KIND_MISMATCH,
                severity: Severity::Error,
                range: value_range(param),
                message: format!(
                    "`{}=` does not match the JavaScript type inferred from `[{}]`'s master_tag implementation",
                    param.name(), tag.name()
                ),
            });
        }
    }
}

fn check_macro(
    db: &dyn ProjectDb,
    file: File,
    tag: &AnyTag,
    signature: &MacroSignature,
    options: &InterpretOptions,
    out: &mut Vec<FileDiagnostic>,
) {
    let params = tag.params();
    let (parameters, accepts_extra) = active_macro_signature(signature, &params, options);
    let forwards_all = params.iter().any(|param| param.is_macro_star());
    if !forwards_all {
        for missing in parameters.iter().filter(|parameter| {
            parameter.required && !params.iter().any(|param| param.name() == parameter.name)
        }) {
            out.push(FileDiagnostic {
                code: codes::MISSING_PARAM,
                severity: Severity::Error,
                range: name_range(tag),
                message: format!(
                    "`[{}]` is missing its inferred required `{}=` macro parameter",
                    tag.name(),
                    missing.name
                ),
            });
        }
    }
    for param in &params {
        if param.is_macro_star() || GLOBAL_PARAMS.contains(&param.name().as_str()) {
            continue;
        }
        let Some(expected) = parameters
            .iter()
            .find(|expected| expected.name == param.name())
        else {
            if !accepts_extra {
                out.push(FileDiagnostic {
                    code: codes::UNKNOWN_PARAM,
                    severity: Severity::Warning,
                    range: param.syntax().text_range(),
                    message: format!(
                        "parameter `{}` is not used by macro `[{}]`",
                        param.name(),
                        tag.name()
                    ),
                });
            }
            continue;
        };
        let (static_value, candidates) = match classify(param, options) {
            ValueClass::Static(value) => (Some(value), None),
            ValueClass::Entity => (
                None,
                value_candidates_for_parameter(db, file, tag.syntax().text_range(), &param.name()),
            ),
            ValueClass::ParamRef | ValueClass::Absent => continue,
        };
        let kinds_valid = expected.kinds.iter().all(|&kind| {
            static_value.as_ref().is_some_and(|value| {
                if kind == ValueKind::Color {
                    !value.is_empty()
                } else {
                    check_static(kind, value).is_ok()
                }
            }) || candidates
                .as_ref()
                .is_some_and(|candidates| candidates_match_kind(candidates, kind))
        });
        let js_valid = expected.js_types.is_empty()
            || static_value
                .as_ref()
                .is_some_and(|value| static_matches_js_types(value, expected.js_types))
            || candidates
                .as_ref()
                .is_some_and(|candidates| candidates_match_js_types(candidates, expected.js_types));
        if !kinds_valid || !js_valid {
            out.push(FileDiagnostic {
                code: codes::KIND_MISMATCH,
                severity: Severity::Error,
                range: value_range(param),
                message: format!(
                    "`{}=` does not satisfy the uses of `mp.{}` inferred from macro `[{}]`",
                    param.name(),
                    expected.name,
                    tag.name()
                ),
            });
        }
    }
}

fn active_macro_signature(
    signature: &MacroSignature,
    params: &[Param],
    options: &InterpretOptions,
) -> (Vec<crate::MacroParameterSpec>, bool) {
    let selected =
        crate::select_macro_signature(signature, &crate::static_macro_arguments(params, options));
    (selected.parameters, selected.accepts_extra)
}

fn candidates_match_kind(candidates: &ValueCandidates, kind: ValueKind) -> bool {
    if candidates.unknown {
        return true;
    }
    if !candidates.values.is_empty() {
        return candidates
            .values
            .iter()
            .any(|value| scalar_matches_kind(value, kind));
    }
    match kind {
        ValueKind::Any | ValueKind::Expression => true,
        ValueKind::Text
        | ValueKind::Label
        | ValueKind::Color
        | ValueKind::Enum(_)
        | ValueKind::VariableName
        | ValueKind::Scenario
        | ValueKind::Asset(_) => candidates.types.intersects(JsTypeSet::STRING),
        ValueKind::Number => candidates.types.intersects(JsTypeSet::NUMBER),
        ValueKind::Boolean => candidates.types.intersects(JsTypeSet::BOOLEAN),
    }
}

fn scalar_matches_kind(value: &ScalarValue, kind: ValueKind) -> bool {
    match value {
        ScalarValue::String(value) => match kind {
            ValueKind::Color => !value.is_empty(),
            _ => check_static(kind, value).is_ok(),
        },
        ScalarValue::Number(_) => matches!(
            kind,
            ValueKind::Any | ValueKind::Text | ValueKind::Expression | ValueKind::Number
        ),
        ScalarValue::Boolean(_) => matches!(
            kind,
            ValueKind::Any | ValueKind::Text | ValueKind::Expression | ValueKind::Boolean
        ),
        ScalarValue::Null | ScalarValue::Undefined => {
            matches!(kind, ValueKind::Any | ValueKind::Expression)
        }
    }
}

fn static_matches_js_types(value: &str, types: JsTypeSet) -> bool {
    types == JsTypeSet::UNKNOWN
        || types.intersects(JsTypeSet::STRING)
        || types.intersects(JsTypeSet::NUMBER) && value.parse::<f64>().is_ok()
        || types.intersects(JsTypeSet::BOOLEAN) && matches!(value, "true" | "false")
}

fn candidates_match_js_types(candidates: &ValueCandidates, types: JsTypeSet) -> bool {
    candidates.unknown
        || types == JsTypeSet::UNKNOWN
        || candidates.types.intersects(types)
        || candidates.values.iter().any(|value| match value {
            ScalarValue::String(value) => static_matches_js_types(value, types),
            _ => false,
        })
}

fn compatibility_diagnostic(kind: tyrano_project::ValueKind, value: &str) -> Option<String> {
    match kind {
        tyrano_project::ValueKind::Label if !value.is_empty() && !value.starts_with('*') => {
            Some(format!(
                "bare label target `{value}` is accepted by Tyrano; `*{value}` is the canonical form"
            ))
        }
        tyrano_project::ValueKind::Color if !value.is_empty() && !is_hex_color(value) => {
            Some(format!(
                "non-hex color `{value}` is accepted by Tyrano; a hex color is the canonical form"
            ))
        }
        _ => None,
    }
}

fn compatibility_severity(
    kind: tyrano_project::ValueKind,
    settings: CompatibilityDiagnostics,
) -> Severity {
    match kind {
        tyrano_project::ValueKind::Label => settings.bare_label_target.severity(),
        tyrano_project::ValueKind::Color => settings.non_hex_color.severity(),
        _ => Severity::Error,
    }
}

fn is_hex_color(raw: &str) -> bool {
    let digits = raw
        .strip_prefix('#')
        .or_else(|| raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")));
    digits.is_some_and(|digits| {
        (digits.len() == 6 || digits.len() == 8) && digits.chars().all(|c| c.is_ascii_hexdigit())
    })
}

/// The tag-name range (fallback: the whole tag).
fn name_range(tag: &AnyTag) -> TextRange {
    tag.tag_name()
        .map_or(tag.syntax().text_range(), |n| n.syntax().text_range())
}

/// The value range (fallback: the whole parameter).
fn value_range(param: &Param) -> TextRange {
    param
        .value_node()
        .map_or(param.syntax().text_range(), |v| v.syntax().text_range())
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
    fn checks_quoted_dynamic_values_using_javascript_types() {
        let db = ProjectBuilder::new()
            .file("data/scenario/first.ks", "[wait time=\"&true\"]\n")
            .build();
        assert!(
            diagnostics(&db)
                .iter()
                .any(|diagnostic| diagnostic.code == codes::KIND_MISMATCH)
        );
    }

    #[test]
    fn validates_macro_calls_from_inferred_mp_signature() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[macro name=pause][wait time=%delay][endmacro]\n[pause delay=nope]\n",
            )
            .build();
        assert!(
            diagnostics(&db)
                .iter()
                .any(|diagnostic| diagnostic.code == codes::KIND_MISMATCH)
        );
    }

    #[test]
    fn validates_macro_star_against_the_statically_selected_branch() {
        let valid = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                concat!(
                    "[macro name=chara_mask]\n[if exp=\"mp.type === 'show'\"][chara_show *]\n",
                    "[elsif exp=\"mp.type === 'mod'\"][chara_mod *]\n[endif]\n[endmacro]\n",
                    "[chara_mask type=mod name=sui face=sad wait=false]\n"
                ),
            )
            .build();
        assert!(diagnostics(&valid).is_empty());

        let invalid = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                concat!(
                    "[macro name=chara_mask]\n[if exp=\"mp.type === 'show'\"][chara_show *]\n",
                    "[elsif exp=\"mp.type === 'mod'\"][chara_mod *]\n[endif]\n[endmacro]\n",
                    "[chara_mask type=mod name=sui time=not_a_number]\n"
                ),
            )
            .build();
        assert!(
            diagnostics(&invalid)
                .iter()
                .any(|diagnostic| diagnostic.code == codes::KIND_MISMATCH),
            "the selected chara_mod branch requires a numeric time"
        );
    }

    #[test]
    fn validates_reachable_master_tag_vital_parameters() {
        let db = ProjectBuilder::new()
            .file("data/scenario/first.ks", "[loadjs storage=demo.js]\n[custom]\n")
            .javascript(
                "data/others/demo.js",
                "TYRANO.kag.ftag.master_tag.custom = { pm: { storage: '' }, vital: ['storage'] };\n",
            )
            .build();
        assert!(
            diagnostics(&db)
                .iter()
                .any(|diagnostic| diagnostic.code == codes::MISSING_PARAM)
        );
    }

    #[test]
    fn compatibility_diagnostics_default_to_notes_and_honor_configuration() {
        let source = "*start\n[jump target=start]\n[mark color=white]\n";
        let default_db = ProjectBuilder::new()
            .file("data/scenario/first.ks", source)
            .build();
        let default_compat = diagnostics(&default_db)
            .into_iter()
            .filter(|diagnostic| {
                diagnostic.code == codes::KIND_MISMATCH
                    && (diagnostic.message.contains("bare label")
                        || diagnostic.message.contains("non-hex color"))
            })
            .collect::<Vec<_>>();
        assert_eq!(default_compat.len(), 2, "{default_compat:#?}");
        assert!(
            default_compat
                .iter()
                .all(|diagnostic| diagnostic.severity == Severity::Info)
        );

        let settings = tyrano_project::ProjectSettings {
            compatibility_diagnostics: tyrano_project::CompatibilityDiagnostics {
                non_hex_color: tyrano_project::DiagnosticLevel::Error,
                bare_label_target: tyrano_project::DiagnosticLevel::Warning,
            },
            ..tyrano_project::ProjectSettings::default()
        };
        let configured_db = ProjectBuilder::new()
            .settings(settings)
            .file("data/scenario/first.ks", source)
            .build();
        let configured = diagnostics(&configured_db);
        assert!(configured.iter().any(|diagnostic| {
            diagnostic.message.contains("non-hex color") && diagnostic.severity == Severity::Error
        }));
        assert!(configured.iter().any(|diagnostic| {
            diagnostic.message.contains("bare label") && diagnostic.severity == Severity::Warning
        }));
    }
}
