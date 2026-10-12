//! Completion: tag names, parameter names, and parameter values.
//!
//! Context detection works on the raw line text rather than the CST — a
//! half-typed tag (`[jump tar`) routinely parses as an error line, but its
//! text is still perfectly analyzable.

use tyrano_parser_core::SymbolKind;
use tyrano_project::registry::{GLOBAL_PARAMS, ValueKind, builtin_registry};
use tyrano_project::{File, ProjectDb};
use tyrano_semantic::{
    SemanticModel, TagResolution, macro_signature, project_javascript_tag_names,
};
use tyrano_syntax::text::TextSize;

/// What a completion item is, mapped to an LSP kind by the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    /// A builtin tag.
    Tag,
    /// A macro (project-wide or file-local).
    Macro,
    /// A `*label` target.
    Label,
    /// A scenario file.
    File,
    /// An asset file.
    Asset,
    /// A parameter name.
    Param,
    /// A fixed enum word / boolean.
    Value,
}

/// One completion suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
}

/// The syntactic completion context at one offset, derived from the line
/// prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Context {
    /// Typing a tag name (after `[` or a line-leading `@`).
    TagName,
    /// Typing a parameter name inside tag `tag`.
    ParamName {
        tag: String,
    },
    /// Typing the value of `param` inside tag `tag`; `params` holds the
    /// already-complete `name=value` pairs of the same tag (for
    /// `storage=`-dependent label completion).
    ParamValue {
        tag: String,
        param: String,
        params: Vec<(String, String)>,
    },
    None,
}

/// Completion items at `offset` in `file`.
pub fn completions(db: &dyn ProjectDb, file: File, offset: TextSize) -> Vec<CompletionItem> {
    if file.path(db).extension() == Some("js") {
        return Vec::new();
    }
    let text = file.source(db).text(db);
    let offset = offset.min(TextSize::of(text));
    let arguments = completion_arguments(db, file, text, offset);
    match context(text, usize::from(offset)) {
        Context::TagName => tag_completions(db, file),
        Context::ParamName { tag } => param_completions(db, file, &tag, &arguments),
        Context::ParamValue { tag, param, params } => {
            let mut arguments = arguments.clone();
            arguments.remove(&param);
            let params = if builtin_registry().get(&tag).is_some() {
                params
            } else {
                arguments.into_iter().collect()
            };
            value_completions(db, file, &tag, &param, &params)
        }
        Context::None => Vec::new(),
    }
}

/// Builtin tags plus every macro visible from `file`.
fn tag_completions(db: &dyn ProjectDb, file: File) -> Vec<CompletionItem> {
    let mut out: Vec<CompletionItem> = builtin_registry()
        .names()
        .map(|name| CompletionItem {
            label: name.to_string(),
            kind: CompletionKind::Tag,
            detail: builtin_registry().get(name).map(|s| s.doc.to_string()),
        })
        .collect();

    let index = tyrano_parser_core::semantic_index(db, file.source(db));
    for name in project_javascript_tag_names(db, db.project()).iter() {
        out.push(CompletionItem {
            label: name.clone(),
            kind: CompletionKind::Tag,
            detail: Some("custom tag — reachable plugin JavaScript".to_string()),
        });
    }
    let mut macros: Vec<(String, String)> = tyrano_project::project_macros(db)
        .iter()
        .map(|(name, def)| (name.to_string(), def.file.path(db).to_string()))
        .collect();
    for symbol in index.symbols() {
        if symbol.kind == SymbolKind::Macro {
            macros.push((symbol.name.clone(), file.path(db).to_string()));
        }
    }
    macros.sort();
    macros.dedup();
    for (name, path) in macros {
        if builtin_registry().get(&name).is_some() {
            continue; // builtins shadow same-named macros
        }
        out.push(CompletionItem {
            label: name,
            kind: CompletionKind::Macro,
            detail: Some(format!("macro — {path}")),
        });
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out.dedup_by(|a, b| a.label == b.label);
    out
}

/// Parameter names of `tag` (builtin spec + universal parameters).
fn param_completions(
    db: &dyn ProjectDb,
    file: File,
    tag: &str,
    arguments: &std::collections::BTreeMap<String, String>,
) -> Vec<CompletionItem> {
    let mut out = Vec::new();
    if let Some(spec) = builtin_registry().get(tag) {
        for p in spec.params {
            let requirement = if p.required { "required" } else { "optional" };
            out.push(CompletionItem {
                label: p.name.to_string(),
                kind: CompletionKind::Param,
                detail: Some(requirement.to_string()),
            });
        }
        if let Some(implementation) =
            tyrano_semantic::project_javascript_tag_spec(db, db.project(), tag)
        {
            for parameter in implementation
                .parameters
                .into_iter()
                .filter(|parameter| spec.param(&parameter.name).is_none())
            {
                let requirement = if parameter.required {
                    "required"
                } else {
                    "optional"
                };
                out.push(CompletionItem {
                    label: parameter.name,
                    kind: CompletionKind::Param,
                    detail: Some(format!("master_tag implementation, {requirement}")),
                });
            }
        }
    } else {
        match SemanticModel::new(db, file).resolve_tag(tag) {
            TagResolution::Macro(definition) => {
                if let Some(signature) = macro_signature(db, &definition) {
                    let selected = super::macro_call::select(&signature, arguments);
                    for (name, detail) in super::macro_call::parameter_docs(&selected, None) {
                        out.push(CompletionItem {
                            label: name,
                            kind: CompletionKind::Param,
                            detail: Some(detail),
                        });
                    }
                    out.push(CompletionItem {
                        label: "*".to_owned(),
                        kind: CompletionKind::Param,
                        detail: Some("forward every caller mp parameter".to_owned()),
                    });
                }
            }
            TagResolution::Plugin(spec) => {
                for parameter in spec.parameters {
                    let requirement = if parameter.required {
                        "required"
                    } else {
                        "optional"
                    };
                    out.push(CompletionItem {
                        label: parameter.name,
                        kind: CompletionKind::Param,
                        detail: Some(format!("master_tag parameter, {requirement}")),
                    });
                }
            }
            TagResolution::Builtin(_) | TagResolution::Unknown => return Vec::new(),
        }
    }
    for name in GLOBAL_PARAMS {
        out.push(CompletionItem {
            label: name.to_string(),
            kind: CompletionKind::Param,
            detail: Some("universal (condition)".to_string()),
        });
    }
    merge_completions(out)
}

/// Values for `param` of `tag`: labels, scenario files, assets, enums.
fn value_completions(
    db: &dyn ProjectDb,
    file: File,
    tag: &str,
    param: &str,
    params: &[(String, String)],
) -> Vec<CompletionItem> {
    if let Some(spec) = builtin_registry().get(tag) {
        return spec
            .param(param)
            .map(|p| kind_completions(db, file, p.kind, params))
            .unwrap_or_default();
    }
    let TagResolution::Macro(definition) = SemanticModel::new(db, file).resolve_tag(tag) else {
        return Vec::new();
    };
    let Some(signature) = macro_signature(db, &definition) else {
        return Vec::new();
    };
    let arguments = params.iter().cloned().collect();
    let selected = super::macro_call::select(&signature, &arguments);
    let mut out = Vec::new();
    let mut add = |parameters: &[tyrano_semantic::MacroParameterSpec],
                   condition: Option<String>| {
        for p in parameters.iter().filter(|p| p.name == param) {
            let mut kinds = p.kinds.clone();
            if p.js_types == tyrano_js_analysis::JsTypeSet::BOOLEAN
                && !kinds.contains(&ValueKind::Boolean)
            {
                kinds.push(ValueKind::Boolean);
            }
            for kind in kinds {
                for mut item in kind_completions(db, file, kind, params) {
                    if let Some(c) = &condition {
                        item.detail = Some(format!(
                            "{}when {c}",
                            item.detail.map(|d| format!("{d}; ")).unwrap_or_default()
                        ));
                    }
                    out.push(item);
                }
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
    for branch in &selected.pending {
        for c in branch.conditions.iter().filter(|c| c.parameter == param) {
            out.push(CompletionItem {
                label: c.equals.clone(),
                kind: CompletionKind::Value,
                detail: Some(super::macro_call::conditions(&branch.conditions)),
            });
        }
    }
    merge_completions(out)
}

fn kind_completions(
    db: &dyn ProjectDb,
    file: File,
    kind: ValueKind,
    params: &[(String, String)],
) -> Vec<CompletionItem> {
    let model = SemanticModel::new(db, file);
    let settings = db.project().settings(db);

    match kind {
        ValueKind::Label => {
            // Labels of the `storage=` file when one is already written on
            // this tag, otherwise labels of the current file.
            let target_file = params
                .iter()
                .find(|(name, _)| name == "storage")
                .and_then(|(_, value)| model.resolve_storage(value))
                .unwrap_or(file);
            let index = tyrano_parser_core::semantic_index(db, target_file.source(db));
            let mut labels: Vec<&str> = index
                .symbols()
                .iter()
                .filter(|s| s.kind == SymbolKind::Label)
                .map(|s| s.name.as_str())
                .collect();
            labels.sort();
            labels
                .into_iter()
                .map(|name| CompletionItem {
                    label: format!("*{name}"),
                    kind: CompletionKind::Label,
                    detail: Some(target_file.path(db).to_string()),
                })
                .collect()
        }
        ValueKind::Scenario => db
            .project()
            .scenario_files(db)
            .iter()
            .map(|f| {
                let path = f.path(db);
                let short = settings
                    .scenario_roots
                    .iter()
                    .find_map(|root| path.strip_prefix(root))
                    .unwrap_or(path.as_str());
                CompletionItem {
                    label: short.to_string(),
                    kind: CompletionKind::File,
                    detail: Some(path.to_string()),
                }
            })
            .collect(),
        ValueKind::Asset(kind) => {
            let index = db.project().asset_index(db);
            let roots = settings.asset_roots.get(&kind);
            index
                .of_kind(kind)
                .map(|path| {
                    let short = roots
                        .into_iter()
                        .flatten()
                        .find_map(|root| path.strip_prefix(root))
                        .unwrap_or(path.as_str());
                    CompletionItem {
                        label: short.to_string(),
                        kind: CompletionKind::Asset,
                        detail: Some(path.to_string()),
                    }
                })
                .collect()
        }
        ValueKind::Enum(words) => words
            .iter()
            .map(|w| CompletionItem {
                label: w.to_string(),
                kind: CompletionKind::Value,
                detail: None,
            })
            .collect(),
        ValueKind::Boolean => ["true", "false"]
            .into_iter()
            .map(|w| CompletionItem {
                label: w.to_string(),
                kind: CompletionKind::Value,
                detail: None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn merge_completions(items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    let mut merged = std::collections::BTreeMap::<String, CompletionItem>::new();
    for item in items {
        if let Some(previous) = merged.get_mut(&item.label) {
            if let Some(detail) = item.detail {
                let existing = previous.detail.get_or_insert_with(String::new);
                if !existing.split("; ").any(|part| part == detail) {
                    if !existing.is_empty() {
                        existing.push_str("; ");
                    }
                    existing.push_str(&detail);
                }
            }
        } else {
            merged.insert(item.label.clone(), item);
        }
    }
    merged.into_values().collect()
}

fn completion_arguments(
    db: &dyn ProjectDb,
    file: File,
    text: &str,
    offset: TextSize,
) -> std::collections::BTreeMap<String, String> {
    let pos = usize::from(offset);
    let start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    let prefix = &text[start..pos];
    let mut arguments = std::collections::BTreeMap::new();
    if let Some(inner) = tag_start(prefix) {
        let words = split_words(&prefix[inner..]);
        for (word, _) in words.iter().skip(1) {
            if let Some((name, value)) = split_param(word) {
                if value.is_empty() {
                    continue;
                }
                let first = value.chars().next().unwrap();
                if matches!(first, '\'' | '"' | '`') && (value.len() < 2 || !value.ends_with(first))
                {
                    continue;
                }
                let value = unquote(value);
                if value.starts_with('&') || value.starts_with('%') {
                    continue;
                }
                arguments.insert(name.to_owned(), value.to_owned());
            }
        }
    }
    if let Some(cst) = super::macro_call::arguments_at(db, file, offset) {
        arguments.extend(cst);
    }
    arguments
}

// ======================================================================
// Context detection
// ======================================================================

/// Detects the completion context from the raw text before `offset`.
fn context(text: &str, offset: usize) -> Context {
    let line_start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let prefix = &text[line_start..offset];

    let inner_start = match tag_start(prefix) {
        Some(at) => at,
        None => return Context::None,
    };
    let inner = &prefix[inner_start..];

    let words = split_words(inner);
    let Some((tag_word, _)) = words.first() else {
        return Context::TagName; // right after `[` / `@`
    };
    let tag = tag_word.clone();

    // Whitespace between the last word and the cursor starts a fresh
    // parameter name.
    let (last, last_start) = words.last().cloned().expect("words is non-empty");
    if last_start + last.len() != inner.len() {
        return Context::ParamName { tag };
    }

    if words.len() == 1 {
        return Context::TagName; // still typing the tag name
    }

    let params = complete_params(&words[1..words.len() - 1]);
    match split_param(&last) {
        Some((name, _)) => Context::ParamValue {
            tag,
            param: name.to_string(),
            params,
        },
        None => Context::ParamName { tag },
    }
}

/// Byte index just past the `[` / `@` opening the innermost unclosed tag
/// of `prefix`, or `None` when the cursor is not inside a tag.
fn tag_start(prefix: &str) -> Option<usize> {
    let mut open: Option<usize> = prefix.starts_with('@').then_some(1);
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, ch) in prefix.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' | '\'' | '`' => match quote {
                Some(q) if q == ch => quote = None,
                Some(_) => {}
                None => quote = Some(ch),
            },
            '[' if quote.is_none() => open = Some(i + 1),
            ']' if quote.is_none() => {
                // Closing the inline tag; an `@` line stays open.
                open = if prefix.starts_with('@') {
                    Some(1)
                } else {
                    None
                };
            }
            _ => {}
        }
    }
    open
}

/// Splits tag-interior text into whitespace-separated words (quote-aware),
/// keeping each word's start offset.
fn split_words(inner: &str) -> Vec<(String, usize)> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut start = 0usize;
    let mut quote: Option<char> = None;
    for (i, ch) in inner.char_indices() {
        let is_space = ch.is_whitespace() && quote.is_none();
        if is_space {
            if !current.is_empty() {
                words.push((std::mem::take(&mut current), start));
            }
            continue;
        }
        if current.is_empty() {
            start = i;
        }
        if matches!(ch, '"' | '\'' | '`') {
            match quote {
                Some(q) if q == ch => quote = None,
                Some(_) => {}
                None => quote = Some(ch),
            }
        }
        current.push(ch);
    }
    if !current.is_empty() {
        words.push((current, start));
    }
    words
}

/// `name=value` words as `(name, unquoted value)` pairs.
fn complete_params(words: &[(String, usize)]) -> Vec<(String, String)> {
    words
        .iter()
        .filter_map(|(w, _)| split_param(w))
        .map(|(n, v)| (n.to_string(), unquote(v).to_string()))
        .collect()
}

/// Splits one word at its first `=`, if it has one.
fn split_param(word: &str) -> Option<(&str, &str)> {
    let (name, value) = word.split_once('=')?;
    if name.is_empty() {
        None
    } else {
        Some((name, value))
    }
}

/// Strips one layer of (possibly unclosed) quotes.
fn unquote(value: &str) -> &str {
    let mut v = value;
    for q in ['"', '\'', '`'] {
        if let Some(rest) = v.strip_prefix(q) {
            v = rest.strip_suffix(q).unwrap_or(rest);
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::testutil::{file, offset, project};
    use tyrano_project::testing::ProjectBuilder;
    use tyrano_project::{AssetKind, ProjectPath};

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn context_detection() {
        assert_eq!(context("[", 1), Context::TagName);
        assert_eq!(context("[ju", 3), Context::TagName);
        assert_eq!(context("@ju", 3), Context::TagName);
        assert_eq!(
            context("[jump ", 6),
            Context::ParamName {
                tag: "jump".to_string()
            }
        );
        assert_eq!(
            context("[jump tar", 9),
            Context::ParamName {
                tag: "jump".to_string()
            }
        );
        assert_eq!(
            context("[jump target=", 13),
            Context::ParamValue {
                tag: "jump".to_string(),
                param: "target".to_string(),
                params: vec![]
            }
        );
        assert_eq!(
            context("[jump storage=b.ks target=*t", 28),
            Context::ParamValue {
                tag: "jump".to_string(),
                param: "target".to_string(),
                params: vec![("storage".to_string(), "b.ks".to_string())]
            }
        );
        assert_eq!(context("text [l] more", 13), Context::None);
        assert_eq!(context("plain text", 5), Context::None);
        // A closed inline tag on an @ line never happens, but a closed
        // bracket pair inside an @ line stays in the tag.
        assert_eq!(
            context("@jump ", 6),
            Context::ParamName {
                tag: "jump".to_string()
            }
        );
    }

    #[test]
    fn tag_completion_lists_builtins_and_cross_file_macros() {
        let db = project(&[
            ("data/scenario/a.ks", "[\n"),
            ("data/scenario/b.ks", "[macro name=fancy][endmacro]\n"),
        ]);
        let f = file(&db, "data/scenario/a.ks");
        let items = completions(&db, f, offset(&db, f, "[", 1));
        let names = labels(&items);
        assert!(names.contains(&"jump"), "{names:?}");
        assert!(names.contains(&"fancy"), "{names:?}");
        let fancy = items.iter().find(|i| i.label == "fancy").unwrap();
        assert_eq!(fancy.kind, CompletionKind::Macro);
        assert!(
            fancy
                .detail
                .as_deref()
                .unwrap()
                .contains("data/scenario/b.ks")
        );
    }

    #[test]
    fn tag_completion_lists_reachable_plugin_tags() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/a.ks",
                "[loadjs storage=demo.js]\n[custom_tag cond=\n[",
            )
            .javascript(
                "data/others/demo.js",
                "TYRANO.kag.ftag.master_tag.custom_tag = {};",
            )
            .build();
        let f = file(&db, "data/scenario/a.ks");
        let source = f.source(&db).text(&db);
        let items = completions(&db, f, TextSize::of(source));
        assert!(labels(&items).contains(&"custom_tag"));

        let param_at = source.find("[custom_tag ").unwrap() + "[custom_tag ".len();
        let params = completions(&db, f, TextSize::new(u32::try_from(param_at).unwrap()));
        assert!(labels(&params).contains(&"cond"));
    }

    #[test]
    fn param_completion_for_builtin_tag() {
        let db = project(&[("data/scenario/a.ks", "[jump \n")]);
        let f = file(&db, "data/scenario/a.ks");
        let items = completions(&db, f, offset(&db, f, "[jump ", 6));
        let names = labels(&items);
        assert!(names.contains(&"storage"), "{names:?}");
        assert!(names.contains(&"target"), "{names:?}");
        assert!(names.contains(&"cond"), "{names:?}");
    }

    #[test]
    fn target_value_completion_local_labels() {
        let db = project(&[("data/scenario/a.ks", "*intro\n*ending\n[jump target=\n")]);
        let f = file(&db, "data/scenario/a.ks");
        let items = completions(&db, f, offset(&db, f, "target=", 7));
        assert_eq!(labels(&items), ["*ending", "*intro"]);
    }

    #[test]
    fn target_value_completion_uses_storage_file() {
        let db = project(&[
            ("data/scenario/a.ks", "[jump storage=b.ks target=\n"),
            ("data/scenario/b.ks", "*top\n*bottom\n"),
        ]);
        let f = file(&db, "data/scenario/a.ks");
        let items = completions(&db, f, offset(&db, f, "target=", 7));
        assert_eq!(labels(&items), ["*bottom", "*top"]);
        assert_eq!(items[0].detail.as_deref(), Some("data/scenario/b.ks"));
    }

    #[test]
    fn storage_value_completion_lists_scenarios_relative_to_root() {
        let db = project(&[
            ("data/scenario/a.ks", "[jump storage=\n"),
            ("data/scenario/sub/ev.ks", "*x\n"),
        ]);
        let f = file(&db, "data/scenario/a.ks");
        let items = completions(&db, f, offset(&db, f, "storage=", 8));
        assert_eq!(labels(&items), ["a.ks", "sub/ev.ks"]);
    }

    #[test]
    fn asset_value_completion_lists_assets() {
        let db = ProjectBuilder::new()
            .file("data/scenario/a.ks", "[bg storage=\n")
            .asset(AssetKind::BgImage, "room.jpg")
            .asset(AssetKind::Image, "face.png")
            .build();
        let f = db
            .file(&ProjectPath::new("data/scenario/a.ks").unwrap())
            .unwrap();
        let items = completions(&db, f, offset(&db, f, "storage=", 8));
        assert_eq!(
            labels(&items),
            ["room.jpg"],
            "bg only offers bgimage assets"
        );
    }

    #[test]
    fn boolean_value_completion() {
        let db = project(&[("data/scenario/a.ks", "[playbgm loop=\n")]);
        let f = file(&db, "data/scenario/a.ks");
        let items = completions(&db, f, offset(&db, f, "loop=", 5));
        assert_eq!(labels(&items), ["true", "false"]);
    }

    #[test]
    fn no_completion_in_plain_text() {
        let db = project(&[("data/scenario/a.ks", "hello world\n")]);
        let f = file(&db, "data/scenario/a.ks");
        assert!(completions(&db, f, offset(&db, f, "world", 3)).is_empty());
    }
}
