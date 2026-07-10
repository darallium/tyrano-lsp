//! Hover: markdown documentation for the construct under the cursor.

use std::fmt::Write as _;

use tyrano_parser_core::{RefKind, SymbolKind, semantic_index};
use tyrano_project::registry::{GLOBAL_PARAMS, TagSpec, ValueKind, builtin_registry};
use tyrano_project::{AssetKind, File, ProjectDb};
use tyrano_semantic::{ProjectResolution, SemanticModel, TagResolution};
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
    let model = SemanticModel::new(db, file);
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
                RefKind::JumpTarget => describe_jump(db, &use_.name, resolution),
                RefKind::MacroCall => describe_tag(db, &model, &use_.name)?,
                RefKind::CharacterRef => format!("**#{}** — character reference", use_.name),
            };
            Some(HoverResult { markdown, range })
        }
        CursorTarget::Tag { name, range } => {
            Some(HoverResult { markdown: describe_tag(db, &model, &name)?, range })
        }
        CursorTarget::ParamName { tag, name, range } => {
            let markdown = describe_param(&tag, &name)?;
            Some(HoverResult { markdown, range })
        }
        CursorTarget::ParamValue { tag, param, value, range } => {
            let markdown = describe_value(db, &model, &tag, &param, &value)?;
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
fn describe_tag(db: &dyn ProjectDb, model: &SemanticModel<'_>, name: &str) -> Option<String> {
    match model.resolve_tag(name) {
        TagResolution::Builtin(spec) => Some(builtin_doc(spec)),
        TagResolution::Macro(def) => {
            Some(format!("**[{}]** — macro defined in `{}`", def.name, def.file.path(db)))
        }
        TagResolution::Unknown => None,
    }
}

fn builtin_doc(spec: &TagSpec) -> String {
    let mut out = format!("**[{}]** (builtin)\n\n{}", spec.name, spec.doc);
    if !spec.params.is_empty() {
        out.push_str("\n\n**Parameters**\n");
        for p in spec.params {
            let requirement = if p.required { "required" } else { "optional" };
            let _ = write!(out, "\n- `{}` — {}, {requirement}", p.name, kind_name(p.kind));
            if let Some(default) = p.default {
                let _ = write!(out, " (default `{default}`)");
            }
        }
    }
    out
}

fn describe_param(tag: &str, name: &str) -> Option<String> {
    if GLOBAL_PARAMS.contains(&name) {
        return Some(format!(
            "`{name}` — universal parameter (JavaScript condition; the tag runs only when it is truthy)"
        ));
    }
    let spec = builtin_registry().get(tag)?;
    let param = spec.param(name)?;
    let requirement = if param.required { "required" } else { "optional" };
    let mut out =
        format!("`{name}` — parameter of **[{tag}]**: {}, {requirement}", kind_name(param.kind));
    if let Some(default) = param.default {
        let _ = write!(out, ", default `{default}`");
    }
    Some(out)
}

fn describe_value(
    db: &dyn ProjectDb,
    model: &SemanticModel<'_>,
    tag: &str,
    param: &str,
    value: &str,
) -> Option<String> {
    // Dynamic values are never resolved statically.
    if value.starts_with('&') || value.starts_with('%') {
        return None;
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
        assert!(hover.markdown.contains("**[jump]** (builtin)"), "{}", hover.markdown);
        assert!(hover.markdown.contains("`storage`"), "{}", hover.markdown);
        assert!(hover.markdown.contains("`target`"), "{}", hover.markdown);
    }

    #[test]
    fn hover_on_cross_file_jump_target_names_the_file() {
        let db = two_files();
        let f = file(&db, "data/scenario/main.ks");
        let hover = hover(&db, f, offset(&db, f, "*top", 2)).expect("hover on target");
        assert!(
            hover.markdown.contains("label in `data/scenario/scene2.ks`"),
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
            hover.markdown.contains("macro defined in `data/scenario/main.ks`"),
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
            hover.markdown.contains("macro defined in `data/scenario/b.ks`"),
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
            hover.markdown.contains("scenario file `data/scenario/scene2.ks`"),
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
    fn hover_on_label_definition() {
        let db = two_files();
        let f = file(&db, "data/scenario/main.ks");
        let hover = hover(&db, f, offset(&db, f, "*start", 2)).expect("label def");
        assert!(hover.markdown.contains("label in `data/scenario/main.ks`"), "{}", hover.markdown);
    }

    #[test]
    fn hover_on_plain_text_is_none() {
        let db = project(&[("data/scenario/main.ks", "*a\nこんにちは\n")]);
        let f = file(&db, "data/scenario/main.ks");
        assert_eq!(hover(&db, f, offset(&db, f, "こん", 0)), None);
    }
}
