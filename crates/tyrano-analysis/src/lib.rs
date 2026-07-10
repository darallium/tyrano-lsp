//! Semantic model over the lossless TyranoScript CST.
//!
//! Pipeline position: source → lossless CST (`tyrano-syntax`) → typed AST
//! view → **this crate**. [`lower`] projects a parsed scenario into a
//! [`ScenarioModel`]: a flat, document-ordered list of semantic [`Item`]s
//! with a label symbol table, same-file jump resolution, and semantic
//! diagnostics. Every item keeps a [`SyntaxNodePtr`] back into the CST and
//! never owns source text — the CST stays the single source of truth.
//!
//! This layer is deliberately minimal: it is the foundation an LSP
//! (go-to-label, rename, unknown-target diagnostics) would build on, not a
//! full interpreter.

use std::collections::HashMap;

use tyrano_syntax::Parse;
use tyrano_syntax::ast::{self, AstNode as _, InterpretOptions, Line, TextSegment};
use tyrano_syntax::diagnostics::{DiagCode, Diagnostic, SecondaryKind};
use tyrano_syntax::red::SyntaxNodePtr;
use tyrano_syntax::text::{TextRange, TextSize};

/// Tags that perform a same-file jump when `target=*label` is given
/// without a (non-empty) `storage=` parameter.
pub const JUMP_TAGS: &[&str] = &["jump", "call", "link", "button"];

/// One semantic item, in document order.
#[derive(Debug, Clone)]
pub enum Item {
    Label(LabelItem),
    Tag(TagItem),
    Chara(CharaItem),
    Text(TextItem),
    Comment(CommentItem),
    Script(ScriptItem),
    Html(HtmlItem),
}

impl Item {
    /// The item's source range (trivia-inclusive node range).
    pub fn range(&self) -> TextRange {
        match self {
            Item::Label(i) => i.range,
            Item::Tag(i) => i.range,
            Item::Chara(i) => i.range,
            Item::Text(i) => i.range,
            Item::Comment(i) => i.range,
            Item::Script(i) => i.range,
            Item::Html(i) => i.range,
        }
    }

    /// Pointer back into the CST (resolve against the parse's root).
    pub fn ptr(&self) -> &SyntaxNodePtr {
        match self {
            Item::Label(i) => &i.ptr,
            Item::Tag(i) => &i.ptr,
            Item::Chara(i) => &i.ptr,
            Item::Text(i) => &i.ptr,
            Item::Comment(i) => &i.ptr,
            Item::Script(i) => &i.ptr,
            Item::Html(i) => &i.ptr,
        }
    }
}

/// `*name|value` — a jump target definition.
#[derive(Debug, Clone)]
pub struct LabelItem {
    pub name: String,
    pub value: Option<String>,
    /// Range of the name token (diagnostic anchor), falling back to the line.
    pub name_range: TextRange,
    pub ptr: SyntaxNodePtr,
    pub range: TextRange,
}

/// `[tag …]` / `@tag …` with cooked parameter values.
#[derive(Debug, Clone)]
pub struct TagItem {
    pub name: String,
    /// Cooked `(name, value)` pairs in source order; flag parameters and
    /// the macro `*` pass-through have `None` values.
    pub params: Vec<(String, Option<String>)>,
    pub at_notation: bool,
    pub ptr: SyntaxNodePtr,
    pub range: TextRange,
}

impl TagItem {
    pub fn param(&self, name: &str) -> Option<&Option<String>> {
        self.params.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

/// `#name:face`.
#[derive(Debug, Clone)]
pub struct CharaItem {
    pub name: Option<String>,
    pub face: Option<String>,
    pub ptr: SyntaxNodePtr,
    pub range: TextRange,
}

/// One text line's cooked content.
#[derive(Debug, Clone)]
pub struct TextItem {
    pub text: String,
    pub preserve_whitespace: bool,
    pub ptr: SyntaxNodePtr,
    pub range: TextRange,
}

/// `;comment` or `/* … */`.
#[derive(Debug, Clone)]
pub struct CommentItem {
    pub text: String,
    pub is_block: bool,
    pub ptr: SyntaxNodePtr,
    pub range: TextRange,
}

/// `[iscript] … [endscript]`.
#[derive(Debug, Clone)]
pub struct ScriptItem {
    pub code: String,
    pub ptr: SyntaxNodePtr,
    pub range: TextRange,
}

/// `[html] … [endhtml]`.
#[derive(Debug, Clone)]
pub struct HtmlItem {
    pub content: String,
    pub ptr: SyntaxNodePtr,
    pub range: TextRange,
}

/// A `[macro name=…]` definition site.
#[derive(Debug, Clone)]
pub struct MacroDef {
    pub name: String,
    pub range: TextRange,
}

/// The lowered semantic model of one scenario file.
#[derive(Debug, Clone)]
pub struct ScenarioModel {
    items: Vec<Item>,
    /// name → index into `items` of the FIRST definition (engine behavior:
    /// the first label wins).
    labels: HashMap<String, usize>,
    macros: Vec<MacroDef>,
    diagnostics: Vec<Diagnostic>,
}

impl ScenarioModel {
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn labels(&self) -> impl Iterator<Item = &LabelItem> {
        self.items.iter().filter_map(|i| match i {
            Item::Label(l) => Some(l),
            _ => None,
        })
    }

    /// The first definition of `name`, if any.
    pub fn label(&self, name: &str) -> Option<&LabelItem> {
        match self.items.get(*self.labels.get(name)?) {
            Some(Item::Label(l)) => Some(l),
            _ => None,
        }
    }

    pub fn tags(&self) -> impl Iterator<Item = &TagItem> {
        self.items.iter().filter_map(|i| match i {
            Item::Tag(t) => Some(t),
            _ => None,
        })
    }

    pub fn tags_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a TagItem> {
        self.tags().filter(move |t| t.name == name)
    }

    pub fn macros(&self) -> &[MacroDef] {
        &self.macros
    }

    /// Semantic diagnostics only (parse/lex diagnostics live on `Parse`).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The innermost item whose range contains `offset` (inline tags win
    /// over the text line that carries them).
    pub fn item_at(&self, offset: TextSize) -> Option<&Item> {
        self.items
            .iter()
            .filter(|i| i.range().contains(offset))
            .min_by_key(|i| i.range().len())
    }
}

/// Lowers a parse into a [`ScenarioModel`] under the given interpretation
/// options.
pub fn lower(parse: &Parse, opts: &InterpretOptions) -> ScenarioModel {
    let mut model = ScenarioModel {
        items: Vec::new(),
        labels: HashMap::new(),
        macros: Vec::new(),
        diagnostics: Vec::new(),
    };

    for line in parse.ast().lines() {
        lower_line(&line, opts, &mut model);
    }
    resolve_jumps(&mut model);
    model
}

fn ptr_range(node: &tyrano_syntax::red::SyntaxNode) -> (SyntaxNodePtr, TextRange) {
    (SyntaxNodePtr::new(node), node.text_range())
}

fn lower_line(line: &Line, opts: &InterpretOptions, model: &mut ScenarioModel) {
    match line {
        Line::Label(l) => {
            let (ptr, range) = ptr_range(l.syntax());
            let name = l.name().unwrap_or_default();
            let name_range = l.name_token().map_or(range, |t| t.text_range());
            let item = LabelItem { name: name.clone(), value: l.value(opts), name_range, ptr, range };
            if let Some(&first) = model.labels.get(&name) {
                let first_range = match &model.items[first] {
                    Item::Label(f) => f.name_range,
                    _ => range,
                };
                model.diagnostics.push(
                    Diagnostic::new(DiagCode::SemDuplicateLabel, name_range)
                        .with_secondary(first_range, SecondaryKind::FirstDefinedHere)
                        .with_arg("name", name.clone()),
                );
            } else {
                model.labels.insert(name, model.items.len());
            }
            model.items.push(Item::Label(item));
        }
        Line::Chara(c) => {
            let (ptr, range) = ptr_range(c.syntax());
            model.items.push(Item::Chara(CharaItem {
                name: c.name(),
                face: c.face(opts),
                ptr,
                range,
            }));
        }
        Line::Text(t) => {
            let (ptr, range) = ptr_range(t.syntax());
            let text = t.cooked_text();
            // A line consisting solely of inline tags carries no text
            // item (matches the engine, which emits only the tags).
            if !text.is_empty() || t.preserves_whitespace() {
                model.items.push(Item::Text(TextItem {
                    text,
                    preserve_whitespace: t.preserves_whitespace(),
                    ptr,
                    range,
                }));
            }
            for seg in t.segments() {
                if let TextSegment::Tag(tag) = seg {
                    lower_tag(&tag, false, opts, model);
                }
            }
        }
        Line::AtTag(t) => lower_tag(t, true, opts, model),
        Line::Comment(c) => {
            let (ptr, range) = ptr_range(c.syntax());
            model.items.push(Item::Comment(CommentItem {
                text: c.text().unwrap_or_default(),
                is_block: false,
                ptr,
                range,
            }));
        }
        Line::BlockComment(c) => {
            let (ptr, range) = ptr_range(c.syntax());
            model.items.push(Item::Comment(CommentItem {
                text: c.text_lines().join("\n"),
                is_block: true,
                ptr,
                range,
            }));
        }
        Line::IScript(s) => {
            let (ptr, range) = ptr_range(s.syntax());
            model.items.push(Item::Script(ScriptItem { code: s.code(), ptr, range }));
        }
        Line::Html(h) => {
            let (ptr, range) = ptr_range(h.syntax());
            model.items.push(Item::Html(HtmlItem { content: h.code(), ptr, range }));
        }
        // Error lines carry no semantics; parse diagnostics cover them.
        Line::Error(_) => {}
    }
}

fn lower_tag(tag: &impl ast::Tag, at_notation: bool, opts: &InterpretOptions, model: &mut ScenarioModel) {
    let (ptr, range) = ptr_range(tag.syntax());
    let params: Vec<(String, Option<String>)> =
        tag.params().iter().map(|p| (p.name(), p.cooked_value(opts))).collect();
    let item = TagItem { name: tag.name(), params, at_notation, ptr, range };
    if item.name == "macro"
        && let Some(Some(name)) = item.param("name")
    {
        model.macros.push(MacroDef { name: name.clone(), range });
    }
    model.items.push(Item::Tag(item));
}

/// Same-file `target=*label` resolution for [`JUMP_TAGS`]. A tag with a
/// non-empty `storage=` parameter jumps into another file and is skipped.
fn resolve_jumps(model: &mut ScenarioModel) {
    let mut diags = Vec::new();
    for item in &model.items {
        let Item::Tag(tag) = item else { continue };
        if !JUMP_TAGS.contains(&tag.name.as_str()) {
            continue;
        }
        let Some(Some(target)) = tag.param("target") else { continue };
        let Some(label_name) = target.strip_prefix('*') else { continue };
        if let Some(Some(storage)) = tag.param("storage")
            && !storage.is_empty()
        {
            continue;
        }
        if model.labels.contains_key(label_name) {
            continue;
        }
        diags.push(
            Diagnostic::new(DiagCode::SemUnknownLabel, target_range(tag))
                .with_arg("name", label_name.to_string()),
        );
    }
    model.diagnostics.extend(diags);
    model.diagnostics.sort_by_key(|d| (d.primary.start(), d.primary.end()));
}

/// Best-effort range of the `target` parameter's value token; falls back
/// to the tag range. (The TagItem stores cooked values only, so we do not
/// re-walk the CST here — the tag range is a serviceable diagnostic
/// anchor and keeps the model self-contained.)
fn target_range(tag: &TagItem) -> TextRange {
    tag.range
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_syntax::parse;

    const SRC: &str = "\
*start|開始\n\
#akane:happy\n\
こんにちは[l]世界\n\
@bg storage=room.jpg time=1000\n\
;メモ\n\
[iscript]\n\
var a = 1;\n\
[endscript]\n\
[jump target=*end]\n\
[macro name=greet]\n\
*end\n";

    fn model(src: &str) -> ScenarioModel {
        lower(&parse(src), &InterpretOptions::default())
    }

    #[test]
    fn lowering_shape_in_document_order() {
        let m = model(SRC);
        let kinds: Vec<&str> = m
            .items()
            .iter()
            .map(|i| match i {
                Item::Label(_) => "label",
                Item::Tag(_) => "tag",
                Item::Chara(_) => "chara",
                Item::Text(_) => "text",
                Item::Comment(_) => "comment",
                Item::Script(_) => "script",
                Item::Html(_) => "html",
            })
            .collect();
        assert_eq!(
            kinds,
            // text line lowers to Text + its inline [l] tag
            ["label", "chara", "text", "tag", "tag", "comment", "script", "tag", "tag", "label"]
        );
        assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());

        // Pointers resolve back into the CST.
        let root = tyrano_syntax::red::SyntaxNode::new_root(parse(SRC).green().clone());
        for item in m.items() {
            let node = item.ptr().resolve(&root).expect("ptr must resolve");
            assert_eq!(node.text_range(), item.range());
        }
    }

    #[test]
    fn labels_and_first_definition_wins() {
        let m = model("*a|one\ntext\n*a|two\n*b\n");
        assert_eq!(m.labels().count(), 3);
        assert_eq!(m.label("a").unwrap().value.as_deref(), Some("one"));
        let dup: Vec<_> = m
            .diagnostics()
            .iter()
            .filter(|d| matches!(d.code, DiagCode::SemDuplicateLabel))
            .collect();
        assert_eq!(dup.len(), 1);
        assert_eq!(dup[0].secondary.len(), 1);
        assert!(dup[0].secondary[0].0.start() < dup[0].primary.start());
    }

    #[test]
    fn jump_resolution() {
        // Resolves fine.
        assert!(model("*start\n[jump target=*start]\n").diagnostics().is_empty());
        // Unknown label.
        let m = model("[jump target=*nowhere]\n");
        assert!(m.diagnostics().iter().any(|d| matches!(d.code, DiagCode::SemUnknownLabel)));
        // Cross-file jump: not our business.
        assert!(model("[jump storage=other.ks target=*foo]\n").diagnostics().is_empty());
        // @call is covered; plain value (no `*`) is not a label target.
        let m = model("@call target=*missing\n");
        assert_eq!(m.diagnostics().len(), 1);
        assert!(model("[jump target=start]\n").diagnostics().is_empty());
    }

    #[test]
    fn item_at_prefers_innermost() {
        let src = "こんにちは[l]世界\n";
        let m = model(src);
        let offset = TextSize::new(src.find("[l]").unwrap() as u32 + 1);
        match m.item_at(offset) {
            Some(Item::Tag(t)) => assert_eq!(t.name, "l"),
            other => panic!("expected inline tag, got {other:?}"),
        }
    }

    #[test]
    fn options_flow_through() {
        let m = model("*a|b|c\n");
        assert_eq!(m.label("a").unwrap().value.as_deref(), Some("b"));
        let loose = lower(
            &parse("*a|b|c\n"),
            &InterpretOptions { label_value_first_segment_only: false, ..Default::default() },
        );
        assert_eq!(loose.label("a").unwrap().value.as_deref(), Some("b|c"));
    }

    #[test]
    fn macros_collected() {
        let m = model("[macro name=greet]\n[endmacro]\n");
        assert_eq!(m.macros().len(), 1);
        assert_eq!(m.macros()[0].name, "greet");
    }

    #[test]
    fn chara_and_script_payloads() {
        let m = model(SRC);
        let Some(Item::Chara(c)) = m.items().iter().find(|i| matches!(i, Item::Chara(_))) else {
            panic!("chara item expected");
        };
        assert_eq!(c.name.as_deref(), Some("akane"));
        assert_eq!(c.face.as_deref(), Some("happy"));
        let Some(Item::Script(s)) = m.items().iter().find(|i| matches!(i, Item::Script(_)))
        else {
            panic!("script item expected");
        };
        assert_eq!(s.code, "var a = 1;");
    }
}
