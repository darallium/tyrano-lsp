//! Lowering: one walk over the typed AST producing a [`SemanticIndex`].
//!
//! Pure and salsa-free by design so the whole semantic layer is testable
//! without a database. The walk mirrors `tyrano-analysis::lower` (which
//! this crate supersedes): top-level lines in document order, inline tags
//! inside text lines, `Line::Error` skipped wholesale, block interiors
//! (`[iscript]`/`[html]`) not descended into.

use tyrano_syntax::ast::{AstNode as _, InterpretOptions, Line, Scenario, Tag, TextSegment};
use tyrano_syntax::text::TextRange;

use crate::ast_id::{AstIdMap, ErasedAstId};
use crate::errors::{SemanticError, SemanticErrorKind};
use crate::index::{EmbeddedLang, EmbeddedRegion, PlaceId, PlaceOccurrence, SemanticIndex};
use crate::place::{self, AccessKind, Place};
use crate::scope::{Scope, ScopeId, ScopeKind};
use crate::symbol::{Definition, DefinitionId, Symbol, SymbolId, SymbolKind};
use crate::use_def::{Reference, RefKind, Resolution};

/// Tags that navigate to `target=*label` when no (non-empty) `storage=`
/// parameter redirects them to another file. Besides the classic four,
/// `glink` (graphical link button) and `clickable` (clickable area) take
/// the same target/storage pair in TyranoScript.
pub const JUMP_TAGS: &[&str] = &["jump", "call", "link", "button", "glink", "clickable"];

/// Tags whose `name=` parameter references a character definition.
pub const CHARA_REF_TAGS: &[&str] = &["chara_show", "chara_mod", "chara_hide", "chara_delete"];

/// Builds the [`SemanticIndex`] for one parsed scenario.
pub fn build_index(
    scenario: &Scenario,
    ast_ids: &AstIdMap,
    opts: &InterpretOptions,
) -> SemanticIndex {
    let mut builder = Builder {
        index: SemanticIndex::default(),
        ast_ids,
        opts,
        open_macro: None,
        tag_sites: Vec::new(),
    };

    let file_range = scenario.syntax().text_range();
    builder.index.scopes.push(Scope { kind: ScopeKind::Root, parent: None, range: file_range });

    for line in scenario.lines() {
        builder.line(&line);
    }
    builder.finish(file_range)
}

/// Everything remembered about a non-`macro`/`endmacro` tag during the
/// walk, resolved once all definitions are known.
struct TagSite {
    node: ErasedAstId,
    name: String,
    /// Range of the tag-name node (fallback: the whole tag).
    name_range: TextRange,
    tag_range: TextRange,
    /// Cooked `target=` value and its value-node range.
    target: Option<(String, TextRange)>,
    /// Cooked `storage=` value.
    storage: Option<String>,
    /// Cooked `name=` value and its value-node range (for chara refs).
    name_param: Option<(String, TextRange)>,
}

/// An open `[macro]` body awaiting its `[endmacro]`.
struct OpenMacro {
    scope: ScopeId,
    def: DefinitionId,
}

struct Builder<'a> {
    index: SemanticIndex,
    ast_ids: &'a AstIdMap,
    opts: &'a InterpretOptions,
    open_macro: Option<OpenMacro>,
    tag_sites: Vec<TagSite>,
}

impl Builder<'_> {
    fn current_scope(&self) -> ScopeId {
        self.open_macro.as_ref().map_or(ScopeId::ROOT, |m| m.scope)
    }

    fn item_id(&self, node: &tyrano_syntax::red::SyntaxNode) -> ErasedAstId {
        self.ast_ids
            .erased_id(node)
            .expect("walked node comes from the tree the AstIdMap was built for")
    }

    /// Records the item's scope attribution and returns its id.
    fn enter_item(&mut self, node: &tyrano_syntax::red::SyntaxNode) -> ErasedAstId {
        let id = self.item_id(node);
        self.index.scope_by_item.insert(id, self.current_scope());
        id
    }

    fn error(&mut self, kind: SemanticErrorKind, range: TextRange) {
        self.index.errors.push(SemanticError::new(kind, range));
    }

    /// Adds a definition for `name` in the given namespace, appending to an
    /// existing symbol (first-definition-wins) or creating a new one.
    /// Returns the definition id and whether the name was already defined.
    fn define(
        &mut self,
        kind: SymbolKind,
        name: &str,
        node: ErasedAstId,
        name_range: TextRange,
        full_range: TextRange,
        scope: ScopeId,
    ) -> (DefinitionId, Option<TextRange>) {
        let by_name = match kind {
            SymbolKind::Label => &mut self.index.labels_by_name,
            SymbolKind::Macro => &mut self.index.macros_by_name,
            SymbolKind::Character => &mut self.index.charas_by_name,
        };
        let def_id = DefinitionId(self.index.definitions.len() as u32);
        let (symbol_id, first_range) = match by_name.get(name) {
            Some(&sym) => {
                let first_def = self.index.symbols[sym.index()].defs[0];
                let first_range = self.index.definitions[first_def.index()].name_range;
                self.index.symbols[sym.index()].defs.push(def_id);
                (sym, Some(first_range))
            }
            None => {
                let sym = SymbolId(self.index.symbols.len() as u32);
                by_name.insert(name.to_string(), sym);
                self.index.symbols.push(Symbol {
                    kind,
                    name: name.to_string(),
                    defs: vec![def_id],
                });
                (sym, None)
            }
        };
        self.index.definitions.push(Definition {
            symbol: symbol_id,
            node,
            name_range,
            full_range,
            scope,
        });
        (def_id, first_range)
    }

    fn line(&mut self, line: &Line) {
        match line {
            Line::Label(l) => {
                let node = self.enter_item(l.syntax());
                let full_range = l.syntax().text_range();
                let name = l.name().unwrap_or_default();
                let name_range = l.name_token().map_or(full_range, |t| t.text_range());
                let scope = self.current_scope();
                let (_, first) =
                    self.define(SymbolKind::Label, &name, node, name_range, full_range, scope);
                if let Some(first) = first {
                    self.error(SemanticErrorKind::DuplicateLabel { name, first }, name_range);
                }
            }
            Line::Chara(c) => {
                let node = self.enter_item(c.syntax());
                let full_range = c.syntax().text_range();
                let Some(name) = c.name().filter(|n| !n.is_empty()) else { return };
                let name_range = c
                    .syntax()
                    .children()
                    .find(|n| n.kind() == tyrano_syntax::SyntaxKind::CHARA_NAME)
                    .map_or(full_range, |n| n.text_range());
                let scope = self.current_scope();
                // Duplicate characters are legal and common; no error.
                self.define(SymbolKind::Character, &name, node, name_range, full_range, scope);
            }
            Line::Text(t) => {
                self.enter_item(t.syntax());
                for seg in t.segments() {
                    if let TextSegment::Tag(tag) = seg {
                        self.tag(&tag);
                    }
                }
            }
            Line::AtTag(t) => self.tag(t),
            Line::IScript(s) => {
                // Deliberate non-goal: the block interior is full JS and is
                // not analyzed; where it is, however, is a file-local fact
                // an IDE layer needs (highlighting, embedded tooling).
                let node = self.enter_item(s.syntax());
                self.embedded_region(s.syntax(), node, EmbeddedLang::IScript);
            }
            Line::Html(h) => {
                let node = self.enter_item(h.syntax());
                self.embedded_region(h.syntax(), node, EmbeddedLang::Html);
            }
            // Comment lines are not item-like (no AstId) and carry no
            // semantics; error lines are covered by parse diagnostics.
            Line::Comment(_) | Line::BlockComment(_) => {}
            Line::Error(_) => {}
        }
    }

    /// Records the embedded-code region of an `[iscript]`/`[html]` block:
    /// the cover of its code tokens, or an empty range anchored right after
    /// the opening tag when the block has no code.
    fn embedded_region(
        &mut self,
        block: &tyrano_syntax::red::SyntaxNode,
        node: ErasedAstId,
        lang: EmbeddedLang,
    ) {
        let code_kind = match lang {
            EmbeddedLang::IScript => tyrano_syntax::SyntaxKind::SCRIPT_TEXT,
            EmbeddedLang::Html => tyrano_syntax::SyntaxKind::HTML_TEXT,
        };
        let code_range = block
            .descendants_with_tokens()
            .filter_map(|el| el.into_token())
            .filter(|t| t.kind() == code_kind)
            .map(|t| t.text_range())
            .reduce(TextRange::cover)
            .unwrap_or_else(|| {
                let anchor = block
                    .children()
                    .find(|n| n.kind() == tyrano_syntax::SyntaxKind::INLINE_TAG)
                    .map_or(block.text_range().start(), |open| open.text_range().end());
                TextRange::new(anchor, anchor)
            });
        self.index.embedded_regions.push(EmbeddedRegion { lang, block: node, code_range });
    }

    fn tag(&mut self, tag: &impl Tag) {
        let node = self.enter_item(tag.syntax());
        let tag_range = tag.syntax().text_range();
        let name = tag.name();
        let name_range = tag.tag_name().map_or(tag_range, |n| n.syntax().text_range());

        // The engine fills a JS object with params left to right, so a
        // repeated name silently drops the earlier value — warn on every
        // repeat. The macro pass-through `*` is legitimately repeatable.
        let mut seen: Vec<String> = Vec::new();
        for p in tag.params() {
            let pname = p.name();
            if pname.is_empty() || pname == "*" {
                continue;
            }
            if seen.contains(&pname) {
                self.error(
                    SemanticErrorKind::DuplicateParam { name: pname },
                    p.syntax().text_range(),
                );
            } else {
                seen.push(pname);
            }
        }

        match name.as_str() {
            "macro" => self.macro_def(tag, node, tag_range),
            "endmacro" => self.endmacro(tag_range),
            _ => {
                let mut target = None;
                let mut storage = None;
                let mut name_param = None;
                for p in tag.params() {
                    let value = || {
                        let cooked = p.cooked_value(self.opts)?;
                        let range =
                            p.value_node().map_or_else(|| p.syntax().text_range(), |v| v.syntax().text_range());
                        Some((cooked, range))
                    };
                    match p.name().as_str() {
                        "target" if target.is_none() => target = value(),
                        "storage" if storage.is_none() => storage = value().map(|(v, _)| v),
                        "name" if name_param.is_none() => name_param = value(),
                        _ => {}
                    }
                }

                // `[chara_new name=…]` is a definition site, not a reference.
                if name == "chara_new" {
                    if let Some((chara, range)) = &name_param {
                        if !chara.is_empty() {
                            let scope = self.current_scope();
                            self.define(
                                SymbolKind::Character,
                                chara,
                                node,
                                *range,
                                tag_range,
                                scope,
                            );
                        }
                    }
                }

                self.tag_sites.push(TagSite {
                    node,
                    name,
                    name_range,
                    tag_range,
                    target,
                    storage,
                    name_param,
                });
            }
        }

        self.places(tag, node);
    }

    fn macro_def(&mut self, tag: &impl Tag, node: ErasedAstId, tag_range: TextRange) {
        if let Some(open) = &self.open_macro {
            let outer =
                self.index.symbols[self.index.definitions[open.def.index()].symbol.index()]
                    .name
                    .clone();
            self.error(SemanticErrorKind::NestedMacro { outer }, tag_range);
            return;
        }
        let name_param = tag.param("name");
        let name = name_param.as_ref().and_then(|p| p.cooked_value(self.opts));
        let Some(name) = name.filter(|n| !n.is_empty()) else {
            self.error(SemanticErrorKind::MacroMissingName, tag_range);
            return;
        };
        let name_range = name_param
            .as_ref()
            .and_then(|p| p.value_node())
            .map_or(tag_range, |v| v.syntax().text_range());

        // The [macro] tag itself sits in the root scope; the body scope
        // starts right after it (its end is patched at [endmacro]/EOF).
        let (def, first) =
            self.define(SymbolKind::Macro, &name, node, name_range, tag_range, ScopeId::ROOT);
        if let Some(first) = first {
            self.error(SemanticErrorKind::DuplicateMacro { name, first }, name_range);
        }
        let scope = ScopeId(self.index.scopes.len() as u32);
        self.index.scopes.push(Scope {
            kind: ScopeKind::MacroBody { def },
            parent: Some(ScopeId::ROOT),
            range: TextRange::new(tag_range.end(), tag_range.end()),
        });
        self.open_macro = Some(OpenMacro { scope, def });
    }

    fn endmacro(&mut self, tag_range: TextRange) {
        match self.open_macro.take() {
            Some(open) => {
                let scope = &mut self.index.scopes[open.scope.index()];
                scope.range = TextRange::new(scope.range.start(), tag_range.end());
            }
            None => self.error(SemanticErrorKind::StrayEndMacro, tag_range),
        }
    }

    /// Collects variable places from the tag's expression-bearing params:
    /// `exp=` and `cond=` are parsed as expressions; any other param value
    /// contributes only when it is an `&entity` reference.
    fn places(&mut self, tag: &impl Tag, node: ErasedAstId) {
        let mut found: Vec<(Place, TextRange, AccessKind)> = Vec::new();
        for p in tag.params() {
            let Some(value) = p.value_node() else { continue };
            match p.name().as_str() {
                "exp" | "cond" => {
                    let expr = value.expr();
                    place::collect_places(&expr, expr.anchor(), &mut found);
                }
                _ => {
                    if let Some(expr) = value.entity_expr() {
                        place::collect_places(&expr, expr.anchor(), &mut found);
                    }
                }
            }
        }
        for (place, range, access) in found {
            let place_id = match self.index.place_lookup.get(&place) {
                Some(&id) => id,
                None => {
                    let id = PlaceId(self.index.places.len() as u32);
                    self.index.place_lookup.insert(place.clone(), id);
                    self.index.places.push(place);
                    id
                }
            };
            self.index.place_occurrences.push(PlaceOccurrence {
                place: place_id,
                node,
                range,
                access,
            });
        }
    }

    /// Closes an unterminated macro, resolves all collected references,
    /// and sorts errors.
    fn finish(mut self, file_range: TextRange) -> SemanticIndex {
        if let Some(open) = self.open_macro.take() {
            let def = &self.index.definitions[open.def.index()];
            let name = self.index.symbols[def.symbol.index()].name.clone();
            let name_range = def.name_range;
            let scope = &mut self.index.scopes[open.scope.index()];
            scope.range = TextRange::new(scope.range.start(), file_range.end());
            self.error(SemanticErrorKind::UnclosedMacro { name }, name_range);
        }

        let def_count = self.index.definitions.len();
        let sites = std::mem::take(&mut self.tag_sites);
        for site in &sites {
            self.resolve_site(site, def_count);
        }

        self.index.errors.sort_by_key(|e| (e.range.start(), e.range.end()));
        self.index
    }

    fn resolve_site(&mut self, site: &TagSite, def_count: usize) {
        if JUMP_TAGS.contains(&site.name.as_str()) {
            self.resolve_jump(site, def_count);
        } else if let Some(&sym) = self.index.macros_by_name.get(&site.name) {
            // A tag matching a file-local macro definition is a macro call.
            // Unmatched tag names are never diagnosed: they may be builtins
            // or macros defined in another file.
            let def = self.index.symbols[sym.index()].defs[0];
            self.index.use_def.push(
                Reference {
                    kind: RefKind::MacroCall,
                    node: site.node,
                    range: site.name_range,
                    name: site.name.clone(),
                    resolution: Resolution::Def(def),
                },
                def_count,
            );
        }

        if CHARA_REF_TAGS.contains(&site.name.as_str()) {
            if let Some((name, range)) = &site.name_param {
                if !name.is_empty() {
                    // Unknown characters are recorded, not diagnosed: they
                    // are routinely defined in another file.
                    let resolution = match self.index.charas_by_name.get(name) {
                        Some(&sym) => Resolution::Def(self.index.symbols[sym.index()].defs[0]),
                        None => Resolution::Unknown,
                    };
                    self.index.use_def.push(
                        Reference {
                            kind: RefKind::CharacterRef,
                            node: site.node,
                            range: *range,
                            name: name.clone(),
                            resolution,
                        },
                        def_count,
                    );
                }
            }
        }
    }

    fn resolve_jump(&mut self, site: &TagSite, def_count: usize) {
        let label = site
            .target
            .as_ref()
            .and_then(|(v, _)| v.strip_prefix('*'))
            .map(str::to_string);
        let range = site.target.as_ref().map_or(site.tag_range, |(_, r)| *r);

        if let Some(storage) = site.storage.as_ref().filter(|s| !s.is_empty()) {
            // Cross-file jump: out of file-local resolution, recorded for
            // the future multi-file layer (tyrano-analysis dropped these).
            self.index.use_def.push(
                Reference {
                    kind: RefKind::JumpTarget,
                    node: site.node,
                    range,
                    name: label.unwrap_or_default(),
                    resolution: Resolution::External {
                        storage: storage.clone(),
                        target: site.target.as_ref().map(|(v, _)| v.clone()),
                    },
                },
                def_count,
            );
            return;
        }

        // A target without a leading `*` is not a label reference.
        let Some(label) = label else { return };
        let resolution = match self.index.labels_by_name.get(&label) {
            Some(&sym) => Resolution::Def(self.index.symbols[sym.index()].defs[0]),
            None => {
                self.error(SemanticErrorKind::UnknownLabelTarget { name: label.clone() }, range);
                Resolution::Unknown
            }
        };
        self.index.use_def.push(
            Reference {
                kind: RefKind::JumpTarget,
                node: site.node,
                range,
                name: label,
                resolution,
            },
            def_count,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place::{PathSeg, PlaceRoot};
    use tyrano_syntax::parse;

    fn index(src: &str) -> SemanticIndex {
        let parsed = parse(src);
        let root = parsed.syntax();
        let ast_ids = AstIdMap::from_root(&root);
        build_index(&parsed.ast(), &ast_ids, &InterpretOptions::default())
    }

    fn error_codes(index: &SemanticIndex) -> Vec<&'static str> {
        index.errors().iter().map(|e| e.code()).collect()
    }

    // ---- labels (step 4) ----

    #[test]
    fn labels_are_indexed_first_definition_wins() {
        let idx = index("*a|one\ntext\n*a|two\n*b\n");
        assert_eq!(idx.definitions().len(), 3);
        assert_eq!(idx.symbols().len(), 2);
        let a = idx.symbol(idx.label("a").unwrap());
        assert_eq!(a.defs.len(), 2);
        // defs[0] is the winning (first) definition.
        let first = idx.definition(a.defs[0]);
        assert!(first.name_range.start() < idx.definition(a.defs[1]).name_range.start());
        assert!(idx.label("b").is_some());
        assert!(idx.label("missing").is_none());
    }

    #[test]
    fn duplicate_label_error_points_at_second_site() {
        let idx = index("*a|one\n*a|two\n");
        assert_eq!(error_codes(&idx), ["sem-duplicate-label"]);
        let err = &idx.errors()[0];
        let SemanticErrorKind::DuplicateLabel { name, first } = &err.kind else {
            panic!("wrong kind: {err:?}");
        };
        assert_eq!(name, "a");
        assert!(first.start() < err.range.start(), "primary is the second site");
    }

    #[test]
    fn label_name_range_is_name_token() {
        let idx = index("*start|value\n");
        let def = idx.definition(idx.symbol(idx.label("start").unwrap()).defs[0]);
        // "*start|value\n": name token "start" spans 1..6.
        assert_eq!(u32::from(def.name_range.start()), 1);
        assert_eq!(u32::from(def.name_range.end()), 6);
        assert!(def.full_range.contains_range(def.name_range));
    }

    // ---- jumps / use-def (step 5) ----

    #[test]
    fn jump_to_known_label_resolves() {
        for src in ["*start\n[jump target=*start]\n", "*start\n@call target=*start\n"] {
            let idx = index(src);
            assert!(idx.errors().is_empty(), "{:?}", idx.errors());
            let uses = idx.use_def().uses();
            assert_eq!(uses.len(), 1);
            assert_eq!(uses[0].kind, RefKind::JumpTarget);
            assert_eq!(uses[0].name, "start");
            let Resolution::Def(def) = &uses[0].resolution else {
                panic!("unresolved: {:?}", uses[0].resolution);
            };
            assert_eq!(idx.symbol(idx.definition(*def).symbol).name, "start");
        }
    }

    #[test]
    fn unknown_target_is_error() {
        let idx = index("[jump target=*nowhere]\n");
        assert_eq!(error_codes(&idx), ["sem-unknown-label-target"]);
        assert_eq!(idx.use_def().uses().len(), 1);
        assert_eq!(idx.use_def().uses()[0].resolution, Resolution::Unknown);
        // The error anchors at the target value, not the whole tag.
        let err_range = idx.errors()[0].range;
        assert_eq!(err_range, idx.use_def().uses()[0].range);
        let src = "[jump target=*nowhere]\n";
        assert_eq!(
            &src[u32::from(err_range.start()) as usize..u32::from(err_range.end()) as usize],
            "*nowhere"
        );
    }

    #[test]
    fn storage_jump_is_external_not_error() {
        let idx = index("[jump storage=other.ks target=*foo]\n");
        assert!(idx.errors().is_empty(), "{:?}", idx.errors());
        let uses = idx.use_def().uses();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].name, "foo");
        assert_eq!(
            uses[0].resolution,
            Resolution::External {
                storage: "other.ks".to_string(),
                target: Some("*foo".to_string())
            }
        );
        // storage without target is also recorded.
        let idx = index("[jump storage=other.ks]\n");
        assert_eq!(idx.use_def().uses().len(), 1);
        assert_eq!(idx.use_def().uses()[0].name, "");
    }

    #[test]
    fn non_star_target_is_not_a_reference() {
        let idx = index("[jump target=start]\n");
        assert!(idx.errors().is_empty());
        assert!(idx.use_def().uses().is_empty());
    }

    #[test]
    fn def_to_uses_back_map() {
        let idx = index("*a\n[jump target=*a]\n[link target=*a]\n");
        let def = idx.symbol(idx.label("a").unwrap()).defs[0];
        let uses = idx.use_def().uses_of(def);
        assert_eq!(uses.len(), 2);
        // Document order.
        assert!(
            idx.use_def().use_(uses[0]).range.start() < idx.use_def().use_(uses[1]).range.start()
        );
    }

    // ---- macros (step 6) ----

    #[test]
    fn macro_def_indexed_with_body_scope() {
        let src = "[macro name=greet]\n[image storage=a.png]\n[endmacro]\n*after\n";
        let idx = index(src);
        assert!(idx.errors().is_empty(), "{:?}", idx.errors());
        let sym = idx.symbol(idx.macro_("greet").unwrap());
        assert_eq!(sym.kind, SymbolKind::Macro);

        assert_eq!(idx.scopes().len(), 2);
        let body = idx.scope(crate::scope::ScopeId(1));
        assert_eq!(body.parent, Some(ScopeId::ROOT));
        let ScopeKind::MacroBody { def } = &body.kind else { panic!("not a macro body") };
        assert_eq!(*def, sym.defs[0]);
        // Body runs from after the [macro] tag to the end of [endmacro].
        let macro_tag_end = src.find("]\n").unwrap() + 1;
        let endmacro_end = src.find("[endmacro]").unwrap() + "[endmacro]".len();
        assert_eq!(u32::from(body.range.start()) as usize, macro_tag_end);
        assert_eq!(u32::from(body.range.end()) as usize, endmacro_end);

        // The inner tag is attributed to the body scope; the label after
        // [endmacro] is back at root.
        let parsed = parse(src);
        let root = parsed.syntax();
        let ast_ids = AstIdMap::from_root(&root);
        let image_tag = root
            .descendants()
            .find(|n| {
                n.kind() == tyrano_syntax::SyntaxKind::INLINE_TAG
                    && n.to_string().starts_with("[image")
            })
            .expect("fixture has an [image] tag");
        let image_id = ast_ids.erased_id(&image_tag).expect("tag has an id");
        assert_eq!(idx.scope_of(image_id), crate::scope::ScopeId(1));
        let after = idx.definition(idx.symbol(idx.label("after").unwrap()).defs[0]);
        assert_eq!(after.scope, ScopeId::ROOT);
    }

    #[test]
    fn unclosed_macro_error_at_eof() {
        let src = "[macro name=greet]\n[image storage=a.png]\n";
        let idx = index(src);
        assert_eq!(error_codes(&idx), ["sem-unclosed-macro"]);
        // Scope extends to EOF.
        let body = idx.scope(crate::scope::ScopeId(1));
        assert_eq!(u32::from(body.range.end()) as usize, src.len());
    }

    #[test]
    fn stray_endmacro_error() {
        let idx = index("[endmacro]\n");
        assert_eq!(error_codes(&idx), ["sem-stray-endmacro"]);
        assert_eq!(idx.scopes().len(), 1);
    }

    #[test]
    fn macro_missing_name_error() {
        for src in ["[macro]\n", "[macro name=]\n"] {
            let idx = index(src);
            assert_eq!(error_codes(&idx), ["sem-macro-missing-name"], "src: {src}");
            assert!(idx.symbols().is_empty());
        }
    }

    #[test]
    fn nested_macro_error() {
        let idx = index("[macro name=outer]\n[macro name=inner]\n[endmacro]\n");
        assert_eq!(error_codes(&idx), ["sem-nested-macro"]);
        let SemanticErrorKind::NestedMacro { outer } = &idx.errors()[0].kind else {
            panic!("wrong kind");
        };
        assert_eq!(outer, "outer");
        // The inner [macro] is ignored: no def, and the [endmacro] closes
        // the outer body.
        assert!(idx.macro_("inner").is_none());
        assert_eq!(idx.scopes().len(), 2);
    }

    #[test]
    fn duplicate_macro_first_wins() {
        let idx = index("[macro name=greet]\n[endmacro]\n[macro name=greet]\n[endmacro]\n");
        assert_eq!(error_codes(&idx), ["sem-duplicate-macro"]);
        let sym = idx.symbol(idx.macro_("greet").unwrap());
        assert_eq!(sym.defs.len(), 2);
        assert!(
            idx.definition(sym.defs[0]).name_range.start()
                < idx.definition(sym.defs[1]).name_range.start()
        );
    }

    #[test]
    fn tag_matching_local_macro_is_macro_call_use() {
        let idx = index("[macro name=greet]\n[endmacro]\n[greet]\n@greet\n");
        assert!(idx.errors().is_empty(), "{:?}", idx.errors());
        let calls: Vec<_> =
            idx.use_def().uses().iter().filter(|u| u.kind == RefKind::MacroCall).collect();
        assert_eq!(calls.len(), 2);
        let def = idx.symbol(idx.macro_("greet").unwrap()).defs[0];
        for call in calls {
            assert_eq!(call.resolution, Resolution::Def(def));
            assert_eq!(call.name, "greet");
        }
    }

    #[test]
    fn unknown_tag_is_not_an_error() {
        let idx = index("[not_a_builtin_or_macro foo=1]\n");
        assert!(idx.errors().is_empty());
        assert!(idx.use_def().uses().is_empty());
    }

    // ---- characters (step 7) ----

    #[test]
    fn chara_line_defines_character() {
        let idx = index("#akane:happy\n");
        let sym = idx.symbol(idx.character("akane").unwrap());
        assert_eq!(sym.kind, SymbolKind::Character);
        assert_eq!(sym.defs.len(), 1);
    }

    #[test]
    fn chara_new_tag_defines_character() {
        let idx = index("[chara_new name=akane storage=akane.png]\n");
        assert!(idx.character("akane").is_some());
        // A definition site, not a reference.
        assert!(idx.use_def().uses().is_empty());
    }

    #[test]
    fn chara_show_ref_resolves() {
        let idx = index("[chara_new name=akane]\n[chara_show name=akane]\n");
        let uses = idx.use_def().uses();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].kind, RefKind::CharacterRef);
        let def = idx.symbol(idx.character("akane").unwrap()).defs[0];
        assert_eq!(uses[0].resolution, Resolution::Def(def));
    }

    #[test]
    fn unknown_chara_ref_is_unknown_not_error() {
        let idx = index("[chara_show name=ghost]\n");
        assert!(idx.errors().is_empty());
        assert_eq!(idx.use_def().uses()[0].resolution, Resolution::Unknown);
    }

    #[test]
    fn first_chara_definition_wins() {
        let idx = index("#akane:a\n#akane:b\n");
        assert!(idx.errors().is_empty(), "duplicate charas are not an error");
        let sym = idx.symbol(idx.character("akane").unwrap());
        assert_eq!(sym.defs.len(), 2);
    }

    // ---- places (step 8) ----

    #[test]
    fn eval_exp_yields_write_and_read() {
        let src = "[eval exp=\"f.hp = f.hp - 1\"]\n";
        let idx = index(src);
        let occs = idx.place_occurrences();
        assert_eq!(occs.len(), 2);
        let place = idx.place(occs[0].place);
        assert_eq!(place.root, PlaceRoot::GameVar);
        assert_eq!(place.path, vec![PathSeg::Field("hp".to_string())]);
        assert_eq!(occs[0].access, AccessKind::Write);
        assert_eq!(occs[1].access, AccessKind::Read);
        // Same place interned once.
        assert_eq!(occs[0].place, occs[1].place);
        assert_eq!(idx.places().len(), 1);
        // Ranges are file-absolute.
        let text = &src[u32::from(occs[0].range.start()) as usize
            ..u32::from(occs[0].range.end()) as usize];
        assert_eq!(text, "f.hp");
    }

    #[test]
    fn cond_and_entity_values_yield_reads() {
        let idx = index("[jump target=*a cond=\"sf.done\"]\n*a\n[image storage=&f.bg]\n");
        let occs = idx.place_occurrences();
        assert_eq!(occs.len(), 2);
        assert!(occs.iter().all(|o| o.access == AccessKind::Read));
        let roots: Vec<PlaceRoot> = occs.iter().map(|o| idx.place(o.place).root).collect();
        assert_eq!(roots, [PlaceRoot::SystemVar, PlaceRoot::GameVar]);
    }

    #[test]
    fn iscript_bodies_are_skipped() {
        let idx = index("[iscript]\nf.hidden = 1;\n[endscript]\n");
        assert!(idx.place_occurrences().is_empty());
    }

    // ---- domain enhancements (ruff_db-inspired round) ----

    #[test]
    fn glink_and_clickable_are_jump_tags() {
        let idx = index("*menu\n[glink target=*menu text=Back]\n[clickable target=*menu]\n");
        assert!(idx.errors().is_empty(), "{:?}", idx.errors());
        let uses = idx.use_def().uses();
        assert_eq!(uses.len(), 2);
        let def = idx.symbol(idx.label("menu").unwrap()).defs[0];
        for u in uses {
            assert_eq!(u.kind, RefKind::JumpTarget);
            assert_eq!(u.resolution, Resolution::Def(def));
        }
        // storage= still routes them out of the file.
        let idx = index("[glink storage=other.ks target=*foo]\n");
        assert!(matches!(
            idx.use_def().uses()[0].resolution,
            Resolution::External { .. }
        ));
    }

    #[test]
    fn duplicate_param_is_warning() {
        let src = "[image storage=a.png storage=b.png]\n";
        let idx = index(src);
        assert_eq!(error_codes(&idx), ["sem-duplicate-param"]);
        let err = &idx.errors()[0];
        assert_eq!(err.severity(), tyrano_syntax::diagnostics::Severity::Warning);
        // Anchored at the SECOND occurrence.
        let text =
            &src[u32::from(err.range.start()) as usize..u32::from(err.range.end()) as usize];
        assert_eq!(text, "storage=b.png");
    }

    #[test]
    fn duplicate_param_ignores_macro_star_and_flags() {
        // `*` pass-through can repeat; distinct flag params are fine; a
        // repeated flag param does warn.
        assert!(index("[greet * *]\n").errors().is_empty());
        assert!(index("[button flag other]\n").errors().is_empty());
        assert_eq!(index("[button flag flag]\n").errors().len(), 1);
    }

    #[test]
    fn duplicate_param_checked_on_macro_tags_too() {
        let idx = index("[macro name=a name=b]\n[endmacro]\n");
        assert_eq!(error_codes(&idx), ["sem-duplicate-param"]);
    }

    #[test]
    fn embedded_regions_cover_block_code() {
        let src = "[iscript]\nvar a = 1;\nvar b = 2;\n[endscript]\n[html]\n<b>x</b>\n[endhtml]\n";
        let idx = index(src);
        let regions = idx.embedded_regions();
        assert_eq!(regions.len(), 2);

        assert_eq!(regions[0].lang, EmbeddedLang::IScript);
        let code = &src[u32::from(regions[0].code_range.start()) as usize
            ..u32::from(regions[0].code_range.end()) as usize];
        assert!(code.starts_with("var a = 1;"), "got {code:?}");
        assert!(code.ends_with("var b = 2;"), "got {code:?}");

        assert_eq!(regions[1].lang, EmbeddedLang::Html);
        let html = &src[u32::from(regions[1].code_range.start()) as usize
            ..u32::from(regions[1].code_range.end()) as usize];
        assert_eq!(html, "<b>x</b>");
    }

    #[test]
    fn empty_block_yields_empty_region_after_open_tag() {
        let src = "[iscript]\n[endscript]\n";
        let idx = index(src);
        let regions = idx.embedded_regions();
        assert_eq!(regions.len(), 1);
        assert!(regions[0].code_range.is_empty());
        assert_eq!(
            u32::from(regions[0].code_range.start()) as usize,
            src.find(']').unwrap() + 1,
            "empty region anchors right after [iscript]"
        );
    }

    #[test]
    fn mp_places_in_macro_body_attach_to_macro_scope() {
        let idx = index("[macro name=greet]\n[eval exp=\"f.count = mp.n\"]\n[endmacro]\n");
        let occs = idx.place_occurrences();
        assert_eq!(occs.len(), 2);
        let mp = occs.iter().find(|o| idx.place(o.place).root == PlaceRoot::MacroParams).unwrap();
        assert_eq!(idx.scope_of(mp.node), crate::scope::ScopeId(1));
    }
}

