//! What is under the cursor.
//!
//! Classification consults the semantic index first (definition names,
//! recorded references) and falls back to the CST for constructs the
//! index deliberately does not record (builtin tag names, cross-file
//! macro calls, parameter names and values).

use tyrano_parser_core::{DefinitionId, semantic_index};
use tyrano_project::{File, ProjectDb};
use tyrano_syntax::SyntaxKind;
use tyrano_syntax::ast::{AnyTag, AstNode as _, Param, ParamValue, TagName};
use tyrano_syntax::text::{TextRange, TextSize};

/// The semantic construct at one byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorTarget {
    /// A definition's name (label `*name`, `[macro name=…]`, character).
    Def { def: DefinitionId, range: TextRange },
    /// A recorded reference; `index` is the position in the file's
    /// `uses()` / `resolved_references()` arrays (document order).
    Use { index: usize, range: TextRange },
    /// A tag name with no recorded use: a builtin or a cross-file macro.
    Tag { name: String, range: TextRange },
    /// A parameter name inside tag `tag`.
    ParamName { tag: String, name: String, range: TextRange },
    /// A parameter value inside tag `tag`. `value` is the cooked value.
    ParamValue { tag: String, param: String, value: String, range: TextRange },
    /// Nothing semantic (plain text, comments, whitespace).
    None,
}

/// Classifies the construct at `offset` in `file`.
pub fn classify(db: &dyn ProjectDb, file: File, offset: TextSize) -> CursorTarget {
    let index = semantic_index(db, file.source(db));

    for symbol in index.symbols() {
        for &def in &symbol.defs {
            let range = index.definition(def).name_range;
            if range.contains_inclusive(offset) && !range.is_empty() {
                return CursorTarget::Def { def, range };
            }
        }
    }

    for (i, use_) in index.uses().iter().enumerate() {
        if use_.range.contains_inclusive(offset) && !use_.range.is_empty() {
            return CursorTarget::Use { index: i, range: use_.range };
        }
    }

    classify_cst(db, file, offset)
}

/// CST fallback: tag names, parameter names, parameter values.
fn classify_cst(db: &dyn ProjectDb, file: File, offset: TextSize) -> CursorTarget {
    let module = tyrano_db::parsed_module(db, file.source(db));
    let root = module.syntax();
    let Some(node) = root.find_node_at_offset(offset) else {
        return CursorTarget::None;
    };

    for anc in node.ancestors() {
        match anc.kind() {
            SyntaxKind::TAG_NAME => {
                let Some(name) = TagName::cast(anc.clone()) else { continue };
                let Some(token) = name.token() else { continue };
                let Some(tag) = anc.parent().and_then(AnyTag::cast) else { continue };
                return CursorTarget::Tag { name: tag.name(), range: token.text_range() };
            }
            SyntaxKind::PARAM => {
                let Some(param) = Param::cast(anc.clone()) else { continue };
                return classify_param(db, file, &param, offset);
            }
            SyntaxKind::PARAM_VALUE => {
                let Some(param) = anc.parent().and_then(Param::cast) else { continue };
                return classify_param(db, file, &param, offset);
            }
            _ => {}
        }
    }
    CursorTarget::None
}

fn classify_param(
    db: &dyn ProjectDb,
    file: File,
    param: &Param,
    offset: TextSize,
) -> CursorTarget {
    let Some(tag) = param.syntax().parent().and_then(AnyTag::cast) else {
        return CursorTarget::None;
    };
    let tag_name = tag.name();

    if let Some(value) = param.value_node() {
        let range = value_range(&value);
        if range.contains_inclusive(offset) {
            let opts = file.source(db).interpret_options(db);
            return CursorTarget::ParamValue {
                tag: tag_name,
                param: param.name(),
                value: value.cooked(&opts),
                range,
            };
        }
    }

    CursorTarget::ParamName {
        tag: tag_name,
        name: param.name(),
        range: param
            .syntax()
            .children_with_tokens()
            .find(|e| e.kind() == SyntaxKind::IDENT)
            .map(|e| e.text_range())
            .unwrap_or_else(|| param.syntax().text_range()),
    }
}

/// The value's token range (falls back to the node range).
fn value_range(value: &ParamValue) -> TextRange {
    value.token().map(|t| t.text_range()).unwrap_or_else(|| value.syntax().text_range())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::testutil::{file, offset, project};

    const MAIN: &str = "\
*start
[jump storage=scene2.ks target=*top]
[greet]
[macro name=greet]hi[endmacro]
[bg storage=room.jpg time=100]
";

    #[test]
    fn label_definition_name() {
        let db = project(&[("data/scenario/main.ks", MAIN)]);
        let f = file(&db, "data/scenario/main.ks");
        match classify(&db, f, offset(&db, f, "start", 2)) {
            CursorTarget::Def { .. } => {}
            other => panic!("expected Def, got {other:?}"),
        }
    }

    #[test]
    fn jump_target_is_a_use() {
        let db = project(&[
            ("data/scenario/main.ks", MAIN),
            ("data/scenario/scene2.ks", "*top\n"),
        ]);
        let f = file(&db, "data/scenario/main.ks");
        match classify(&db, f, offset(&db, f, "*top", 2)) {
            CursorTarget::Use { index: 0, .. } => {}
            other => panic!("expected Use 0, got {other:?}"),
        }
    }

    #[test]
    fn local_macro_call_is_a_use() {
        let db = project(&[("data/scenario/main.ks", MAIN)]);
        let f = file(&db, "data/scenario/main.ks");
        match classify(&db, f, offset(&db, f, "[greet]", 3)) {
            CursorTarget::Use { .. } => {}
            other => panic!("expected Use, got {other:?}"),
        }
    }

    #[test]
    fn builtin_tag_name_falls_back_to_cst() {
        let db = project(&[("data/scenario/main.ks", MAIN)]);
        let f = file(&db, "data/scenario/main.ks");
        match classify(&db, f, offset(&db, f, "[jump", 2)) {
            CursorTarget::Tag { name, .. } => assert_eq!(name, "jump"),
            other => panic!("expected Tag, got {other:?}"),
        }
    }

    #[test]
    fn param_name_and_value() {
        let db = project(&[("data/scenario/main.ks", MAIN)]);
        let f = file(&db, "data/scenario/main.ks");
        match classify(&db, f, offset(&db, f, "storage=scene2.ks", 3)) {
            CursorTarget::ParamName { tag, name, .. } => {
                assert_eq!(tag, "jump");
                assert_eq!(name, "storage");
            }
            other => panic!("expected ParamName, got {other:?}"),
        }
        match classify(&db, f, offset(&db, f, "scene2.ks target", 4)) {
            CursorTarget::ParamValue { tag, param, value, .. } => {
                assert_eq!(tag, "jump");
                assert_eq!(param, "storage");
                assert_eq!(value, "scene2.ks");
            }
            other => panic!("expected ParamValue, got {other:?}"),
        }
    }

    #[test]
    fn plain_text_is_none() {
        let db = project(&[("data/scenario/main.ks", "*a\nこんにちは\n")]);
        let f = file(&db, "data/scenario/main.ks");
        assert_eq!(classify(&db, f, offset(&db, f, "こん", 3)), CursorTarget::None);
    }
}
