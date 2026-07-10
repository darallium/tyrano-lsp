//! Tag-level checks against the builtin registry: unknown tags, missing
//! and undeclared parameters, kind mismatches.

use tyrano_db::FileDiagnostic;
use tyrano_project::registry::{ExtraParams, GLOBAL_PARAMS, TagSpec};
use tyrano_project::{File, ProjectDb};
use tyrano_syntax::ast::{AnyTag, AstNode as _, InterpretOptions, Param};
use tyrano_syntax::diagnostics::Severity;
use tyrano_syntax::text::TextRange;

use crate::check::kind::{ValueClass, check_static, classify};
use crate::check::all_tags;
use crate::codes;
use crate::model::{SemanticModel, TagResolution};

pub(crate) fn check_tags(db: &dyn ProjectDb, file: File) -> Vec<FileDiagnostic> {
    let module = tyrano_db::parsed_module(db, file.source(db));
    let opts = file.source(db).interpret_options(db);
    let model = SemanticModel::new(db, file);

    let mut out = Vec::new();
    for tag in all_tags(&module.scenario()) {
        let name = tag.name();
        if name.is_empty() {
            continue; // the parser already reported the missing name
        }
        match model.resolve_tag(&name) {
            TagResolution::Builtin(spec) => check_builtin(&tag, spec, &opts, &mut out),
            // Macro calls: parameters flow into `mp` untyped; nothing to
            // validate against.
            TagResolution::Macro(_) => {}
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
    tag: &AnyTag,
    spec: &'static TagSpec,
    opts: &InterpretOptions,
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
            if spec.param(&name).is_none() && !GLOBAL_PARAMS.contains(&name.as_str()) {
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
    }

    for param in &params {
        let Some(ps) = spec.param(&param.name()) else { continue };
        if let ValueClass::Static(value) = classify(param, opts)
            && let Err(mismatch) = check_static(ps.kind, &value)
        {
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
}

/// The tag-name range (fallback: the whole tag).
fn name_range(tag: &AnyTag) -> TextRange {
    tag.tag_name().map_or(tag.syntax().text_range(), |n| n.syntax().text_range())
}

/// The value range (fallback: the whole parameter).
fn value_range(param: &Param) -> TextRange {
    param.value_node().map_or(param.syntax().text_range(), |v| v.syntax().text_range())
}
