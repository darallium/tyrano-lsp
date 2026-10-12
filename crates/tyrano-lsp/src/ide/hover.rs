//! Hover: markdown documentation for the construct under the cursor.

use std::fmt::Write as _;

use tyrano_parser_core::{RefKind, SymbolKind, semantic_index};
use tyrano_project::registry::{GLOBAL_PARAMS, TagSpec, ValueKind, builtin_registry};
use tyrano_project::{AssetKind, File, ProjectDb};
use tyrano_semantic::{
    JsTagSpec, MacroSignature, ProjectResolution, ScalarValue, SemanticModel, TagResolution,
    macro_signature, value_candidates_in_range,
};
use tyrano_syntax::text::{TextRange, TextSize};

use super::cursor::{CursorTarget, classify};

/// A hover response: markdown plus the range it applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverResult {
    pub markdown: String,
    pub range: TextRange,
}

/// Hover documentation at `offset` in `file`.
pub fn hover(db: &dyn ProjectDb, file: File, offset: TextSize) -> Option<HoverResult> {
    if file.path(db).extension() == Some("js") {
        return None;
    }
    let model = SemanticModel::new(db, file);
    let arguments = super::macro_call::arguments_at(db, file, offset).unwrap_or_default();
    match classify(db, file, offset) {
        CursorTarget::Def { def, range } => {
            let index = semantic_index(db, file.source(db));
            let symbol = index.symbol(index.definition(def).symbol);
            let path = file.path(db);
            let markdown = match symbol.kind {
                SymbolKind::Label => {
                    format!("**\\*{}** — label in `{path}`", symbol.name)
                }
                SymbolKind::Macro => {
                    format!("**[{}]** — macro defined in `{path}`", symbol.name)
                }
                SymbolKind::Character => {
                    format!("**#{}** — character", symbol.name)
                }
            };
            Some(HoverResult { markdown, range })
        }
        CursorTarget::Use { index: i, range } => {
            let index = semantic_index(db, file.source(db));
            let use_ = &index.uses()[i];
            let resolution = &model.resolved_references()[i];
            let markdown = match use_.kind {
                RefKind::JumpTarget => match dynamic_reference_name(&use_.name, resolution) {
                    Some(name) => describe_dynamic_reference(db, &model, name, range, resolution)?,
                    None => describe_jump(db, &use_.name, resolution),
                },
                RefKind::MacroCall => describe_tag(db, &model, &use_.name, &arguments)?,
                RefKind::CharacterRef => format!("**#{}** — character reference", use_.name),
            };
            Some(HoverResult { markdown, range })
        }
        CursorTarget::Tag { name, range } => Some(HoverResult {
            markdown: describe_tag(db, &model, &name, &arguments)?,
            range,
        }),
        CursorTarget::ParamName { tag, name, range } => {
            let markdown = describe_param(db, &model, &tag, &name, &arguments)?;
            Some(HoverResult { markdown, range })
        }
        CursorTarget::ParamValue {
            tag,
            param,
            value,
            range,
        } => {
            let markdown = describe_value(db, &model, &tag, &param, &value, range, &arguments)?;
            Some(HoverResult { markdown, range })
        }
        CursorTarget::None => None,
    }
}

fn describe_jump(db: &dyn ProjectDb, name: &str, resolution: &ProjectResolution) -> String {
    match resolution {
        ProjectResolution::Local(_) => {
            format!("**\\*{name}** — label in this file")
        }
        ProjectResolution::ExternalLabel(target) => {
            format!("**\\*{name}** — label in `{}`", target.file.path(db))
        }
        ProjectResolution::ExternalFile(file) => {
            format!("scenario file `{}`", file.path(db))
        }
        ProjectResolution::FileNotFound { storage } => {
            format!("⚠ scenario file `{storage}` not found")
        }
        ProjectResolution::LabelNotFound { file, label } => {
            format!("⚠ label `*{label}` not found in `{}`", file.path(db))
        }
        ProjectResolution::Unknown => format!("⚠ unresolved reference `{name}`"),
    }
}

/// Markdown for a tag name: builtin doc + parameter table, or macro site.
fn describe_tag(
    db: &dyn ProjectDb,
    model: &SemanticModel<'_>,
    name: &str,
    arguments: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    match model.resolve_tag(name) {
        TagResolution::Builtin(spec) => Some(builtin_doc(db, spec)),
        TagResolution::Macro(def) => {
            let signature = macro_signature(db, &def);
            Some(macro_doc(db, &def, signature.as_ref(), arguments))
        }
        TagResolution::Plugin(spec) => Some(javascript_tag_doc(name, &spec)),
        TagResolution::Unknown => None,
    }
}

fn builtin_doc(db: &dyn ProjectDb, spec: &TagSpec) -> String {
    let mut out = format!("**[{}]** (builtin)\n\n{}", spec.name, spec.doc);
    if !spec.params.is_empty() {
        out.push_str("\n\n**Parameters**\n");
        for p in spec.params {
            let requirement = if p.required { "required" } else { "optional" };
            let _ = write!(
                out,
                "\n- `{}` — {}, {requirement}",
                p.name,
                kind_name(p.kind)
            );
            if let Some(default) = p.default {
                let _ = write!(out, " (default `{default}`)");
            }
        }
    }
    if let Some(implementation) =
        tyrano_semantic::project_javascript_tag_spec(db, db.project(), spec.name)
        && !implementation.parameters.is_empty()
    {
        out.push_str("\n\n**master_tag implementation types**\n");
        for parameter in implementation.parameters {
            let requirement = if parameter.required {
                "required"
            } else {
                "optional"
            };
            let _ = write!(
                out,
                "\n- `{}` — {}, {requirement}",
                parameter.name,
                js_type_name(parameter.types)
            );
        }
    }
    out
}

fn describe_param(
    db: &dyn ProjectDb,
    model: &SemanticModel<'_>,
    tag: &str,
    name: &str,
    arguments: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    if GLOBAL_PARAMS.contains(&name) {
        return Some(format!(
            "`{name}` — universal parameter (JavaScript condition; the tag runs only when it is truthy)"
        ));
    }
    if let Some(spec) = builtin_registry().get(tag) {
        let param = spec.param(name)?;
        let requirement = if param.required {
            "required"
        } else {
            "optional"
        };
        let mut out = format!(
            "`{name}` — parameter of **[{tag}]**: {}, {requirement}",
            kind_name(param.kind)
        );
        if let Some(default) = param.default {
            let _ = write!(out, ", default `{default}`");
        }
        return Some(out);
    }
    match model.resolve_tag(tag) {
        TagResolution::Macro(definition) => {
            let signature = macro_signature(db, &definition)?;
            let selected = super::macro_call::select(&signature, arguments);
            let docs = super::macro_call::parameter_docs(&selected, Some(name));
            if docs.is_empty() {
                return None;
            }
            Some(
                docs.into_iter()
                    .map(|(name, detail)| {
                        format!("`{name}` — inferred macro parameter of **[{tag}]**: {detail}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            )
        }
        TagResolution::Plugin(spec) => {
            let parameter = spec
                .parameters
                .iter()
                .find(|parameter| parameter.name == name)?;
            let requirement = if parameter.required {
                "required"
            } else {
                "optional"
            };
            Some(format!(
                "`{name}` — master_tag parameter of **[{tag}]**: {}, {requirement}",
                js_type_name(parameter.types)
            ))
        }
        TagResolution::Builtin(_) | TagResolution::Unknown => None,
    }
}

fn macro_doc(
    db: &dyn ProjectDb,
    definition: &tyrano_project::MacroDef,
    signature: Option<&MacroSignature>,
    arguments: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut out = format!(
        "**[{}]** — macro defined in `{}`",
        definition.name,
        definition.file.path(db)
    );
    if let Some(signature) = signature {
        let selected = super::macro_call::select(signature, arguments);
        for condition in &selected.matched {
            let _ = write!(
                out,
                "\n\nSelected: `{}`",
                super::macro_call::conditions(condition)
            );
        }
        out.push_str("\n\n**Inferred parameters**\n");
        for (name, detail) in super::macro_call::parameter_docs(&selected, None) {
            let _ = write!(out, "\n- `{name}` — {detail}");
        }
        out.push_str("\n- `*` — forwards all caller `mp` parameters");
    }
    out
}

fn javascript_tag_doc(name: &str, spec: &JsTagSpec) -> String {
    let mut out = format!("**[{name}]** — custom tag registered by reachable plugin JavaScript");
    if !spec.parameters.is_empty() {
        out.push_str("\n\n**master_tag parameters**\n");
        for parameter in &spec.parameters {
            let requirement = if parameter.required {
                "required"
            } else {
                "optional"
            };
            let _ = write!(
                out,
                "\n- `{}` — {}, {requirement}",
                parameter.name,
                js_type_name(parameter.types)
            );
        }
    }
    out
}

pub(super) fn macro_parameter_types(parameter: &tyrano_semantic::MacroParameterSpec) -> String {
    let mut types = parameter
        .kinds
        .iter()
        .map(|kind| kind_name(*kind))
        .collect::<Vec<_>>();
    if !parameter.js_types.is_empty() {
        types.push(js_type_name(parameter.js_types));
    }
    if types.is_empty() {
        "unknown value".to_owned()
    } else {
        types.join(" + ")
    }
}

fn js_type_name(types: tyrano_js_analysis::JsTypeSet) -> String {
    if types == tyrano_js_analysis::JsTypeSet::UNKNOWN {
        return "unknown JavaScript value".to_owned();
    }
    let mut names = Vec::new();
    for (kind, name) in [
        (tyrano_js_analysis::JsTypeSet::UNDEFINED, "undefined"),
        (tyrano_js_analysis::JsTypeSet::NULL, "null"),
        (tyrano_js_analysis::JsTypeSet::BOOLEAN, "boolean"),
        (tyrano_js_analysis::JsTypeSet::NUMBER, "number"),
        (tyrano_js_analysis::JsTypeSet::STRING, "string"),
        (tyrano_js_analysis::JsTypeSet::ARRAY, "array"),
        (tyrano_js_analysis::JsTypeSet::OBJECT, "object"),
        (tyrano_js_analysis::JsTypeSet::FUNCTION, "function"),
    ] {
        if types.intersects(kind) {
            names.push(name);
        }
    }
    if names.is_empty() {
        "unknown JavaScript value".to_owned()
    } else {
        names.join(" | ")
    }
}

fn describe_value(
    db: &dyn ProjectDb,
    model: &SemanticModel<'_>,
    tag: &str,
    param: &str,
    value: &str,
    range: TextRange,
    arguments: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    let dynamic = value.trim_start();
    if dynamic.starts_with('&') {
        return describe_javascript_value(
            db,
            model,
            range,
            param_kind(tag, param) == Some(ValueKind::Scenario),
        );
    }
    if let Some(reference) = dynamic.strip_prefix('%') {
        let (name, fallback) = reference
            .split_once('|')
            .map_or((reference, None), |(name, fallback)| (name, Some(fallback)));
        let mut out = format!("macro parameter `mp.{}`", name.trim());
        if let Some(fallback) = fallback {
            let _ = write!(out, " (fallback `{}`)", fallback.trim());
        }
        return Some(out);
    }
    if let TagResolution::Macro(definition) = model.resolve_tag(tag) {
        let signature = macro_signature(db, &definition)?;
        let selected = super::macro_call::select(&signature, arguments);
        let mut docs = Vec::new();
        let mut add = |parameters: &[tyrano_semantic::MacroParameterSpec],
                       condition: Option<String>| {
            for p in parameters.iter().filter(|p| p.name == param) {
                for kind in &p.kinds {
                    let detail = match kind {
                        ValueKind::Asset(kind) => match model.resolve_asset(*kind, value) {
                            Some(path) => format!("{} asset `{path}`", kind.dir_name()),
                            None => format!("⚠ {} asset `{value}` not found", kind.dir_name()),
                        },
                        ValueKind::Scenario => match model.resolve_storage(value) {
                            Some(file) => format!("scenario file `{}`", file.path(db)),
                            None => format!("⚠ scenario file `{value}` not found"),
                        },
                        _ => continue,
                    };
                    docs.push(match &condition {
                        Some(c) => format!("{detail} when {c}"),
                        None => detail,
                    });
                }
            }
        };
        add(&selected.parameters, None);
        for branch in &selected.pending {
            add(
                &branch.parameters,
                Some(super::macro_call::conditions(&branch.conditions)),
            );
        }
        return (!docs.is_empty()).then(|| docs.join("\n\n"));
    }
    match param_kind(tag, param)? {
        ValueKind::Scenario => Some(match model.resolve_storage(value) {
            Some(file) => format!("scenario file `{}`", file.path(db)),
            None => format!("⚠ scenario file `{value}` not found"),
        }),
        ValueKind::Asset(kind) => Some(match model.resolve_asset(kind, value) {
            Some(path) => format!("{} asset `{path}`", kind.dir_name()),
            None => format!("⚠ {} asset `{value}` not found", kind.dir_name()),
        }),
        _ => None,
    }
}

fn is_dynamic(value: &str) -> bool {
    matches!(value.trim_start().chars().next(), Some('&' | '%'))
}

fn dynamic_reference_name<'a>(name: &'a str, resolution: &'a ProjectResolution) -> Option<&'a str> {
    if is_dynamic(name) {
        return Some(name);
    }
    match resolution {
        ProjectResolution::FileNotFound { storage } if is_dynamic(storage) => Some(storage),
        _ => None,
    }
}

fn describe_dynamic_reference(
    db: &dyn ProjectDb,
    model: &SemanticModel<'_>,
    value: &str,
    range: TextRange,
    resolution: &ProjectResolution,
) -> Option<String> {
    let dynamic = value.trim_start();
    if dynamic.starts_with('&') {
        return describe_javascript_value(
            db,
            model,
            range,
            matches!(resolution, ProjectResolution::FileNotFound { .. }),
        );
    }
    let reference = dynamic.strip_prefix('%')?;
    Some(describe_macro_parameter_reference(reference))
}

fn describe_javascript_value(
    db: &dyn ProjectDb,
    model: &SemanticModel<'_>,
    range: TextRange,
    resolve_scenarios: bool,
) -> Option<String> {
    let candidates = value_candidates_in_range(db, model.file(), range)?;
    let mut out = describe_candidates(&candidates);
    if resolve_scenarios {
        let resolved = candidates
            .string_values()
            .map(|storage| match model.resolve_storage(storage) {
                Some(file) => format!("`{storage}` → `{}`", file.path(db)),
                None => format!("`{storage}` → ⚠ not found"),
            })
            .collect::<Vec<_>>();
        if !resolved.is_empty() {
            out.push_str("\n\n**Scenario candidates**\n\n");
            out.push_str(&resolved.join("\n\n"));
        }
    }
    Some(out)
}

fn describe_macro_parameter_reference(reference: &str) -> String {
    let (name, fallback) = reference
        .split_once('|')
        .map_or((reference, None), |(name, fallback)| (name, Some(fallback)));
    let mut out = format!("macro parameter `mp.{}`", name.trim());
    if let Some(fallback) = fallback {
        let _ = write!(out, " (fallback `{}`)", fallback.trim());
    }
    out
}

fn describe_candidates(candidates: &tyrano_semantic::ValueCandidates) -> String {
    let values = candidates
        .values
        .iter()
        .map(render_scalar)
        .collect::<Vec<_>>();
    let mut out = if values.is_empty() {
        "JavaScript expression".to_owned()
    } else {
        format!("JavaScript candidate value(s): {}", values.join(", "))
    };
    let _ = write!(out, "\n\nInferred type: {}", js_type_name(candidates.types));
    if candidates.unknown {
        out.push_str(" (additional unresolved values are possible)");
    }
    out
}

fn render_scalar(value: &ScalarValue) -> String {
    match value {
        ScalarValue::Undefined => "`undefined`".to_owned(),
        ScalarValue::Null => "`null`".to_owned(),
        ScalarValue::Boolean(value) => format!("`{value}`"),
        ScalarValue::Number(value) => format!("`{value}`"),
        ScalarValue::String(value) => format!("`{:?}`", value),
    }
}

/// The declared kind of `param` on builtin `tag` (with the universal
/// `storage`-on-jump special case handled by the registry itself).
fn param_kind(tag: &str, param: &str) -> Option<ValueKind> {
    Some(builtin_registry().get(tag)?.param(param)?.kind)
}

fn kind_name(kind: ValueKind) -> String {
    match kind {
        ValueKind::Any => "any value".to_string(),
        ValueKind::Label => "`*label` reference".to_string(),
        ValueKind::Scenario => "scenario file".to_string(),
        ValueKind::Asset(kind) => format!("{} asset", kind_dir(kind)),
        ValueKind::Number => "number".to_string(),
        ValueKind::Boolean => "boolean".to_string(),
        ValueKind::Color => "color".to_string(),
        ValueKind::Expression => "JavaScript expression".to_string(),
        ValueKind::VariableName => "game-variable path".to_string(),
        ValueKind::Enum(words) => format!("one of {}", words.join(" | ")),
        ValueKind::Text => "text".to_string(),
    }
}

fn kind_dir(kind: AssetKind) -> &'static str {
    kind.dir_name()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::testutil::{file, offset, project};

    fn two_files() -> tyrano_project::ProjectDatabase {
        project(&[
            (
                "data/scenario/main.ks",
                "*start\n[jump storage=scene2.ks target=*top]\n[greet]\n[macro name=greet]hi[endmacro]\n",
            ),
            ("data/scenario/scene2.ks", "*top\ntext\n"),
        ])
    }

    #[test]
    fn hover_on_builtin_tag_shows_doc_and_params() {
        let db = two_files();
        let f = file(&db, "data/scenario/main.ks");
        let hover = hover(&db, f, offset(&db, f, "[jump", 2)).expect("hover on jump");
        assert!(
            hover.markdown.contains("**[jump]** (builtin)"),
            "{}",
            hover.markdown
        );
        assert!(hover.markdown.contains("`storage`"), "{}", hover.markdown);
        assert!(hover.markdown.contains("`target`"), "{}", hover.markdown);
    }

    #[test]
    fn hover_on_cross_file_jump_target_names_the_file() {
        let db = two_files();
        let f = file(&db, "data/scenario/main.ks");
        let hover = hover(&db, f, offset(&db, f, "*top", 2)).expect("hover on target");
        assert!(
            hover
                .markdown
                .contains("label in `data/scenario/scene2.ks`"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn hover_on_macro_call_names_definition_site() {
        let db = two_files();
        let f = file(&db, "data/scenario/main.ks");
        let hover = hover(&db, f, offset(&db, f, "[greet]", 3)).expect("hover on call");
        assert!(
            hover
                .markdown
                .contains("macro defined in `data/scenario/main.ks`"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn hover_on_cross_file_macro_call() {
        let db = project(&[
            ("data/scenario/a.ks", "[only_b]\n"),
            ("data/scenario/b.ks", "[macro name=only_b][endmacro]\n"),
        ]);
        let f = file(&db, "data/scenario/a.ks");
        let hover = hover(&db, f, offset(&db, f, "[only_b]", 3)).expect("cross-file macro");
        assert!(
            hover
                .markdown
                .contains("macro defined in `data/scenario/b.ks`"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn hover_on_storage_value_resolves_the_file() {
        let db = two_files();
        let f = file(&db, "data/scenario/main.ks");
        let hover = hover(&db, f, offset(&db, f, "scene2.ks target", 4)).expect("storage value");
        assert!(
            hover
                .markdown
                .contains("scenario file `data/scenario/scene2.ks`"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn hover_on_missing_storage_warns() {
        let db = project(&[("data/scenario/main.ks", "[jump storage=gone.ks]\n")]);
        let f = file(&db, "data/scenario/main.ks");
        let hover = hover(&db, f, offset(&db, f, "gone.ks", 3)).expect("missing storage");
        assert!(hover.markdown.contains("not found"), "{}", hover.markdown);
    }

    #[test]
    fn hover_on_dynamic_storage_shows_javascript_candidates() {
        let db = project(&[
            (
                "data/scenario/main.ks",
                "[iscript]\ntf.next = 'scene2.ks';\n[endscript]\n[jump storage=\"&tf.next\"]\n",
            ),
            ("data/scenario/scene2.ks", "*start\n"),
        ]);
        let f = file(&db, "data/scenario/main.ks");
        let hover = hover(&db, f, offset(&db, f, "&tf.next", 3)).expect("dynamic storage");
        assert!(
            hover
                .markdown
                .contains("JavaScript candidate value(s): `\"scene2.ks\"`"),
            "{}",
            hover.markdown
        );
        assert!(
            hover
                .markdown
                .contains("`scene2.ks` → `data/scenario/scene2.ks`"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn hover_on_label_definition() {
        let db = two_files();
        let f = file(&db, "data/scenario/main.ks");
        let hover = hover(&db, f, offset(&db, f, "*start", 2)).expect("label def");
        assert!(
            hover.markdown.contains("label in `data/scenario/main.ks`"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn hover_on_plain_text_is_none() {
        let db = project(&[("data/scenario/main.ks", "*a\nこんにちは\n")]);
        let f = file(&db, "data/scenario/main.ks");
        assert_eq!(hover(&db, f, offset(&db, f, "こん", 0)), None);
    }
}
